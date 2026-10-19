//! Non-interactive zero-knowledge proofs (sigma protocols + Fiat–Shamir).
//!
//! * [`BitProof`]: a ciphertext encrypts 0 or 1, without revealing which
//!   (disjunctive Chaum–Pedersen, as in Helios). Stops a voter from casting
//!   `Enc(100)` and stuffing the tally.
//! * [`DecryptionProof`]: a claimed plaintext `M` is the correct decryption
//!   of a ciphertext under the poll key (Chaum–Pedersen equality of discrete
//!   logs). Stops the key holder from lying about the result.
//!
//! Proofs are stored in compact `(challenge, response)` form; the verifier
//! recomputes the prover's commitments and checks they hash to the challenge.

use curve25519_dalek::{
    constants::RISTRETTO_BASEPOINT_POINT as G, ristretto::RistrettoPoint, scalar::Scalar,
};
use rand_core::CryptoRngCore;

use crate::elgamal::{Ciphertext, PublicKey, SecretKey};
use crate::transcript::Transcript;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BitProof {
    c: [Scalar; 2],
    z: [Scalar; 2],
}

/// Proves a ciphertext encrypts a publicly known value, using the encryption
/// randomness as witness (no secret key needed). Used to show that a sum of
/// a voter's ciphertexts encrypts exactly 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValueProof {
    c: Scalar,
    z: Scalar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecryptionProof {
    c: Scalar,
    z: Scalar,
}

fn bit_challenge(
    pk: &PublicKey,
    ct: &Ciphertext,
    ctx: &[u8],
    commitments: &[(RistrettoPoint, RistrettoPoint); 2],
) -> Scalar {
    let mut t = Transcript::new(b"bit-proof");
    t.append_bytes(b"ctx", ctx);
    t.append_point(b"pk", &pk.0);
    t.append_point(b"a", &ct.a);
    t.append_point(b"b", &ct.b);
    for (a, b) in commitments {
        t.append_point(b"commit-a", a);
        t.append_point(b"commit-b", b);
    }
    t.challenge()
}

/// The commitments a branch-`i` prover would have sent, reconstructed from
/// its challenge and response: `a = z·G − c·A`, `b = z·H − c·(B − i·G)`.
fn bit_commitments(
    pk: &PublicKey,
    ct: &Ciphertext,
    i: usize,
    c: &Scalar,
    z: &Scalar,
) -> (RistrettoPoint, RistrettoPoint) {
    let shifted_b = if i == 0 { ct.b } else { ct.b - G };
    (
        RistrettoPoint::mul_base(z) - c * ct.a,
        z * pk.0 - c * shifted_b,
    )
}

impl BitProof {
    /// Proves `ct == pk.encrypt_with(bit as u64, r)`. `ctx` binds the proof to
    /// a specific poll and option so it cannot be replayed elsewhere.
    pub fn prove(
        pk: &PublicKey,
        ct: &Ciphertext,
        bit: bool,
        r: &Scalar,
        ctx: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Self {
        let real = bit as usize;
        let fake = 1 - real;

        // Simulate the branch we can't prove by picking its challenge and response first.
        let mut c = [Scalar::ZERO; 2];
        let mut z = [Scalar::ZERO; 2];
        c[fake] = Scalar::random(rng);
        z[fake] = Scalar::random(rng);

        // Honest commitment for the true branch.
        let w = Scalar::random(rng);
        let mut commitments = [(G, G); 2];
        commitments[fake] = bit_commitments(pk, ct, fake, &c[fake], &z[fake]);
        commitments[real] = (RistrettoPoint::mul_base(&w), w * pk.0);

        let challenge = bit_challenge(pk, ct, ctx, &commitments);
        c[real] = challenge - c[fake];
        z[real] = w + c[real] * r;

        BitProof { c, z }
    }

    pub fn verify(&self, pk: &PublicKey, ct: &Ciphertext, ctx: &[u8]) -> bool {
        let commitments = [
            bit_commitments(pk, ct, 0, &self.c[0], &self.z[0]),
            bit_commitments(pk, ct, 1, &self.c[1], &self.z[1]),
        ];
        self.c[0] + self.c[1] == bit_challenge(pk, ct, ctx, &commitments)
    }
}

fn value_challenge(
    pk: &PublicKey,
    ct: &Ciphertext,
    value: u64,
    ctx: &[u8],
    t1: &RistrettoPoint,
    t2: &RistrettoPoint,
) -> Scalar {
    let mut t = Transcript::new(b"value-proof");
    t.append_bytes(b"ctx", ctx);
    t.append_point(b"pk", &pk.0);
    t.append_point(b"a", &ct.a);
    t.append_point(b"b", &ct.b);
    t.append_bytes(b"value", &value.to_le_bytes());
    t.append_point(b"t1", t1);
    t.append_point(b"t2", t2);
    t.challenge()
}

impl ValueProof {
    /// Proves `ct == pk.encrypt_with(value, r)`, i.e. `log_G(A) == log_H(B − value·G)`.
    pub fn prove(
        pk: &PublicKey,
        ct: &Ciphertext,
        value: u64,
        r: &Scalar,
        ctx: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> Self {
        let w = Scalar::random(rng);
        let t1 = RistrettoPoint::mul_base(&w);
        let t2 = w * pk.0;
        let c = value_challenge(pk, ct, value, ctx, &t1, &t2);
        ValueProof { c, z: w + c * r }
    }

    pub fn verify(&self, pk: &PublicKey, ct: &Ciphertext, value: u64, ctx: &[u8]) -> bool {
        let d = ct.b - RistrettoPoint::mul_base(&Scalar::from(value));
        let t1 = RistrettoPoint::mul_base(&self.z) - self.c * ct.a;
        let t2 = self.z * pk.0 - self.c * d;
        self.c == value_challenge(pk, ct, value, ctx, &t1, &t2)
    }
}

fn decryption_challenge(
    pk: &PublicKey,
    ct: &Ciphertext,
    plaintext: &RistrettoPoint,
    ctx: &[u8],
    t1: &RistrettoPoint,
    t2: &RistrettoPoint,
) -> Scalar {
    let mut t = Transcript::new(b"decryption-proof");
    t.append_bytes(b"ctx", ctx);
    t.append_point(b"pk", &pk.0);
    t.append_point(b"a", &ct.a);
    t.append_point(b"b", &ct.b);
    t.append_point(b"plaintext", plaintext);
    t.append_point(b"t1", t1);
    t.append_point(b"t2", t2);
    t.challenge()
}

impl DecryptionProof {
    /// Decrypts `ct` to `M = m·G` and proves `log_G(H) == log_A(B − M)`,
    /// i.e. the same secret key was used for the public key and the decryption.
    pub fn prove(
        sk: &SecretKey,
        ct: &Ciphertext,
        ctx: &[u8],
        rng: &mut impl CryptoRngCore,
    ) -> (RistrettoPoint, Self) {
        let pk = sk.public_key();
        let plaintext = sk.decrypt_to_point(ct);
        let w = Scalar::random(rng);
        let t1 = RistrettoPoint::mul_base(&w);
        let t2 = w * ct.a;
        let c = decryption_challenge(&pk, ct, &plaintext, ctx, &t1, &t2);
        (
            plaintext,
            DecryptionProof {
                c,
                z: w + c * sk.scalar(),
            },
        )
    }

