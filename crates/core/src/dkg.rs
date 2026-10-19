//! Dealerless distributed key generation (Pedersen DKG over Feldman VSS).
//!
//! Splitting one key with Shamir would need a dealer who once held the whole
//! key. Instead every trustee is a dealer of their *own* random secret, and
//! the poll key is the sum of all of them, so nobody ever knows it:
//!
//! 1. **Register.** Each trustee generates a [`TrusteeIdentity`] and publishes
//!    its public key with a proof of possession. The list of registrations
//!    fixes the [`Ceremony`]: trustee indices (1-based) and threshold `t`.
//! 2. **Deal.** Each trustee `i` picks a random degree-`t − 1` polynomial
//!    `f_i`, publishes Feldman commitments `C_ik = a_ik·G` to its
//!    coefficients, and sends every trustee `j` the share `f_i(j)` encrypted
//!    to `j`'s identity key (Diffie–Hellman pad). Dealings are signed, so the
//!    server relaying them can neither read nor forge them.
//! 3. **Check.** Each trustee decrypts their shares and checks them against
//!    the commitments. A bad share yields a [`Complaint`] that reveals the
//!    Diffie–Hellman value with a proof, so *anyone* can re-open the share
//!    and see who's lying — no further round with the dealer is needed.
//! 4. **Finalize.** Dealers with a valid dealing and no upheld complaint are
//!    qualified. The poll key is `Σ C_i0` over them; trustee `j`'s key share
//!    is `Σ f_i(j)` and its public verification key follows from the
//!    commitments.
//!
//! Any `t` trustees can then decrypt (see [`crate::partial_decrypt`]); any
//! `t` colluding trustees could also decrypt individual ballots, so `t` is
//! the privacy/liveness trade-off.

use std::collections::{BTreeMap, HashSet};
use std::fmt;

use curve25519_dalek::{ristretto::RistrettoPoint, scalar::Scalar, traits::Identity};
use rand_core::CryptoRngCore;
use sha2::{Digest, Sha256};

use crate::elgamal::PublicKey;
use crate::proofs::{DleqProof, SchnorrProof};
use crate::threshold::{KeyShare, TallyKey};
use crate::transcript::Transcript;

/// A trustee's long-term secret for one ceremony. Keep it on the trustee's device.
pub struct TrusteeIdentity {
    secret: Scalar,
    public: RistrettoPoint,
}

/// A trustee's public key plus proof they hold the matching secret.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    key: RistrettoPoint,
    proof: SchnorrProof,
}

#[derive(Clone, Debug)]
pub struct Ceremony {
    threshold: usize,
    trustees: Vec<RistrettoPoint>,
    digest: [u8; 32],
}

/// One trustee's published contribution: commitments and encrypted shares.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Dealing {
    pub dealer: u32,
    commitments: Vec<RistrettoPoint>,
    ephemeral: RistrettoPoint,
    encrypted_shares: Vec<Scalar>,
    signature: SchnorrProof,
}

/// "Dealer sent me a bad share", with the evidence to check it publicly.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Complaint {
    pub dealer: u32,
    pub accuser: u32,
    shared_secret: RistrettoPoint,
    proof: DleqProof,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DkgError {
    InvalidThreshold,
    InvalidRegistration { index: usize },
    DuplicateIdentity,
    NotATrustee,
    NoQualifiedDealers,
    BadShare { dealer: u32 },
}

impl fmt::Display for DkgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DkgError::InvalidThreshold => {
                write!(f, "threshold must be between 1 and the trustee count")
            }
            DkgError::InvalidRegistration { index } => {
                write!(f, "registration {index} has an invalid proof of possession")
            }
            DkgError::DuplicateIdentity => write!(f, "a trustee key is registered twice"),
            DkgError::NotATrustee => write!(f, "this identity is not a trustee of the ceremony"),
            DkgError::NoQualifiedDealers => write!(f, "no dealer qualified"),
            DkgError::BadShare { dealer } => write!(
                f,
                "dealer {dealer} sent this trustee a bad share and no complaint was filed"
            ),
        }
    }
}

impl std::error::Error for DkgError {}

fn registration_context(ceremony_id: &str) -> Vec<u8> {
    let mut ctx = b"register".to_vec();
    ctx.extend_from_slice(&(ceremony_id.len() as u64).to_le_bytes());
    ctx.extend_from_slice(ceremony_id.as_bytes());
    ctx
}

