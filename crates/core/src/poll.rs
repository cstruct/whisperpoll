//! Poll lifecycle: cast ballots, collect them on a public bulletin board,
//! tally homomorphically, and let anyone verify the result.

use std::fmt;

use curve25519_dalek::{constants::RISTRETTO_BASEPOINT_POINT as G, scalar::Scalar};
use rand_core::CryptoRngCore;
use sha2::{Digest, Sha256};

use crate::elgamal::{discrete_log, Ciphertext, SecretKey};
use crate::proofs::{BitProof, ValueProof};
use crate::threshold::{self, KeyShare, PartialDecryption, TallyKey};
use crate::{ranked, tiered};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BallotKind {
    /// Mark every day that works: one encrypted 0/1 per option.
    Approval,
    /// Rank every day: an encrypted n×n permutation matrix (see [`crate::cast_ranked_ballot`]).
    Ranked,
    /// Put each day in one of `tiers` preference levels, or leave it out as
    /// "can't make it". Ties and partial ballots allowed (see [`crate::cast_tiered_ballot`]).
    Tiered { tiers: usize },
}

impl BallotKind {
    /// Bound into every proof, so the ballot format is fixed before anyone votes.
    fn tag(&self) -> [u8; 9] {
        let (tag, tiers) = match self {
            BallotKind::Approval => (0, 0),
            BallotKind::Ranked => (1, 0),
            BallotKind::Tiered { tiers } => (2, *tiers as u64),
        };
        let mut out = [0; 9];
        out[0] = tag;
        out[1..].copy_from_slice(&tiers.to_le_bytes());
        out
    }
}

/// A poll's public parameters. `options` are the candidate days.
#[derive(Clone, Debug)]
pub struct Poll {
    pub id: String,
    pub options: Vec<String>,
    pub kind: BallotKind,
    pub key: TallyKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncryptedChoice {
    pub ciphertext: Ciphertext,
    pub proof: BitProof,
}

/// A proof about the homomorphic sum of some of a ballot's cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SumProof {
    /// The cells sum to exactly 1 (ranked rows and columns).
    One(ValueProof),
    /// The cells sum to 0 or 1 (tiered rows: at most one tier per day).
    ZeroOrOne(BitProof),
}

/// One participant's ballot: encrypted 0/1 cells, each proven to be a bit,
/// plus kind-specific proofs about row/column sums (none for approval).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ballot {
    pub choices: Vec<EncryptedChoice>,
    pub sum_proofs: Vec<SumProof>,
}

/// What a voter keeps to later find their ballot on the board. Reveals nothing about the vote.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Receipt(pub [u8; 32]);

/// Decrypted per-cell totals: per option for approval polls, per
/// (option, rank) for ranked polls (see [`crate::RankedResult`]), per
/// (option, tier) for tiered polls (see [`crate::TieredResult`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TallyResult {
    /// How many ballots were tallied.
    pub ballots: u64,
    pub counts: Vec<u64>,
    /// The trustees' proven partial decryptions the counts were combined from.
    pub partials: Vec<PartialDecryption>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    WrongBallotKind,
    WrongOptionCount { expected: usize, got: usize },
    WrongChoiceCount { expected: usize, got: usize },
    InvalidRanking,
    NoTiers,
    InvalidTier { option: usize },
    InvalidBallotProof { cell: usize },
    WrongSumProofCount,
    InvalidRowProof { option: usize },
    InvalidColumnProof { rank: usize },
    DuplicateBallot,
    WrongResultLength,
    WrongBallotCount,
    NotEnoughPartials { have: usize, need: usize },
    InvalidPartial { trustee: u32 },
    DuplicatePartial { trustee: u32 },
    CountMismatch { cell: usize },
    CountOutOfRange { cell: usize },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::WrongBallotKind => write!(f, "ballot kind does not match the poll"),
            Error::WrongOptionCount { expected, got } => {
                write!(f, "got {got} entries, poll has {expected} options")
            }
            Error::WrongChoiceCount { expected, got } => {
                write!(f, "ballot has {got} encrypted cells, expected {expected}")
            }
            Error::InvalidRanking => write!(f, "ranking must list every option exactly once"),
            Error::NoTiers => write!(f, "a tiered poll needs at least one tier"),
            Error::InvalidTier { option } => {
                write!(f, "option {option} is given a tier that doesn't exist")
            }
            Error::InvalidBallotProof { cell } => {
                write!(f, "cell {cell} is not a proven encryption of 0 or 1")
            }
            Error::WrongSumProofCount => write!(f, "ballot has the wrong number of sum proofs"),
            Error::InvalidRowProof { option } => {
                write!(f, "option {option} is not given a valid rank or tier")
            }
            Error::InvalidColumnProof { rank } => {
                write!(f, "rank {rank} is not given to exactly one option")
            }
            Error::DuplicateBallot => write!(f, "ballot already on the board"),
            Error::WrongResultLength => write!(f, "result does not cover every cell"),
            Error::WrongBallotCount => write!(f, "result's ballot count does not match the board"),
            Error::NotEnoughPartials { have, need } => {
                write!(f, "{have} valid partial decryptions, {need} needed")
            }
            Error::InvalidPartial { trustee } => {
                write!(
                    f,
                    "partial decryption from trustee {trustee} does not verify"
                )
            }
            Error::DuplicatePartial { trustee } => {
                write!(f, "trustee {trustee} contributed more than once")
            }
            Error::CountMismatch { cell } => {
                write!(
                    f,
                    "count for cell {cell} is not what the partials decrypt to"
                )
            }
            Error::CountOutOfRange { cell } => {
                write!(f, "count for cell {cell} is not a valid tally")
            }
        }
    }
}