    pub fn verify(
        &self,
        pk: &PublicKey,
        ct: &Ciphertext,
        plaintext: &RistrettoPoint,
        ctx: &[u8],
    ) -> bool {
        let d = ct.b - plaintext;
        let t1 = RistrettoPoint::mul_base(&self.z) - self.c * pk.0;
        let t2 = self.z * ct.a - self.c * d;
        self.c == decryption_challenge(pk, ct, plaintext, ctx, &t1, &t2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_core::OsRng;

    #[test]
    fn bit_proofs_verify_for_zero_and_one() {
        let pk = SecretKey::generate(&mut OsRng).public_key();
        for bit in [false, true] {
            let (ct, r) = pk.encrypt(bit as u64, &mut OsRng);
            let proof = BitProof::prove(&pk, &ct, bit, &r, b"ctx", &mut OsRng);
            assert!(proof.verify(&pk, &ct, b"ctx"));
        }
    }

    #[test]
    fn bit_proof_cannot_be_forged_for_two() {
        let pk = SecretKey::generate(&mut OsRng).public_key();
        let (ct, r) = pk.encrypt(2, &mut OsRng);
        // A cheater runs the honest prover on an out-of-range plaintext.
        for claimed in [false, true] {
            let proof = BitProof::prove(&pk, &ct, claimed, &r, b"ctx", &mut OsRng);
            assert!(!proof.verify(&pk, &ct, b"ctx"));
        }
    }

    #[test]
    fn bit_proof_is_bound_to_context_and_ciphertext() {
        let pk = SecretKey::generate(&mut OsRng).public_key();
        let (ct, r) = pk.encrypt(1, &mut OsRng);
        let proof = BitProof::prove(&pk, &ct, true, &r, b"poll-a", &mut OsRng);
        assert!(!proof.verify(&pk, &ct, b"poll-b"));
        let (other, _) = pk.encrypt(1, &mut OsRng);
        assert!(!proof.verify(&pk, &other, b"poll-a"));
    }

    #[test]
    fn value_proof_holds_for_homomorphic_sums() {
        let pk = SecretKey::generate(&mut OsRng).public_key();
        let (c1, r1) = pk.encrypt(0, &mut OsRng);
        let (c2, r2) = pk.encrypt(1, &mut OsRng);
        let (c3, r3) = pk.encrypt(0, &mut OsRng);
        let sum = c1 + c2 + c3;
        let proof = ValueProof::prove(&pk, &sum, 1, &(r1 + r2 + r3), b"ctx", &mut OsRng);
        assert!(proof.verify(&pk, &sum, 1, b"ctx"));
        assert!(!proof.verify(&pk, &sum, 2, b"ctx"));
        assert!(!proof.verify(&pk, &sum, 1, b"other"));
        // Claiming the wrong value can't be made to verify either.
        let lie = ValueProof::prove(&pk, &sum, 2, &(r1 + r2 + r3), b"ctx", &mut OsRng);
        assert!(!lie.verify(&pk, &sum, 2, b"ctx"));
    }

    #[test]
    fn decryption_proof_rejects_wrong_plaintext() {
        let sk = SecretKey::generate(&mut OsRng);
        let pk = sk.public_key();
        let (ct, _) = pk.encrypt(5, &mut OsRng);
        let (m, proof) = DecryptionProof::prove(&sk, &ct, b"ctx", &mut OsRng);
        assert!(proof.verify(&pk, &ct, &m, b"ctx"));
        assert!(!proof.verify(&pk, &ct, &(m + G), b"ctx"));
    }
}