impl TrusteeIdentity {
    pub fn generate(rng: &mut impl CryptoRngCore) -> Self {
        let secret = Scalar::random(rng);
        TrusteeIdentity {
            secret,
            public: RistrettoPoint::mul_base(&secret),
        }
    }

    /// The secret, for storing on the trustee's device.
    pub fn to_bytes(&self) -> [u8; 32] {
        self.secret.to_bytes()
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Option<Self> {
        let secret: Option<Scalar> = Scalar::from_canonical_bytes(bytes).into();
        secret.map(|secret| TrusteeIdentity {
            secret,
            public: RistrettoPoint::mul_base(&secret),
        })
    }

    pub fn public_bytes(&self) -> [u8; 32] {
        self.public.compress().to_bytes()
    }

    pub fn register(&self, ceremony_id: &str, rng: &mut impl CryptoRngCore) -> Registration {
        Registration {
            key: self.public,
            proof: SchnorrProof::prove(&self.secret, &registration_context(ceremony_id), rng),
        }
    }
}

impl Registration {
    pub fn key_bytes(&self) -> [u8; 32] {
        self.key.compress().to_bytes()
    }

    /// Checks the proof of possession, so a relay can reject bad registrations early.
    pub fn verify(&self, ceremony_id: &str) -> bool {
        self.proof
            .verify(&self.key, &registration_context(ceremony_id))
    }
}

/// `f(j)` for coefficients `[a_0, a_1, …]`.
fn eval_poly(coeffs: &[Scalar], j: u32) -> Scalar {
    let x = Scalar::from(j);
    coeffs.iter().rev().fold(Scalar::ZERO, |acc, a| acc * x + a)
}

/// `f(j)·G` from the Feldman commitments `[a_0·G, a_1·G, …]`.
fn eval_commitments(commitments: &[RistrettoPoint], j: u32) -> RistrettoPoint {
    let x = Scalar::from(j);
    commitments
        .iter()
        .rev()
        .fold(RistrettoPoint::identity(), |acc, c| acc * x + c)
}

impl Ceremony {
    /// Fixes the trustee set: index `j` is `registrations[j − 1]`.
    pub fn new(
        ceremony_id: &str,
        threshold: usize,
        registrations: &[Registration],
    ) -> Result<Self, DkgError> {
        let n = registrations.len();
        if threshold == 0 || threshold > n {
            return Err(DkgError::InvalidThreshold);
        }
        let ctx = registration_context(ceremony_id);
        let mut seen = HashSet::new();
        for (index, r) in registrations.iter().enumerate() {
            if !r.proof.verify(&r.key, &ctx) {
                return Err(DkgError::InvalidRegistration { index });
            }
            if !seen.insert(r.key.compress().to_bytes()) {
                return Err(DkgError::DuplicateIdentity);
            }
        }
        let mut h = Sha256::new();
        h.update(&ctx);
        h.update((threshold as u64).to_le_bytes());
        h.update((n as u64).to_le_bytes());
        for r in registrations {
            h.update(r.key.compress().as_bytes());
        }
        Ok(Ceremony {
            threshold,
            trustees: registrations.iter().map(|r| r.key).collect(),
            digest: h.finalize().into(),
        })
    }

    pub fn threshold(&self) -> usize {
        self.threshold
    }

    pub fn trustees(&self) -> usize {
        self.trustees.len()
    }

    pub fn index_of(&self, me: &TrusteeIdentity) -> Option<u32> {
        self.trustees
            .iter()
            .position(|k| *k == me.public)
            .map(|i| i as u32 + 1)
    }

    fn identity_key(&self, index: u32) -> Option<&RistrettoPoint> {
        (index as usize)
            .checked_sub(1)
            .and_then(|i| self.trustees.get(i))
    }

    fn context(&self, label: &[u8], dealer: u32, other: u32) -> Vec<u8> {
        let mut ctx = self.digest.to_vec();
        ctx.extend_from_slice(label);
        ctx.extend_from_slice(&dealer.to_le_bytes());
        ctx.extend_from_slice(&other.to_le_bytes());
        ctx
    }

    fn pad(
        &self,
        dealer: u32,
        recipient: u32,
        ephemeral: &RistrettoPoint,
        dh: &RistrettoPoint,
    ) -> Scalar {
        let mut t = Transcript::new(b"share-pad");
        t.append_bytes(b"ctx", &self.context(b"pad", dealer, recipient));
        t.append_point(b"ephemeral", ephemeral);
        t.append_point(b"dh", dh);
        t.challenge()
    }

