//! Core cryptography for whisperpoll: verifiable, anonymous date polls.
//!
//! Each participant marks which candidate days work for them. A ballot is one
//! exponential-ElGamal ciphertext per day, encrypting `1` (yes) or `0` (no),
//! plus a zero-knowledge proof that each ciphertext really is 0 or 1.
//!
//! Because ElGamal is additively homomorphic (in the exponent), anyone can
//! multiply all ballots together to get an encryption of the per-day totals
//! without decrypting a single individual ballot. Only that aggregate is
//! decrypted, together with a proof of correct decryption.
//!
//! Verifiability: a voter keeps the receipt (hash) of their ballot, checks it
//! appears on the public bulletin board, and re-runs [`verify_tally`] — which
//! re-derives the aggregate from every ballot on the board and checks the
//! decryption proofs. If it passes, their vote is counted in the result.

mod elgamal;
mod poll;
mod proofs;
mod transcript;

pub use elgamal::{Ciphertext, PublicKey, SecretKey};
pub use poll::{
    cast_ballot, decrypt_tally, verify_tally, Ballot, BulletinBoard, EncryptedChoice, Error,
    Poll, Receipt, TallyResult,
};
pub use proofs::{BitProof, DecryptionProof};
