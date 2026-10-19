//! Poll lifecycle: cast ballots, collect them on a public bulletin board,
//! tally homomorphically, and let anyone verify the result.

use std::fmt;

use curve25519_dalek::{constants::RISTRETTO_BASEPOINT_POINT as G, scalar::Scalar};
use rand_core::CryptoRngCore;
use sha2::{Digest, Sha256};

use crate::elgamal::{discrete_log, Ciphertext, PublicKey, SecretKey};
use crate::proofs::{BitProof, DecryptionProof};

/// A poll's public parameters. `options` are the candidate days.
#[derive(Clone, Debug)]
pub struct Poll {
    pub id: String,
    pub options: Vec<String>,
    pub public_key: PublicKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncryptedChoice {
    pub ciphertext: Ciphertext,
    pub proof: BitProof,
}

/// One participant's approvals: an encrypted 0/1 per option.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ballot {
    pub choices: Vec<EncryptedChoice>,
}

/// What a voter keeps to later find their ballot on the board. Reveals nothing about the vote.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Receipt(pub [u8; 32]);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TallyResult {
    pub counts: Vec<u64>,
    pub proofs: Vec<DecryptionProof>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    WrongOptionCount { expected: usize, got: usize },
    InvalidBallotProof { option: usize },
    DuplicateBallot,
    WrongResultLength,
    InvalidDecryptionProof { option: usize },
    CountOutOfRange { option: usize },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::WrongOptionCount { expected, got } => {
                write!(f, "ballot has {got} choices, poll has {expected} options")
            }
            Error::InvalidBallotProof { option } => {
                write!(f, "choice {option} is not a proven encryption of 0 or 1")
            }
            Error::DuplicateBallot => write!(f, "ballot already on the board"),
            Error::WrongResultLength => write!(f, "result does not cover every option"),
            Error::InvalidDecryptionProof { option } => {
                write!(f, "decryption proof for option {option} does not verify")
            }
            Error::CountOutOfRange { option } => {
                write!(f, "count for option {option} is not a valid tally")
            }
        }
    }
}

impl std::error::Error for Error {}

impl Poll {
    pub fn new(id: impl Into<String>, options: Vec<String>, public_key: PublicKey) -> Self {
        Poll { id: id.into(), options, public_key }
    }

    /// Binds every proof to this exact poll (id, options, key) and option index,
    /// so proofs cannot be lifted from another poll or shuffled between days.
    fn proof_context(&self, option: usize) -> Vec<u8> {
        let mut h = Sha256::new();
        h.update((self.id.len() as u64).to_le_bytes());
        h.update(self.id.as_bytes());
        h.update((self.options.len() as u64).to_le_bytes());
        for o in &self.options {
            h.update((o.len() as u64).to_le_bytes());
            h.update(o.as_bytes());
        }
        h.update(self.public_key.to_bytes());
        let mut ctx = h.finalize().to_vec();
        ctx.extend_from_slice(&(option as u64).to_le_bytes());
        ctx
    }

    pub fn verify_ballot(&self, ballot: &Ballot) -> Result<(), Error> {
        if ballot.choices.len() != self.options.len() {
            return Err(Error::WrongOptionCount {
                expected: self.options.len(),
                got: ballot.choices.len(),
            });
        }
        for (i, choice) in ballot.choices.iter().enumerate() {
            if !choice.proof.verify(&self.public_key, &choice.ciphertext, &self.proof_context(i)) {
                return Err(Error::InvalidBallotProof { option: i });
            }
        }
        Ok(())
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
    if approvals.len() != poll.options.len() {
        return Err(Error::WrongOptionCount { expected: poll.options.len(), got: approvals.len() });
    }
    let choices = approvals
        .iter()
        .enumerate()
        .map(|(i, &yes)| {
            let (ciphertext, r) = poll.public_key.encrypt(yes as u64, rng);
            let proof =
                BitProof::prove(&poll.public_key, &ciphertext, yes, &r, &poll.proof_context(i), rng);
            EncryptedChoice { ciphertext, proof }
        })
        .collect();
    Ok(Ballot { choices })
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
        BulletinBoard { poll, ballots: Vec::new() }
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

    /// Per-option sum of every ballot's ciphertexts. Deterministic from the
    /// board's contents, so anyone can recompute it.
    pub fn encrypted_tally(&self) -> Vec<Ciphertext> {
        (0..self.poll.options.len())
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
            let (point, proof) = DecryptionProof::prove(sk, ct, &board.poll.proof_context(i), rng);
            let count = discrete_log(&point, max)
                .expect("every ballot proved its choices are 0/1, so the sum is <= ballot count");
            (count, proof)
        })
        .unzip();
    TallyResult { counts, proofs }
}

/// Run by anyone: re-check every ballot, recompute the aggregate from scratch
/// and confirm each published count is its proven decryption.
pub fn verify_tally(board: &BulletinBoard, result: &TallyResult) -> Result<(), Error> {
    let poll = &board.poll;
    if result.counts.len() != poll.options.len() || result.proofs.len() != poll.options.len() {
        return Err(Error::WrongResultLength);
    }
    for ballot in &board.ballots {
        poll.verify_ballot(ballot)?;
    }
    for (i, ct) in board.encrypted_tally().iter().enumerate() {
        let count = result.counts[i];
        if count > board.ballots.len() as u64 {
            return Err(Error::CountOutOfRange { option: i });
        }
        let plaintext = Scalar::from(count) * G;
        if !result.proofs[i].verify(&poll.public_key, ct, &plaintext, &poll.proof_context(i)) {
            return Err(Error::InvalidDecryptionProof { option: i });
        }
    }
    Ok(())
}