impl std::error::Error for Error {}

impl Poll {
    pub fn new(
        id: impl Into<String>,
        options: Vec<String>,
        kind: BallotKind,
        key: impl Into<TallyKey>,
    ) -> Self {
        Poll {
            id: id.into(),
            options,
            kind,
            key: key.into(),
        }
    }

    /// Number of encrypted cells per ballot, and hence of tallied counts.
    pub fn tally_len(&self) -> usize {
        let n = self.options.len();
        match self.kind {
            BallotKind::Approval => n,
            BallotKind::Ranked => n * n,
            BallotKind::Tiered { tiers, .. } => n * tiers,
        }
    }

    /// Binds every proof to this exact poll (id, kind, options, key) plus a
    /// label and index, so proofs cannot be lifted from another poll or moved
    /// between cells.
    pub(crate) fn context(&self, label: &[u8], index: usize) -> Vec<u8> {
        let mut h = Sha256::new();
        h.update((self.id.len() as u64).to_le_bytes());
        h.update(self.id.as_bytes());
        h.update(self.kind.tag());
        h.update((self.options.len() as u64).to_le_bytes());
        for o in &self.options {
            h.update((o.len() as u64).to_le_bytes());
            h.update(o.as_bytes());
        }
        self.key.hash_into(&mut h);
        let mut ctx = h.finalize().to_vec();
        ctx.extend_from_slice(label);
        ctx.extend_from_slice(&(index as u64).to_le_bytes());
        ctx
    }

    pub fn verify_ballot(&self, ballot: &Ballot) -> Result<(), Error> {
        if ballot.choices.len() != self.tally_len() {
            return Err(Error::WrongChoiceCount {
                expected: self.tally_len(),
                got: ballot.choices.len(),
            });
        }
        for (i, choice) in ballot.choices.iter().enumerate() {
            if !choice.proof.verify(
                &self.key.public_key,
                &choice.ciphertext,
                &self.context(b"cell", i),
            ) {
                return Err(Error::InvalidBallotProof { cell: i });
            }
        }
        match self.kind {
            BallotKind::Approval if ballot.sum_proofs.is_empty() => Ok(()),
            BallotKind::Approval => Err(Error::WrongSumProofCount),
            BallotKind::Ranked => ranked::verify_permutation(self, ballot),
            BallotKind::Tiered { tiers, .. } => tiered::verify_rows(self, tiers, ballot),
        }
    }

    /// Encrypts a 0/1 for cell `i` with its bit proof, returning the randomness.
    pub(crate) fn encrypt_cell(
        &self,
        i: usize,
        bit: bool,
        rng: &mut impl CryptoRngCore,
    ) -> (EncryptedChoice, Scalar) {
        let (ciphertext, r) = self.key.public_key.encrypt(bit as u64, rng);
        let proof = BitProof::prove(
            &self.key.public_key,
            &ciphertext,
            bit,
            &r,
            &self.context(b"cell", i),
            rng,
        );
        (EncryptedChoice { ciphertext, proof }, r)
    }
}

/// Homomorphic sum of the given cells of a ballot.
pub(crate) fn sum_cells(
    choices: &[EncryptedChoice],
    cells: impl IntoIterator<Item = usize>,
) -> Ciphertext {
    cells.into_iter().map(|i| choices[i].ciphertext).sum()
}

impl Ballot {
    pub fn receipt(&self) -> Receipt {
        let mut h = Sha256::new();
        h.update(b"whisperpoll-receipt-v0");
        for c in &self.choices {
            h.update(c.ciphertext.to_bytes());
        }
        Receipt(h.finalize().into())
    }
}

/// Encrypts `approvals[i]` ("day i works for me") for every option, with proofs.
pub fn cast_ballot(
    poll: &Poll,
    approvals: &[bool],
    rng: &mut impl CryptoRngCore,
) -> Result<Ballot, Error> {
    if poll.kind != BallotKind::Approval {
        return Err(Error::WrongBallotKind);
    }
    if approvals.len() != poll.options.len() {
        return Err(Error::WrongOptionCount {
            expected: poll.options.len(),
            got: approvals.len(),
        });
    }
    let choices = approvals
        .iter()
        .enumerate()
        .map(|(i, &yes)| poll.encrypt_cell(i, yes, rng).0)
        .collect();
    Ok(Ballot {
        choices,
        sum_proofs: Vec::new(),
    })
}

