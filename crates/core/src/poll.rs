//! Poll lifecycle: cast ballots, collect them on a public bulletin board,
//! tally homomorphically, and let anyone verify the result.

use std::fmt;

use curve25519_dalek::{constants::RISTRETTO_BASEPOINT_POINT as G, scalar::Scalar};
use rand_core::CryptoRngCore;
use sha2::{Digest, Sha256};

use crate::elgamal::{discrete_log, Ciphertext, PublicKey, SecretKey};
use crate::proofs::{BitProof, DecryptionProof, ValueProof};
use crate::ranked;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BallotKind {
    /// Mark every day that works: one encrypted 0/1 per option.
    Approval,
    /// Rank every day: an encrypted n×n permutation matrix (see [`crate::cast_ranked_ballot`]).
    Ranked,
}

/// A poll's public parameters. `options` are the candidate days.
#[derive(Clone, Debug)]
pub struct Poll {
    pub id: String,
    pub options: Vec<String>,
    pub kind: BallotKind,
    pub public_key: PublicKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncryptedChoice {
    pub ciphertext: Ciphertext,
    pub proof: BitProof,
}

/// One participant's ballot: encrypted 0/1 cells, each proven to be a bit.
/// Ranked ballots also prove that every row and column of the matrix sums to 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ballot {
    pub choices: Vec<EncryptedChoice>,
    pub sum_proofs: Vec<ValueProof>,
}

/// What a voter keeps to later find their ballot on the board. Reveals nothing about the vote.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Receipt(pub [u8; 32]);

/// Decrypted per-cell totals: per option for approval polls, per
/// (option, rank) for ranked polls (see [`crate::RankedResult`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TallyResult {
    pub counts: Vec<u64>,
    pub proofs: Vec<DecryptionProof>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    WrongBallotKind,
    WrongOptionCount { expected: usize, got: usize },
    WrongChoiceCount { expected: usize, got: usize },
    InvalidRanking,
    InvalidBallotProof { cell: usize },
    WrongSumProofCount,
    InvalidRowProof { option: usize },
    InvalidColumnProof { rank: usize },
    DuplicateBallot,
    WrongResultLength,
    InvalidDecryptionProof { cell: usize },
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
            Error::InvalidBallotProof { cell } => {
                write!(f, "cell {cell} is not a proven encryption of 0 or 1")
            }
            Error::WrongSumProofCount => write!(f, "ballot has the wrong number of sum proofs"),
            Error::InvalidRowProof { option } => {
                write!(f, "option {option} is not given exactly one rank")
            }
            Error::InvalidColumnProof { rank } => {
                write!(f, "rank {rank} is not given to exactly one option")
            }
            Error::DuplicateBallot => write!(f, "ballot already on the board"),
            Error::WrongResultLength => write!(f, "result does not cover every cell"),
            Error::InvalidDecryptionProof { cell } => {
                write!(f, "decryption proof for cell {cell} does not verify")
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
        public_key: PublicKey,
    ) -> Self {
        Poll {
            id: id.into(),
            options,
            kind,
            public_key,
        }
    }

    /// Number of encrypted cells per ballot, and hence of tallied counts.
    pub fn tally_len(&self) -> usize {
        let n = self.options.len();
        match self.kind {
            BallotKind::Approval => n,
            BallotKind::Ranked => n * n,
        }
    }

    /// Binds every proof to this exact poll (id, kind, options, key) plus a
    /// label and index, so proofs cannot be lifted from another poll or moved
    /// between cells.
    pub(crate) fn context(&self, label: &[u8], index: usize) -> Vec<u8> {
        let mut h = Sha256::new();
        h.update((self.id.len() as u64).to_le_bytes());
        h.update(self.id.as_bytes());
        h.update([self.kind as u8]);
        h.update((self.options.len() as u64).to_le_bytes());
        for o in &self.options {
            h.update((o.len() as u64).to_le_bytes());
            h.update(o.as_bytes());
        }
        h.update(self.public_key.to_bytes());
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
                &self.public_key,
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
        }
    }

    /// Encrypts a 0/1 for cell `i` with its bit proof, returning the randomness.
    pub(crate) fn encrypt_cell(
        &self,
        i: usize,
        bit: bool,
        rng: &mut impl CryptoRngCore,
    ) -> (EncryptedChoice, Scalar) {
        let (ciphertext, r) = self.public_key.encrypt(bit as u64, rng);
        let proof = BitProof::prove(
            &self.public_key,
            &ciphertext,
            bit,
            &r,
            &self.context(b"cell", i),
            rng,
        );
        (EncryptedChoice { ciphertext, proof }, r)
    }
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

/// Run by the key holder: decrypt only the aggregate, never individual ballots.
pub fn decrypt_tally(
    sk: &SecretKey,
    board: &BulletinBoard,
    rng: &mut impl CryptoRngCore,
) -> TallyResult {
    let max = board.ballots.len() as u64;
    let (counts, proofs) = board
        .encrypted_tally()
        .iter()
        .enumerate()
        .map(|(i, ct)| {
            let (point, proof) =
                DecryptionProof::prove(sk, ct, &board.poll.context(b"cell", i), rng);
            let count = discrete_log(&point, max)
                .expect("every ballot proved its cells are 0/1, so the sum is <= ballot count");
            (count, proof)
        })
        .unzip();
    TallyResult { counts, proofs }
}

/// Run by anyone: re-check every ballot, recompute the aggregate from scratch
/// and confirm each published count is its proven decryption.
pub fn verify_tally(board: &BulletinBoard, result: &TallyResult) -> Result<(), Error> {
    let poll = &board.poll;
    if result.counts.len() != poll.tally_len() || result.proofs.len() != poll.tally_len() {
        return Err(Error::WrongResultLength);
    }
    for ballot in &board.ballots {
        poll.verify_ballot(ballot)?;
    }
    for (i, ct) in board.encrypted_tally().iter().enumerate() {
        let count = result.counts[i];
        if count > board.ballots.len() as u64 {
            return Err(Error::CountOutOfRange { cell: i });
        }
        let plaintext = Scalar::from(count) * G;
        if !result.proofs[i].verify(&poll.public_key, ct, &plaintext, &poll.context(b"cell", i)) {
            return Err(Error::InvalidDecryptionProof { cell: i });
        }
    }
    Ok(())
}
