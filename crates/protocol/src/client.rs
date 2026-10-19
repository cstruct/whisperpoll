//! What every client does with each [`PollView`] it receives, shared by the
//! web client and the mobile app so both run the same logic:
//!
//! * [`next_step`]: the trustee's part of the protocol, done without any
//!   clicks — register a key, deal shares, check received shares, and
//!   partially decrypt the tally.
//! * [`round_outcome`]: a finished round re-derived and verified locally.

use rand_core::CryptoRngCore;
use serde::{Deserialize, Serialize};
use whisperpoll_core::{partial_decrypt, verify_tally, Receipt, TieredResult, TrusteeIdentity};

use crate::{round_winner, ClientMsg, Phase, PollView};

/// A message to send, with a key naming the step. Callers remember the keys
/// they've sent on the current connection and skip repeats, so a burst of
/// state updates doesn't send a step twice before the server acknowledges it.
pub struct Step {
    pub key: String,
    pub msg: ClientMsg,
}

/// The next thing this client should send on its own, if any. `already_sent`
/// says whether a step key was sent on this connection; such steps are
/// skipped without doing their (sometimes costly) work.
pub fn next_step(
    view: &PollView,
    me: &TrusteeIdentity,
    already_sent: impl Fn(&str) -> bool,
    rng: &mut impl CryptoRngCore,
) -> Option<Step> {
    let fresh = |key: String| (!already_sent(&key)).then_some(key);

    // Trustee keys live on the device. If this participant registered from
    // somewhere else (say, a chat app's built-in browser), this device takes
    // over until voting opens; at most once per connection, so two open
    // devices can't keep taking it from each other.
    let registered_here = view.participants[view.me]
        .registration
        .as_ref()
        .is_some_and(|r| r.key_bytes() == me.public_bytes());
    let before_voting = matches!(
        view.phase,
        Phase::Inviting | Phase::Gathering | Phase::KeyCeremony
    );
    if before_voting && !registered_here {
        let key = fresh("register".into())?;
        let msg = ClientMsg::Register(me.register(&view.id, rng));
        return Some(Step { key, msg });
    }

    let (ceremony, cv) = (view.ceremony()?, view.ceremony.as_ref()?);
    let j = ceremony.index_of(me)?;
    match view.phase {
        Phase::KeyCeremony => {
            // Steps are per attempt: a restarted ceremony needs them again.
            let attempt = cv.attempt;
            if !cv.dealings.iter().any(|d| d.dealer == j) {
                let key = fresh(format!("deal:{attempt}"))?;
                let msg = ClientMsg::Deal(ceremony.deal(me, rng).ok()?);
                Some(Step { key, msg })
            } else if cv.dealings.len() == cv.trustees.len() && !cv.checked[j as usize - 1] {
                let key = fresh(format!("check:{attempt}"))?;
                let complaints = ceremony.check_dealings(me, &cv.dealings, rng).ok()?;
                let msg = ClientMsg::Checked {
                    attempt,
                    complaints,
                };
                Some(Step { key, msg })
            } else {
                None
            }
        }
        Phase::Tallying => {
            let round = view.rounds.last()?;
            if round.partials.iter().any(|p| p.trustee == j) {
                return None;
            }
            let key = fresh(format!("partial:{}", view.rounds.len()))?;
            let share = ceremony.key_share(me, &cv.dealings, &cv.complaints).ok()?;
            let partial = partial_decrypt(&share, &view.board()?, rng);
            Some(Step {
                key,
                msg: ClientMsg::Partial(partial),
            })
        }
        _ => None,
    }
}

/// One option's line in a round's results.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// Canonical slot text.
    pub slot: String,
    /// How many put this slot in each tier, most preferred first.
    pub counts: Vec<u64>,
    pub available: u64,
    pub score: u64,
    /// Picked in an earlier round, so it couldn't win this one.
    pub picked_earlier: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Pick {
    pub slot: String,
    pub available: u64,
}

/// A finished round, as re-derived and checked on this device.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// Best first.
    pub rows: Vec<Row>,
    pub ballots: u64,
    pub winner: Option<Pick>,
    /// Why the tally failed to verify, if it did.
    pub verification_error: Option<String>,
    /// Whether the server's pick for the round matches the verified totals.
    pub winner_checks_out: bool,
    pub decrypted_by: Vec<String>,
    /// Whether this participant's ballot is on the round's board, if they
    /// have a receipt for it.
    pub mine: Option<bool>,
}

impl Outcome {
    pub fn checks_out(&self) -> bool {
        self.verification_error.is_none() && self.winner_checks_out
    }
}

/// Round `round` (1-based), if it's finished. `receipt` is this
/// participant's receipt for that round, if any.
pub fn round_outcome(view: &PollView, round: usize, receipt: Option<Receipt>) -> Option<Outcome> {
    let r = view.rounds.get(round.checked_sub(1)?)?;
    let tally = r.tally.as_ref()?;
    let board = view.board_for(round)?;
    let verification_error = verify_tally(&board, tally).err().map(|e| e.to_string());
    let result = TieredResult::from_tally(&board.poll, tally).ok()?;
    let before = view.picked_before(round);
    let cv = view.ceremony.as_ref()?;
    Some(Outcome {
        rows: result
            .order()
            .into_iter()
            .map(|o| Row {
                slot: view.options[o].clone(),
                counts: result.counts[o].clone(),
                available: result.available[o],
                score: result.score[o],
                picked_earlier: before.contains(&o),
            })
            .collect(),
        ballots: tally.ballots,
        winner: r.winner.map(|o| Pick {
            slot: view.options[o].clone(),
            available: result.available[o],
        }),
        verification_error,
        winner_checks_out: round_winner(&result, &before) == r.winner,
        decrypted_by: tally
            .partials
            .iter()
            .map(|p| {
                let trustee = cv.trustees[p.trustee as usize - 1];
                view.participants[trustee].name.clone()
            })
            .collect(),
        mine: receipt.map(|rc| board.contains(&rc)),
    })
}

/// Every finished round's outcome, in order.
pub fn finished_outcomes(
    view: &PollView,
    receipt_for: impl Fn(usize) -> Option<Receipt>,
) -> Vec<Outcome> {
    (1..=view.rounds.len())
        .map_while(|r| round_outcome(view, r, receipt_for(r)))
        .collect()
}
