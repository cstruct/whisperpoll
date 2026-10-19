//! The Rust side of the app: everything cryptographic, so the Dart side only
//! shuttles JSON. Values cross the bridge as JSON strings of the protocol's
//! types (`PollView` in, `ClientMsg` out), which keeps the bridge small and
//! means the app runs exactly the code the web client and server tests run.

use rand_core::OsRng;
use whisperpoll_core::{cast_tiered_ballot, Receipt, TrusteeIdentity};
use whisperpoll_protocol::calendar::{self, Event};
use whisperpoll_protocol::client;
use whisperpoll_protocol::slot::Slot;
use whisperpoll_protocol::{ClientMsg, PollView};

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex<const N: usize>(hex: &str) -> Option<[u8; N]> {
    if hex.len() != 2 * N {
        return None;
    }
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

fn view(view_json: &str) -> Result<PollView, String> {
    serde_json::from_str(view_json).map_err(|e| format!("bad poll state: {e}"))
}

fn identity(hex: &str) -> Result<TrusteeIdentity, String> {
    from_hex::<32>(hex)
        .and_then(TrusteeIdentity::from_bytes)
        .ok_or_else(|| "bad trustee key".to_string())
}

fn msg_json(msg: &ClientMsg) -> String {
    serde_json::to_string(msg).expect("messages serialize")
}

/// A fresh trustee key, as hex, for the device's secure storage.
#[flutter_rust_bridge::frb(sync)]
pub fn new_identity() -> String {
    to_hex(&TrusteeIdentity::generate(&mut OsRng).to_bytes())
}

/// The public part of a trustee key, as hex.
#[flutter_rust_bridge::frb(sync)]
pub fn identity_public(identity_hex: String) -> Result<String, String> {
    Ok(to_hex(&identity(&identity_hex)?.public_bytes()))
}

pub struct StepOut {
    pub key: String,
    pub msg_json: String,
}

/// The next automatic trustee step (see `whisperpoll_protocol::client::next_step`),
/// skipping step keys already in `sent`.
pub fn next_step(
    view_json: String,
    identity_hex: String,
    sent: Vec<String>,
) -> Result<Option<StepOut>, String> {
    let view = view(&view_json)?;
    let me = identity(&identity_hex)?;
    let step = client::next_step(&view, &me, |k| sent.iter().any(|s| s == k), &mut OsRng);
    Ok(step.map(|s| StepOut {
        key: s.key,
        msg_json: msg_json(&s.msg),
    }))
}

pub struct BallotOut {
    pub msg_json: String,
    pub receipt_hex: String,
}

/// Encrypts a tiered ballot: `choices[i]` is the tier for option `i`, or
/// `None` for "can't make it".
pub fn cast_ballot(view_json: String, choices: Vec<Option<u32>>) -> Result<BallotOut, String> {
    let view = view(&view_json)?;
    let poll = view.poll().ok_or("the poll key isn't ready yet")?;
    let choices: Vec<Option<usize>> = choices.into_iter().map(|c| c.map(|t| t as usize)).collect();
    let ballot = cast_tiered_ballot(&poll, &choices, &mut OsRng).map_err(|e| e.to_string())?;
    Ok(BallotOut {
        receipt_hex: to_hex(&ballot.receipt().0),
        msg_json: msg_json(&ClientMsg::Vote(ballot)),
    })
}

// Counts are u32 so they arrive in Dart as plain ints.
pub struct RowOut {
    /// Canonical slot text.
    pub slot: String,
    pub counts: Vec<u32>,
    pub available: u32,
    pub score: u32,
    pub picked_earlier: bool,
}

pub struct OutcomeOut {
    pub rows: Vec<RowOut>,
    pub ballots: u32,
    pub winner_slot: Option<String>,
    pub winner_available: u32,
    pub verification_error: Option<String>,
    pub winner_checks_out: bool,
    pub decrypted_by: Vec<String>,
    pub mine: Option<bool>,
}

/// Every finished round, re-derived and verified on the device.
/// `receipts[r]` is this participant's receipt (hex) for round `r + 1`.
pub fn outcomes(
    view_json: String,
    receipts: Vec<Option<String>>,
) -> Result<Vec<OutcomeOut>, String> {
    let view = view(&view_json)?;
    let receipt_for = |round: usize| {
        // A ballot carried over keeps the receipt of the round it was cast in.
        receipts[..round.min(receipts.len())]
            .iter()
            .rev()
            .flatten()
            .next()
            .and_then(|hex| from_hex::<32>(hex))
            .map(Receipt)
    };
    Ok(client::finished_outcomes(&view, receipt_for)
        .into_iter()
        .map(|o| OutcomeOut {
            rows: o
                .rows
                .into_iter()
                .map(|r| RowOut {
                    slot: r.slot,
                    counts: r.counts.into_iter().map(|c| c as u32).collect(),
                    available: r.available as u32,
                    score: r.score as u32,
                    picked_earlier: r.picked_earlier,
                })
                .collect(),
            ballots: o.ballots as u32,
            winner_available: o.winner.as_ref().map_or(0, |p| p.available as u32),
            winner_slot: o.winner.map(|p| p.slot),
            verification_error: o.verification_error,
            winner_checks_out: o.winner_checks_out,
            decrypted_by: o.decrypted_by,
            mine: o.mine,
        })
        .collect())
}

/// An `.ics` file with an event per picked slot (canonical slot text).
#[flutter_rust_bridge::frb(sync)]
pub fn calendar_ics(
    title: String,
    poll_id: String,
    stamp: String,
    slots: Vec<String>,
    notes: Vec<String>,
) -> String {
    let events: Vec<Event> = slots
        .iter()
        .zip(notes)
        .filter_map(|(slot, description)| {
            Some(Event {
                slot: Slot::parse(slot)?,
                description,
            })
        })
        .collect();
    calendar::ics(&title, &poll_id, &stamp, &events)
}

/// Canonical slot text from a form's parts, or `None` if they're invalid.
/// `offset_minutes` is the device's UTC offset at that local date and time.
#[flutter_rust_bridge::frb(sync)]
pub fn make_slot(date: String, time: String, offset_minutes: i32, minutes: u32) -> Option<String> {
    Slot::new(&date, &time, offset_minutes, minutes).map(|s| s.to_string())
}

pub struct SlotTimes {
    /// Start, in milliseconds since the Unix epoch.
    pub start_millis: i64,
    pub minutes: u32,
}

/// When a slot starts and how long it lasts, for display in local time.
#[flutter_rust_bridge::frb(sync)]
pub fn slot_times(slot: String) -> Option<SlotTimes> {
    let slot = Slot::parse(&slot)?;
    Some(SlotTimes {
        start_millis: slot.start_epoch_millis(),
        minutes: slot.minutes,
    })
}

#[flutter_rust_bridge::frb(init)]
pub fn init_app() {
    flutter_rust_bridge::setup_default_user_utils();
}
