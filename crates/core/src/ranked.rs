//! Ranked ballots as encrypted permutation matrices.
//!
//! For `n` options a ballot is an `n×n` grid of encrypted bits, row-major:
//! cell `(option, rank)` is 1 iff the voter put `option` at `rank` (0 = top).
//! Each cell carries a bit proof, and the voter additionally proves that every
//! row and every column sums to exactly 1. A 0/1 matrix with unit row and
//! column sums is a permutation matrix, so the ballot is provably a complete,
//! strict ranking — without revealing it.
//!
//! Summing ballots gives, per cell, how many voters put each option at each
//! rank. That position matrix is decrypted and Borda scores derived from it in
//! the clear. Instant-runoff is deliberately not offered: its elimination
//! rounds need each ballot individually, which a homomorphic tally can't do.

use curve25519_dalek::scalar::Scalar;
use rand_core::CryptoRngCore;

use crate::elgamal::{Ciphertext, PublicKey};
use crate::poll::{sum_cells, Ballot, BallotKind, Error, Poll, SumProof, TallyResult};
use crate::proofs::ValueProof;

/// Encrypts `ranking` (option indices, most preferred first; every option exactly once).
pub fn cast_ranked_ballot(
    poll: &Poll,
    ranking: &[usize],
    rng: &mut impl CryptoRngCore,
) -> Result<Ballot, Error> {
    if poll.kind != BallotKind::Ranked {
        return Err(Error::WrongBallotKind);
    }
    let n = poll.options.len();
    if ranking.len() != n {
        return Err(Error::WrongOptionCount {
            expected: n,
            got: ranking.len(),
        });
    }
    let mut rank_of = vec![None; n];
    for (rank, &option) in ranking.iter().enumerate() {
        if option >= n || rank_of[option].is_some() {
            return Err(Error::InvalidRanking);
        }
        rank_of[option] = Some(rank);
    }

    let (choices, randomness): (Vec<_>, Vec<_>) = (0..n * n)
        .map(|i| poll.encrypt_cell(i, rank_of[i / n] == Some(i % n), rng))
        .unzip();

    let pk = &poll.key.public_key;
    let mut sum_proofs = Vec::with_capacity(2 * n);
    for option in 0..n {
        let cells = row(n, option);
        let ct = sum_cells(&choices, cells.iter().copied());
        let r: Scalar = cells.iter().map(|&i| randomness[i]).sum();
        sum_proofs.push(SumProof::One(ValueProof::prove(
            pk,
            &ct,
            1,
            &r,
            &poll.context(b"row", option),
            rng,
        )));
    }
    for rank in 0..n {
        let cells = column(n, rank);
        let ct = sum_cells(&choices, cells.iter().copied());
        let r: Scalar = cells.iter().map(|&i| randomness[i]).sum();
        sum_proofs.push(SumProof::One(ValueProof::prove(
            pk,
            &ct,
            1,
            &r,
            &poll.context(b"col", rank),
            rng,
        )));
    }
    Ok(Ballot {
        choices,
        sum_proofs,
    })
}

/// Checks the row/column sum proofs. Cell bit proofs are checked by the caller.
pub(crate) fn verify_permutation(poll: &Poll, ballot: &Ballot) -> Result<(), Error> {
    let n = poll.options.len();
    if ballot.sum_proofs.len() != 2 * n {
        return Err(Error::WrongSumProofCount);
    }
    let pk = &poll.key.public_key;
    let (row_proofs, col_proofs) = ballot.sum_proofs.split_at(n);
    for (option, proof) in row_proofs.iter().enumerate() {
        let ct = sum_cells(&ballot.choices, row(n, option));
        if !proves_one(proof, pk, &ct, &poll.context(b"row", option)) {
            return Err(Error::InvalidRowProof { option });
        }
    }
    for (rank, proof) in col_proofs.iter().enumerate() {
        let ct = sum_cells(&ballot.choices, column(n, rank));
        if !proves_one(proof, pk, &ct, &poll.context(b"col", rank)) {
            return Err(Error::InvalidColumnProof { rank });
        }
    }
    Ok(())
}

fn row(n: usize, option: usize) -> Vec<usize> {
    (0..n).map(|rank| option * n + rank).collect()
}

fn column(n: usize, rank: usize) -> Vec<usize> {
    (0..n).map(|option| option * n + rank).collect()
}

fn proves_one(proof: &SumProof, pk: &PublicKey, ct: &Ciphertext, ctx: &[u8]) -> bool {
    matches!(proof, SumProof::One(p) if p.verify(pk, ct, 1, ctx))
}

/// Plaintext results of a ranked poll, derived from a verified [`TallyResult`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RankedResult {
    /// `positions[option][rank]`: how many voters put `option` at `rank`.
    pub positions: Vec<Vec<u64>>,
    /// Borda score per option: `n − 1` points for a first place, down to 0 for last.
    pub borda: Vec<u64>,
}

impl RankedResult {
    pub fn from_tally(poll: &Poll, tally: &TallyResult) -> Result<Self, Error> {
        if poll.kind != BallotKind::Ranked {
            return Err(Error::WrongBallotKind);
        }
        if tally.counts.len() != poll.tally_len() {
            return Err(Error::WrongResultLength);
        }
        let n = poll.options.len();
        let positions: Vec<Vec<u64>> = tally.counts.chunks(n).map(<[u64]>::to_vec).collect();
        let borda = positions
            .iter()
            .map(|ranks| {
                ranks
                    .iter()
                    .enumerate()
                    .map(|(rank, &c)| c * (n - 1 - rank) as u64)
                    .sum()
            })
            .collect();
        Ok(RankedResult { positions, borda })
    }

    /// Option indices from highest to lowest Borda score; ties keep option order.
    pub fn order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.borda.len()).collect();
        order.sort_by_key(|&o| std::cmp::Reverse(self.borda[o]));
        order
    }
}
