//! Exponential ElGamal over the Ristretto255 group.
//!
//! `Enc(m; r) = (r·G, m·G + r·H)` where `H = x·G` is the public key.
//! Adding two ciphertexts component-wise yields `Enc(m1 + m2; r1 + r2)`,
//! which is what makes the homomorphic tally possible. Decryption recovers
//! `m·G`; `m` itself is then found by a small discrete-log search, which is
//! cheap because vote counts are bounded by the number of participants.

use std::iter::Sum;
use std::ops::Add;

use curve25519_dalek::{
    constants::RISTRETTO_BASEPOINT_POINT as G, ristretto::RistrettoPoint, scalar::Scalar,
    traits::Identity,
};
use rand_core::CryptoRngCore;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PublicKey(pub(crate) RistrettoPoint);

pub struct SecretKey {
    x: Scalar,
    pk: PublicKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ciphertext {
    pub(crate) a: RistrettoPoint,
    pub(crate) b: RistrettoPoint,
}

impl SecretKey {
    pub fn generate(rng: &mut impl CryptoRngCore) -> Self {
        let x = Scalar::random(rng);
        SecretKey {
            x,
            pk: PublicKey(RistrettoPoint::mul_base(&x)),
        }
    }

    pub fn public_key(&self) -> PublicKey {
        self.pk
    }

    pub(crate) fn scalar(&self) -> &Scalar {
        &self.x
    }

    /// Returns `m·G`; [`SecretKey::decrypt`] also recovers `m`.
    pub fn decrypt_to_point(&self, ct: &Ciphertext) -> RistrettoPoint {
        ct.b - self.x * ct.a
    }

    pub fn decrypt(&self, ct: &Ciphertext, max: u64) -> Option<u64> {
        discrete_log(&self.decrypt_to_point(ct), max)
    }
}

impl PublicKey {
    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.compress().to_bytes()
    }

    /// Encrypts `m` and returns the randomness so callers can build proofs about it.
    pub fn encrypt(&self, m: u64, rng: &mut impl CryptoRngCore) -> (Ciphertext, Scalar) {
        let r = Scalar::random(rng);
        (self.encrypt_with(m, &r), r)
    }

    pub fn encrypt_with(&self, m: u64, r: &Scalar) -> Ciphertext {
        Ciphertext {
            a: RistrettoPoint::mul_base(r),
            b: RistrettoPoint::mul_base(&Scalar::from(m)) + r * self.0,
        }
    }
}

impl Ciphertext {
    /// Encryption of zero with zero randomness: the identity for `+`.
    pub fn zero() -> Self {
        Ciphertext {
            a: RistrettoPoint::identity(),
            b: RistrettoPoint::identity(),
        }
    }

    pub fn to_bytes(&self) -> [u8; 64] {
        let mut out = [0u8; 64];
        out[..32].copy_from_slice(self.a.compress().as_bytes());
        out[32..].copy_from_slice(self.b.compress().as_bytes());
        out
    }
}

impl Add for Ciphertext {
    type Output = Ciphertext;
    fn add(self, rhs: Ciphertext) -> Ciphertext {
        Ciphertext {
            a: self.a + rhs.a,
            b: self.b + rhs.b,
        }
    }
}

impl Sum for Ciphertext {
    fn sum<I: Iterator<Item = Ciphertext>>(iter: I) -> Ciphertext {
        iter.fold(Ciphertext::zero(), Add::add)
    }
}

/// Finds `m <= max` with `m·G == point`. Linear search is fine for poll-sized
/// counts; swap for baby-step giant-step if polls ever get large.
pub fn discrete_log(point: &RistrettoPoint, max: u64) -> Option<u64> {
    let mut acc = RistrettoPoint::identity();
    for m in 0..=max {
        if acc == *point {
            return Some(m);
        }
        acc += G;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand_core::OsRng;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let sk = SecretKey::generate(&mut OsRng);
        for m in [0, 1, 7, 42] {
            let (ct, _) = sk.public_key().encrypt(m, &mut OsRng);
            assert_eq!(sk.decrypt(&ct, 100), Some(m));
        }
    }

    #[test]
    fn addition_is_homomorphic() {
        let sk = SecretKey::generate(&mut OsRng);
        let pk = sk.public_key();
        let total: Ciphertext = [3, 0, 1, 5]
            .iter()
            .map(|&m| pk.encrypt(m, &mut OsRng).0)
            .sum();
        assert_eq!(sk.decrypt(&total, 100), Some(9));
    }

    #[test]
    fn encryption_is_randomized() {
        let pk = SecretKey::generate(&mut OsRng).public_key();
        assert_ne!(pk.encrypt(1, &mut OsRng).0, pk.encrypt(1, &mut OsRng).0);
    }

    #[test]
    fn wrong_key_does_not_decrypt() {
        let sk = SecretKey::generate(&mut OsRng);
        let other = SecretKey::generate(&mut OsRng);
        let (ct, _) = sk.public_key().encrypt(3, &mut OsRng);
        assert_eq!(other.decrypt(&ct, 100), None);
    }
}