    fn dealing_message(
        &self,
        dealer: u32,
        commitments: &[RistrettoPoint],
        ephemeral: &RistrettoPoint,
        encrypted_shares: &[Scalar],
    ) -> Vec<u8> {
        let mut msg = self.context(b"dealing", dealer, 0);
        for c in commitments {
            msg.extend_from_slice(c.compress().as_bytes());
        }
        msg.extend_from_slice(ephemeral.compress().as_bytes());
        for s in encrypted_shares {
            msg.extend_from_slice(s.as_bytes());
        }
        msg
    }

    fn sign_dealing(
        &self,
        me: &TrusteeIdentity,
        dealer: u32,
        commitments: Vec<RistrettoPoint>,
        ephemeral: RistrettoPoint,
        encrypted_shares: Vec<Scalar>,
        rng: &mut impl CryptoRngCore,
    ) -> Dealing {
        let msg = self.dealing_message(dealer, &commitments, &ephemeral, &encrypted_shares);
        Dealing {
            dealer,
            commitments,
            ephemeral,
            encrypted_shares,
            signature: SchnorrProof::prove(&me.secret, &msg, rng),
        }
    }

    /// Step 2: this trustee's dealing, to publish.
    pub fn deal(
        &self,
        me: &TrusteeIdentity,
        rng: &mut impl CryptoRngCore,
    ) -> Result<Dealing, DkgError> {
        let dealer = self.index_of(me).ok_or(DkgError::NotATrustee)?;
        let coeffs: Vec<Scalar> = (0..self.threshold).map(|_| Scalar::random(rng)).collect();
        let commitments = coeffs.iter().map(RistrettoPoint::mul_base).collect();
        let e = Scalar::random(rng);
        let ephemeral = RistrettoPoint::mul_base(&e);
        let encrypted_shares = (1..=self.trustees.len() as u32)
            .zip(&self.trustees)
            .map(|(j, key)| eval_poly(&coeffs, j) + self.pad(dealer, j, &ephemeral, &(e * key)))
            .collect();
        Ok(self.sign_dealing(me, dealer, commitments, ephemeral, encrypted_shares, rng))
    }

    /// Whether `d` is well-formed and signed by its dealer for this ceremony.
    pub fn verify_dealing(&self, d: &Dealing) -> bool {
        let Some(key) = self.identity_key(d.dealer) else {
            return false;
        };
        let msg = self.dealing_message(d.dealer, &d.commitments, &d.ephemeral, &d.encrypted_shares);
        d.commitments.len() == self.threshold
            && d.encrypted_shares.len() == self.trustees.len()
            && d.signature.verify(key, &msg)
    }

    /// Decrypts `recipient`'s share of `d` and checks it against the commitments.
    fn open_share(&self, d: &Dealing, recipient: u32, dh: &RistrettoPoint) -> Option<Scalar> {
        let encrypted = d
            .encrypted_shares
            .get((recipient as usize).checked_sub(1)?)?;
        let share = encrypted - self.pad(d.dealer, recipient, &d.ephemeral, dh);
        (RistrettoPoint::mul_base(&share) == eval_commitments(&d.commitments, recipient))
            .then_some(share)
    }

    fn complain(
        &self,
        me: &TrusteeIdentity,
        accuser: u32,
        d: &Dealing,
        rng: &mut impl CryptoRngCore,
    ) -> Complaint {
        let ctx = self.context(b"complaint", d.dealer, accuser);
        Complaint {
            dealer: d.dealer,
            accuser,
            shared_secret: me.secret * d.ephemeral,
            proof: DleqProof::prove(&me.secret, &d.ephemeral, &ctx, rng),
        }
    }

    /// Step 3: complaints against dealers who sent this trustee a bad share.
    pub fn check_dealings(
        &self,
        me: &TrusteeIdentity,
        dealings: &[Dealing],
        rng: &mut impl CryptoRngCore,
    ) -> Result<Vec<Complaint>, DkgError> {
        let j = self.index_of(me).ok_or(DkgError::NotATrustee)?;
        Ok(dealings
            .iter()
            .filter(|d| self.verify_dealing(d))
            .filter(|d| self.open_share(d, j, &(me.secret * d.ephemeral)).is_none())
            .map(|d| self.complain(me, j, d, rng))
            .collect())
    }

    /// Whether `c` proves its dealer cheated. A false or forged complaint is not upheld.
    fn upheld(&self, d: &Dealing, c: &Complaint) -> bool {
        let Some(accuser_key) = self.identity_key(c.accuser) else {
            return false;
        };
        let ctx = self.context(b"complaint", c.dealer, c.accuser);
        c.proof
            .verify(accuser_key, &d.ephemeral, &c.shared_secret, &ctx)
            && self.open_share(d, c.accuser, &c.shared_secret).is_none()
    }

