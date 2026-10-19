//! Tiered ballots: approval and ranking in one.
//!
//! A poll fixes `T` named preference tiers (e.g. "Prefer", "OK", "If needed");
//! tier 0 is the most preferred. A voter puts each day in at most one tier,
//! or in none to say "can't make it". Several days may share a tier (ties),
//! and days may be left out (partial ballots).
//!
//! For `n` options a ballot is an `n×T` grid of encrypted bits, row-major:
//! cell `(option, tier)` is 1 iff the voter put `option` in `tier`. Each cell
//! carries a bit proof, and each row's sum carries a bit proof too — so every
//! day is in at most one tier. Columns are unconstrained, which is what allows
//! ties.
//!
//! Because tiers have a fixed meaning rather than a relative one, there's no
//! need to forbid gaps (using tiers 0 and 2 but not 1): a voter is rating
//! days, not ordering them.

use std::cmp::Reverse;

use curve25519_dalek::scalar::Scalar;
use rand_core::CryptoRngCore;

use crate::poll::{sum_cells, Ballot, BallotKind, Error, Poll, SumProof, TallyResult};
use crate::proofs::BitProof;

/// Encrypts `choices[i]` for every option: `Some(t)` puts day `i` in tier `t`,
/// `None` means "can't make it".
pub fn cast_tiered_ballot(
    poll: &Poll,
    choices: &[Option<usize>],
    rng: &mut impl CryptoRngCore,
) -> Result<Ballot, Error> {
    let BallotKind::Tiered { tiers } = poll.kind else {
        return Err(Error::WrongBallotKind);
    };
    if tiers == 0 {
        return Err(Error::NoTiers);
    }
    let n = poll.options.len();
    if choices.len() != n {
        return Err(Error::WrongOptionCount {
            expected: n,
            got: choices.len(),
        });
    }
    if let Some(option) = choices.iter().position(|c| c.is_some_and(|t| t >= tiers)) {
        return Err(Error::InvalidTier { option });
    }

    let (cells, randomness): (Vec<_>, Vec<_>) = (0..n * tiers)
        .map(|i| poll.encrypt_cell(i, choices[i / tiers] == Some(i % tiers), rng))
        .unzip();

    let sum_proofs = (0..n)
        .map(|option| {
            let row = row(tiers, option);
            let ct = sum_cells(&cells, row.clone());
            let r: Scalar = row.map(|i| randomness[i]).sum();
            let available = choices[option].is_some();
            let ctx = poll.context(b"row", option);
            SumProof::ZeroOrOne(BitProof::prove(
                &poll.public_key,
                &ct,
                available,
                &r,
                &ctx,
                rng,
            ))
        })
        .collect();
    Ok(Ballot {
        choices: cells,
        sum_proofs,
    })
}

/// Checks every row sums to 0 or 1. Cell bit proofs are checked by the caller.
pub(crate) fn verify_rows(poll: &Poll, tiers: usize, ballot: &Ballot) -> Result<(), Error> {
    if tiers == 0 {
        return Err(Error::NoTiers);
    }
    if ballot.sum_proofs.len() != poll.options.len() {
        return Err(Error::WrongSumProofCount);
    }
    for (option, proof) in ballot.sum_proofs.iter().enumerate() {
        let ct = sum_cells(&ballot.choices, row(tiers, option));
        let ctx = poll.context(b"row", option);
        if !matches!(proof, SumProof::ZeroOrOne(p) if p.verify(&poll.public_key, &ct, &ctx)) {
            return Err(Error::InvalidRowProof { option });
        }
    }
    Ok(())
}

fn row(tiers: usize, option: usize) -> std::ops::Range<usize> {
    option * tiers..(option + 1) * tiers
}

/// Plaintext results of a tiered poll, derived from a verified [`TallyResult`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TieredResult {
    /// `counts[option][tier]`: how many voters put `option` in `tier`.
    pub counts: Vec<Vec<u64>>,
    /// How many voters can make each option (put it in any tier).
    pub available: Vec<u64>,
    /// Preference score per option: `T` points for tier 0, down to 1 for the
    /// last tier, 0 for "can't make it".
    pub score: Vec<u64>,
}

impl TieredResult {
    pub fn from_tally(poll: &Poll, tally: &TallyResult) -> Result<Self, Error> {
        let BallotKind::Tiered { tiers } = poll.kind else {
            return Err(Error::WrongBallotKind);
        };
        if tiers == 0 {
            return Err(Error::NoTiers);
        }
        if tally.counts.len() != poll.tally_len() {
            return Err(Error::WrongResultLength);
        }
        let counts: Vec<Vec<u64>> = tally.counts.chunks(tiers).map(<[u64]>::to_vec).collect();
        let available = counts.iter().map(|row| row.iter().sum()).collect();
        let score = counts
            .iter()
            .map(|row| {
                row.iter()
                    .enumerate()
                    .map(|(tier, &c)| c * (tiers - tier) as u64)
                    .sum()
            })
            .collect();
        Ok(TieredResult {
            counts,
            available,
            score,
        })
    }

    /// Option indices, best first: most people available, then highest
    /// preference score; remaining ties keep option order.
    pub fn order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.available.len()).collect();
        order.sort_by_key(|&o| Reverse((self.available[o], self.score[o])));
        order
    }
}
