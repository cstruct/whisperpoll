//! What this browser remembers per participant (keyed by invite token, so
//! several participants can share one browser while testing).

use leptos::prelude::window;
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use whisperpoll_core::{Receipt, TrusteeIdentity};

use crate::util::{from_hex, to_hex};

fn storage() -> Option<web_sys::Storage> {
    window().local_storage().ok().flatten()
}

fn key(poll: &str, token: &str, what: &str) -> String {
    format!("whisperpoll:{poll}:{token}:{what}")
}

fn get(key: &str) -> Option<String> {
    storage()?.get_item(key).ok().flatten()
}

fn set(key: &str, value: &str) {
    if let Some(s) = storage() {
        let _ = s.set_item(key, value);
    }
}

/// A poll this browser has opened, for the "your polls" lists.
#[derive(Serialize, Deserialize, Clone, PartialEq)]
pub struct Visited {
    pub id: String,
    pub token: String,
    pub title: String,
    /// When it was last opened, in milliseconds since the epoch.
    pub seen: f64,
}

const POLLS: &str = "whisperpoll:polls";

/// Polls opened in this browser, most recently opened first.
pub fn visited() -> Vec<Visited> {
    get(POLLS)
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

fn save_visited(polls: &[Visited]) {
    set(POLLS, &serde_json::to_string(polls).unwrap());
}

/// Puts this poll first in the list (or adds it).
pub fn remember(id: &str, token: &str, title: &str) {
    let mut polls = visited();
    polls.retain(|p| !(p.id == id && p.token == token));
    polls.insert(
        0,
        Visited {
            id: id.into(),
            token: token.into(),
            title: title.into(),
            seen: js_sys::Date::now(),
        },
    );
    save_visited(&polls);
}

pub fn forget(id: &str, token: &str) {
    let mut polls = visited();
    polls.retain(|p| !(p.id == id && p.token == token));
    save_visited(&polls);
}

const MUZAK: &str = "whisperpoll:muzak";

/// Whether the waiting music is muted (it plays unless muted).
pub fn muzak_muted() -> bool {
    get(MUZAK).as_deref() == Some("off")
}

pub fn set_muzak_muted(muted: bool) {
    set(MUZAK, if muted { "off" } else { "on" });
}

/// This participant's trustee identity, created on first use.
pub fn identity(poll: &str, token: &str) -> TrusteeIdentity {
    let k = key(poll, token, "identity");
    if let Some(id) = get(&k)
        .and_then(|hex| from_hex::<32>(&hex))
        .and_then(TrusteeIdentity::from_bytes)
    {
        return id;
    }
    let id = TrusteeIdentity::generate(&mut OsRng);
    set(&k, &to_hex(&id.to_bytes()));
    id
}

/// The start time and duration last used when suggesting a slot, so the
/// next suggestion only needs a date.
pub fn slot_defaults() -> (String, u32) {
    get("whisperpoll:slot-defaults")
        .and_then(|v| {
            let (time, minutes) = v.split_once('/')?;
            Some((time.to_string(), minutes.parse().ok()?))
        })
        .unwrap_or_else(|| ("18:00".into(), 120))
}

pub fn save_slot_defaults(time: &str, minutes: u32) {
    set("whisperpoll:slot-defaults", &format!("{time}/{minutes}"));
}

pub fn save_receipt(poll: &str, token: &str, round: usize, receipt: &Receipt) {
    set(
        &key(poll, token, &format!("receipt:{round}")),
        &to_hex(&receipt.0),
    );
}

/// The receipt of this participant's ballot in `round`: the one cast that
/// round, or else the latest earlier one, which was carried over.
pub fn receipt_for(poll: &str, token: &str, round: usize) -> Option<Receipt> {
    (1..=round).rev().find_map(|r| {
        get(&key(poll, token, &format!("receipt:{r}")))
            .and_then(|hex| from_hex::<32>(&hex))
            .map(Receipt)
    })
}

/// The tiers last voted, to prefill the next round's ballot.
pub fn save_choices(poll: &str, token: &str, choices: &[Option<usize>]) {
    set(
        &key(poll, token, "choices"),
        &serde_json::to_string(choices).unwrap(),
    );
}

pub fn choices(poll: &str, token: &str) -> Option<Vec<Option<usize>>> {
    serde_json::from_str(&get(&key(poll, token, "choices"))?).ok()
}