    /// Qualified dealings, by dealer: valid, not equivocated, no upheld complaint.
    fn qualified<'a>(&self, dealings: &'a [Dealing], complaints: &[Complaint]) -> Vec<&'a Dealing> {
        let mut by_dealer: BTreeMap<u32, Option<&Dealing>> = BTreeMap::new();
        for d in dealings.iter().filter(|d| self.verify_dealing(d)) {
            by_dealer
                .entry(d.dealer)
                .and_modify(|seen| {
                    // Two different signed dealings from one dealer: equivocation.
                    if *seen != Some(d) {
                        *seen = None;
                    }
                })
                .or_insert(Some(d));
        }
        by_dealer
            .into_values()
            .flatten()
            .filter(|d| {
                !complaints
                    .iter()
                    .any(|c| c.dealer == d.dealer && self.upheld(d, c))
            })
            .collect()
    }

    /// Step 4 (public): the poll key everyone will encrypt to.
    pub fn finalize(
        &self,
        dealings: &[Dealing],
        complaints: &[Complaint],
    ) -> Result<TallyKey, DkgError> {
        let qualified = self.qualified(dealings, complaints);
        if qualified.is_empty() {
            return Err(DkgError::NoQualifiedDealers);
        }
        let public = qualified.iter().map(|d| d.commitments[0]).sum();
        let verification_keys = (1..=self.trustees.len() as u32)
            .map(|j| {
                qualified
                    .iter()
                    .map(|d| eval_commitments(&d.commitments, j))
                    .sum()
            })
            .collect();
        Ok(TallyKey::new(
            PublicKey(public),
            self.threshold,
            verification_keys,
        ))
    }

    /// Step 4 (private): this trustee's share of the poll key.
    pub fn key_share(
        &self,
        me: &TrusteeIdentity,
        dealings: &[Dealing],
        complaints: &[Complaint],
    ) -> Result<KeyShare, DkgError> {
        let j = self.index_of(me).ok_or(DkgError::NotATrustee)?;
        let qualified = self.qualified(dealings, complaints);
        if qualified.is_empty() {
            return Err(DkgError::NoQualifiedDealers);
        }
        let secret = qualified
            .iter()
            .map(|d| {
                self.open_share(d, j, &(me.secret * d.ephemeral))
                    .ok_or(DkgError::BadShare { dealer: d.dealer })
            })
            .sum::<Result<Scalar, _>>()?;
        Ok(KeyShare { index: j, secret })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_core::OsRng;

    fn setup(n: usize, t: usize) -> (Vec<TrusteeIdentity>, Ceremony) {
        let ids: Vec<_> = (0..n)
            .map(|_| TrusteeIdentity::generate(&mut OsRng))
            .collect();
        let regs: Vec<_> = ids.iter().map(|i| i.register("poll", &mut OsRng)).collect();
        (ids, Ceremony::new("poll", t, &regs).unwrap())
    }

    fn deal_all(ids: &[TrusteeIdentity], c: &Ceremony) -> Vec<Dealing> {
        ids.iter().map(|i| c.deal(i, &mut OsRng).unwrap()).collect()
    }

    /// A dealing whose share for `victim` is off by one, but properly signed.
    fn bad_dealing(c: &Ceremony, dealer: &TrusteeIdentity, victim: u32) -> Dealing {
        let mut d = c.deal(dealer, &mut OsRng).unwrap();
        d.encrypted_shares[victim as usize - 1] += Scalar::ONE;
        c.sign_dealing(
            dealer,
            d.dealer,
            d.commitments,
            d.ephemeral,
            d.encrypted_shares,
            &mut OsRng,
        )
    }

    #[test]
    fn key_shares_match_verification_keys() {
        let (ids, c) = setup(4, 3);
        let dealings = deal_all(&ids, &c);
        for i in &ids {
            assert!(c
                .check_dealings(i, &dealings, &mut OsRng)
                .unwrap()
                .is_empty());
        }
        let key = c.finalize(&dealings, &[]).unwrap();
        assert_eq!((key.threshold(), key.trustees()), (3, 4));
        for i in &ids {
            let share = c.key_share(i, &dealings, &[]).unwrap();
            let x = RistrettoPoint::mul_base(&share.secret);
            assert_eq!(x, eval_commitments_sum(&c, &dealings, share.index));
        }
    }

    fn eval_commitments_sum(c: &Ceremony, dealings: &[Dealing], j: u32) -> RistrettoPoint {
        c.qualified(dealings, &[])
            .iter()
            .map(|d| eval_commitments(&d.commitments, j))
            .sum()
    }

    #[test]
    fn bad_share_is_caught_and_dealer_disqualified() {
        let (ids, c) = setup(3, 2);
        let mut dealings = deal_all(&ids, &c);
        dealings[0] = bad_dealing(&c, &ids[0], 2);

        let complaints = c.check_dealings(&ids[1], &dealings, &mut OsRng).unwrap();
        assert_eq!(complaints.len(), 1);
        assert_eq!((complaints[0].dealer, complaints[0].accuser), (1, 2));

        // Without the complaint, the victim can't build a share; with it, the dealer is out.
        assert_eq!(
            c.key_share(&ids[1], &dealings, &[]).err(),
            Some(DkgError::BadShare { dealer: 1 })
        );
        assert_eq!(c.qualified(&dealings, &complaints).len(), 2);
        let key = c.finalize(&dealings, &complaints).unwrap();
        let expected: RistrettoPoint = dealings[1..].iter().map(|d| d.commitments[0]).sum();
        assert_eq!(key.public_key.0, expected);
        assert!(c.key_share(&ids[1], &dealings, &complaints).is_ok());
    }

    #[test]
    fn false_complaint_is_not_upheld() {
        let (ids, c) = setup(3, 2);
        let dealings = deal_all(&ids, &c);
        let lie = c.complain(&ids[1], 2, &dealings[0], &mut OsRng);
        assert_eq!(c.qualified(&dealings, &[lie]).len(), 3);
    }

    #[test]
    fn complaint_without_the_accusers_key_is_not_upheld() {
        let (ids, c) = setup(3, 2);
        let mut dealings = deal_all(&ids, &c);
        dealings[0] = bad_dealing(&c, &ids[0], 2);
        // Trustee 3 tries to complain in trustee 2's name.
        let mut framed = c.complain(&ids[2], 3, &dealings[0], &mut OsRng);
        framed.accuser = 2;
        assert!(!c.upheld(&dealings[0], &framed));
    }

    #[test]
    fn forged_or_equivocating_dealings_are_disqualified() {
        let (ids, c) = setup(3, 2);
        let mut dealings = deal_all(&ids, &c);
        // A relay rewrites dealer 1's commitments: signature breaks.
        dealings[0].commitments[0] += RistrettoPoint::mul_base(&Scalar::ONE);
        // Dealer 2 publishes two different dealings.
        dealings.push(c.deal(&ids[1], &mut OsRng).unwrap());
        let q: Vec<u32> = c
            .qualified(&dealings, &[])
            .iter()
            .map(|d| d.dealer)
            .collect();
        assert_eq!(q, vec![3]);
    }

    #[test]
    fn identity_roundtrips_through_bytes() {
        let id = TrusteeIdentity::generate(&mut OsRng);
        let back = TrusteeIdentity::from_bytes(id.to_bytes()).unwrap();
        assert_eq!(back.public_bytes(), id.public_bytes());
        let reg = back.register("poll", &mut OsRng);
        assert_eq!(reg.key_bytes(), id.public_bytes());
        assert!(reg.verify("poll") && !reg.verify("other"));
    }

    #[test]
    fn registrations_are_validated() {
        let a = TrusteeIdentity::generate(&mut OsRng);
        let b = TrusteeIdentity::generate(&mut OsRng);
        let (ra, rb) = (
            a.register("poll", &mut OsRng),
            b.register("poll", &mut OsRng),
        );
        assert_eq!(
            Ceremony::new("poll", 3, &[ra.clone(), rb.clone()]).err(),
            Some(DkgError::InvalidThreshold)
        );
        assert_eq!(
            Ceremony::new("poll", 0, std::slice::from_ref(&ra)).err(),
            Some(DkgError::InvalidThreshold)
        );
        assert_eq!(
            Ceremony::new("other", 1, std::slice::from_ref(&ra)).err(),
            Some(DkgError::InvalidRegistration { index: 0 })
        );
        assert_eq!(
            Ceremony::new("poll", 1, &[ra.clone(), ra]).err(),
            Some(DkgError::DuplicateIdentity)
        );
        let outsider = TrusteeIdentity::generate(&mut OsRng);
        let c = Ceremony::new("poll", 1, &[rb]).unwrap();
        assert_eq!(
            c.deal(&outsider, &mut OsRng).err(),
            Some(DkgError::NotATrustee)
        );
    }
}
