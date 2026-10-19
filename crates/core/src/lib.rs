//! Core cryptography for whisperpoll: verifiable, anonymous date polls.
//!
//! Two ballot kinds are supported:
//!
//! * **Approval** — mark which days work. One exponential-ElGamal ciphertext
//!   per day encrypting `1` (yes) or `0` (no), each with a zero-knowledge proof
//!   that it really is 0 or 1.
//! * **Ranked** — order every day by preference. Encoded as an encrypted
//!   permutation matrix with proofs that it's a valid ranking, and tallied
//!   with Borda scores (see [`RankedResult`]).
//! * **Tiered** — put each day in a preference tier ("Prefer", "OK", "If
//!   needed") or leave it out as "can't make it". Ties and partial ballots
//!   are allowed; results give both availability and a preference score
//!   (see [`TieredResult`]).
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
mod ranked;
mod tiered;
mod transcript;

pub use elgamal::{Ciphertext, PublicKey, SecretKey};
pub use poll::{
    cast_ballot, decrypt_tally, verify_tally, Ballot, BallotKind, BulletinBoard, EncryptedChoice,
    Error, Poll, Receipt, SumProof, TallyResult,
};
pub use proofs::{BitProof, DecryptionProof, ValueProof};
pub use ranked::{cast_ranked_ballot, RankedResult};
pub use tiered::{cast_tiered_ballot, TieredResult};
