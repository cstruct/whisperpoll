//! Core cryptography for whisperpoll: verifiable, anonymous date polls.
//!
//! Three ballot kinds are supported:
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
//! No single party has to hold the decryption key. Trustees (e.g. the poll
//! creator and participants) run a [`Ceremony`] — a dealerless distributed
//! key generation — and any `t` of the `n` trustees can then jointly decrypt
//! the tally with [`partial_decrypt`] and [`combine_tally`]. A single
//! [`SecretKey`] still works as the 1-of-1 case via [`decrypt_tally`].
//!
//! Verifiability: a voter keeps the receipt (hash) of their ballot, checks it
//! appears on the public bulletin board, and re-runs [`verify_tally`] — which
//! re-derives the aggregate from every ballot on the board and checks the
//! trustees' partial-decryption proofs. If it passes, their vote is counted in the result.

mod dkg;
mod elgamal;
mod poll;
mod proofs;
mod ranked;
mod threshold;
mod tiered;
mod transcript;

pub use dkg::{Ceremony, Complaint, Dealing, DkgError, Registration, TrusteeIdentity};
pub use elgamal::{Ciphertext, PublicKey, SecretKey};
pub use poll::{
    cast_ballot, combine_tally, decrypt_tally, verify_tally, Ballot, BallotKind, BulletinBoard,
    EncryptedChoice, Error, Poll, Receipt, SumProof, TallyResult,
};
pub use proofs::{BitProof, DleqProof, SchnorrProof, ValueProof};
pub use ranked::{cast_ranked_ballot, RankedResult};
pub use threshold::{partial_decrypt, KeyShare, PartialDecryption, TallyKey};
pub use tiered::{cast_tiered_ballot, TieredResult};