/// Public, append-only list of ballots. In the webapp this is what the server
/// publishes; here it's in memory.
#[derive(Clone, Debug)]
pub struct BulletinBoard {
    pub poll: Poll,
    ballots: Vec<Ballot>,
}

impl BulletinBoard {
    pub fn new(poll: Poll) -> Self {
        BulletinBoard {
            poll,
            ballots: Vec::new(),
        }
    }

    pub fn submit(&mut self, ballot: Ballot) -> Result<Receipt, Error> {
        self.poll.verify_ballot(&ballot)?;
        let receipt = ballot.receipt();
        if self.contains(&receipt) {
            return Err(Error::DuplicateBallot);
        }
        self.ballots.push(ballot);
        Ok(receipt)
    }

    pub fn ballots(&self) -> &[Ballot] {
        &self.ballots
    }

    pub fn contains(&self, receipt: &Receipt) -> bool {
        self.ballots.iter().any(|b| b.receipt() == *receipt)
    }

    /// Per-cell sum of every ballot's ciphertexts. Deterministic from the
    /// board's contents, so anyone can recompute it.
    pub fn encrypted_tally(&self) -> Vec<Ciphertext> {
        (0..self.poll.tally_len())
            .map(|i| self.ballots.iter().map(|b| b.choices[i].ciphertext).sum())
            .collect()
    }
}

/// Run by anyone once at least `t` trustees have published partial
/// decryptions: checks each, ignores invalid ones, and combines the first `t`
/// valid ones into the counts.
pub fn combine_tally(
    board: &BulletinBoard,
    partials: &[PartialDecryption],
) -> Result<TallyResult, Error> {
    let tally = board.encrypted_tally();
    let need = board.poll.key.threshold();
    let mut used: Vec<PartialDecryption> = Vec::with_capacity(need);
    for p in partials {
        if used.len() == need {
            break;
        }
        if used.iter().all(|u| u.trustee != p.trustee)
            && threshold::verify_partial(&board.poll, &tally, p)
        {
            used.push(p.clone());
        }
    }
    if used.len() < need {
        return Err(Error::NotEnoughPartials {
            have: used.len(),
            need,
        });
    }
    let max = board.ballots.len() as u64;
    let counts = threshold::combine(&tally, &used)
        .iter()
        .enumerate()
        .map(|(cell, m)| discrete_log(m, max).ok_or(Error::CountOutOfRange { cell }))
        .collect::<Result<_, _>>()?;
    Ok(TallyResult {
        ballots: max,
        counts,
        partials: used,
    })
}

/// Shortcut for a poll with a single key holder (a 1-of-1 [`TallyKey`]).
pub fn decrypt_tally(
    sk: &SecretKey,
    board: &BulletinBoard,
    rng: &mut impl CryptoRngCore,
) -> Result<TallyResult, Error> {
    let partial = threshold::partial_decrypt(&KeyShare::from(sk), board, rng);
    combine_tally(board, &[partial])
}

/// Run by anyone: re-check every ballot, recompute the aggregate from scratch,
/// check every partial decryption, and confirm each published count is what
/// they combine to.
pub fn verify_tally(board: &BulletinBoard, result: &TallyResult) -> Result<(), Error> {
    let poll = &board.poll;
    if result.counts.len() != poll.tally_len() {
        return Err(Error::WrongResultLength);
    }
    if result.ballots != board.ballots.len() as u64 {
        return Err(Error::WrongBallotCount);
    }
    for ballot in &board.ballots {
        poll.verify_ballot(ballot)?;
    }
    let tally = board.encrypted_tally();
    if result.partials.len() < poll.key.threshold() {
        return Err(Error::NotEnoughPartials {
            have: result.partials.len(),
            need: poll.key.threshold(),
        });
    }
    for (i, p) in result.partials.iter().enumerate() {
        if result.partials[..i].iter().any(|q| q.trustee == p.trustee) {
            return Err(Error::DuplicatePartial { trustee: p.trustee });
        }
        if !threshold::verify_partial(poll, &tally, p) {
            return Err(Error::InvalidPartial { trustee: p.trustee });
        }
    }
    let plaintexts = threshold::combine(&tally, &result.partials);
    for (cell, (&count, m)) in result.counts.iter().zip(&plaintexts).enumerate() {
        if count > result.ballots {
            return Err(Error::CountOutOfRange { cell });
        }
        if Scalar::from(count) * G != *m {
            return Err(Error::CountMismatch { cell });
        }
    }
    Ok(())
}
