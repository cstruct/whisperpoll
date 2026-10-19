//! Fiat–Shamir transcript: turns interactive sigma protocols into
//! non-interactive proofs by hashing every public value into the challenge.

use curve25519_dalek::{ristretto::RistrettoPoint, scalar::Scalar};
use sha2::{Digest, Sha512};

pub(crate) struct Transcript(Sha512);

impl Transcript {
    pub fn new(protocol: &[u8]) -> Self {
        let mut t = Transcript(Sha512::new());
        t.append_bytes(b"domain", b"whisperpoll-v0");
        t.append_bytes(b"protocol", protocol);
        t
    }

    /// Length-prefixed so that distinct inputs can never collide by concatenation.
    pub fn append_bytes(&mut self, label: &[u8], bytes: &[u8]) {
        self.0.update((label.len() as u64).to_le_bytes());
        self.0.update(label);
        self.0.update((bytes.len() as u64).to_le_bytes());
        self.0.update(bytes);
    }

    pub fn append_point(&mut self, label: &[u8], p: &RistrettoPoint) {
        self.append_bytes(label, p.compress().as_bytes());
    }

    pub fn challenge(self) -> Scalar {
        Scalar::from_hash(self.0)
    }
}
