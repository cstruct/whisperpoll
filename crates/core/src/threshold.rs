//! Threshold decryption: `t` of `n` trustees jointly decrypt the tally.
//!
//! The poll key `H = x·G` comes from a distributed key generation (see
//! [`crate::Ceremony`]): nobody ever knows `x`. Trustee `j` holds a Shamir
//! share `x_j = f(j)` of it, for a secret degree-`t − 1` polynomial `f` with
//! `f(0) = x`, and everyone knows the matching verification key `X_j = x_j·G`.
//!
//! To decrypt a tally ciphertext `(A, B)`, each participating trustee
//! publishes `D_j = x_j·A` with a proof that `log_G(X_j) == log_A(D_j)`. Any
//! `t` valid partials combine by Lagrange interpolation in the exponent:
//! `Σ λ_j·D_j = x·A`, so `B − Σ λ_j·D_j = m·G`. Fewer than `t` reveal nothing.
//!
//! A single [`SecretKey`] is the 1-of-1 case, so one code path serves both.

use curve25519_dalek::{ristretto::RistrettoPoint, scalar::Scalar};
use rand_core::CryptoRngCore;
use sha2::{Digest, Sha256};

use crate::elgamal::{Ciphertext, PublicKey, SecretKey};
use crate::poll::{BulletinBoard, Poll};
use crate::proofs::DleqProof;

/// The public side of a poll key: the encryption key plus what's needed to
/// check trustees' partial decryptions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TallyKey {
    pub public_key: PublicKey,
    threshold: usize,
    /// `verification_keys[j − 1] = x_j·G` for trustee index `j`.
    verification_keys: Vec<RistrettoPoint>,
}

impl TallyKey {
    pub(crate) fn new(
        public_key: PublicKey,
        threshold: usize,
        verification_keys: Vec<RistrettoPoint>,
    ) -> Self {
        TallyKey {
            public_key,
            threshold,
            verification_keys,
        }
    }

    /// How many trustees must take part in decryption.
    pub fn threshold(&self) -> usize {
        self.threshold
    }

    pub fn trustees(&self) -> usize {
        self.verification_keys.len()
    }

    fn verification_key(&self, trustee: u32) -> Option<&RistrettoPoint> {
        (trustee as usize)
            .checked_sub(1)
            .and_then(|i| self.verification_keys.get(i))
    }

    pub(crate) fn hash_into(&self, h: &mut Sha256) {
        h.update(self.public_key.to_bytes());
        h.update((self.threshold as u64).to_le_bytes());
        h.update((self.verification_keys.len() as u64).to_le_bytes());
        for x in &self.verification_keys {
            h.update(x.compress().as_bytes());
        }
    }
}

/// A single key holder: trustee 1 of 1.
impl From<PublicKey> for TallyKey {
    fn from(pk: PublicKey) -> Self {
        TallyKey::new(pk, 1, vec![pk.0])
    }
}

/// One trustee's secret share of the poll key.
pub struct KeyShare {
    pub(crate) index: u32,
    pub(crate) secret: Scalar,
}

impl KeyShare {
    pub fn index(&self) -> u32 {
        self.index
    }
}

impl From<&SecretKey> for KeyShare {
    fn from(sk: &SecretKey) -> Self {
        KeyShare {
            index: 1,
            secret: *sk.scalar(),
        }
    }
}

/// One trustee's contribution to decrypting every cell of the tally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartialDecryption {
    pub trustee: u32,
    shares: Vec<RistrettoPoint>,
    proofs: Vec<DleqProof>,
}

fn partial_context(poll: &Poll, cell: usize, trustee: u32) -> Vec<u8> {
    let mut ctx = poll.context(b"partial", cell);
    ctx.extend_from_slice(&trustee.to_le_bytes());
    ctx
}

/// Run by each trustee: partially decrypt the board's aggregate. Never touches
/// individual ballots.
pub fn partial_decrypt(
    share: &KeyShare,
    board: &BulletinBoard,
    rng: &mut impl CryptoRngCore,
) -> PartialDecryption {
    let (shares, proofs) = board
        .encrypted_tally()
        .iter()
        .enumerate()
        .map(|(cell, ct)| {
            let ctx = partial_context(&board.poll, cell, share.index);
            (
                share.secret * ct.a,
                DleqProof::prove(&share.secret, &ct.a, &ctx, rng),
            )
        })
        .unzip();
    PartialDecryption {
        trustee: share.index,
        shares,
        proofs,
    }
}

pub(crate) fn verify_partial(poll: &Poll, tally: &[Ciphertext], p: &PartialDecryption) -> bool {
    let Some(x) = poll.key.verification_key(p.trustee) else {
        return false;
    };
    p.shares.len() == tally.len()
        && p.proofs.len() == tally.len()
        && tally.iter().enumerate().all(|(cell, ct)| {
            let ctx = partial_context(poll, cell, p.trustee);
            p.proofs[cell].verify(x, &ct.a, &p.shares[cell], &ctx)
        })
}

/// Lagrange coefficients for interpolating `f(0)` from `f` at `indices`.
fn lagrange_at_zero(indices: &[u32]) -> Vec<Scalar> {
    indices
        .iter()
        .map(|&j| {
            let xj = Scalar::from(j);
            indices
                .iter()
                .filter(|&&m| m != j)
                .map(|&m| {
                    let xm = Scalar::from(m);
                    xm * (xm - xj).invert()
                })
                .product()
        })
        .collect()
}

/// `m·G` for every tally cell, from already-verified partials with distinct trustees.
pub(crate) fn combine(tally: &[Ciphertext], partials: &[PartialDecryption]) -> Vec<RistrettoPoint> {
    let indices: Vec<u32> = partials.iter().map(|p| p.trustee).collect();
    let lambdas = lagrange_at_zero(&indices);
    tally
        .iter()
        .enumerate()
        .map(|(cell, ct)| {
            let xa: RistrettoPoint = partials
                .iter()
                .zip(&lambdas)
                .map(|(p, l)| l * p.shares[cell])
                .sum();
            ct.b - xa
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_core::OsRng;

    #[test]
    fn lagrange_reconstructs_constant_term() {
        // f(z) = 7 + 3z + 5z², threshold 3: any 3 points give f(0) = 7.
        let f = |z: u64| Scalar::from(7 + 3 * z + 5 * z * z);
        for indices in [[1, 2, 3], [2, 4, 5], [5, 1, 3]] {
            let lambdas = lagrange_at_zero(&indices);
            let at_zero: Scalar = indices
                .iter()
                .zip(&lambdas)
                .map(|(&j, l)| l * f(j as u64))
                .sum();
            assert_eq!(at_zero, Scalar::from(7u64));
        }
    }

    #[test]
    fn single_key_is_one_of_one() {
        let sk = SecretKey::generate(&mut OsRng);
        let key = TallyKey::from(sk.public_key());
        assert_eq!((key.threshold(), key.trustees()), (1, 1));
        assert_eq!(KeyShare::from(&sk).index(), 1);
    }
}
