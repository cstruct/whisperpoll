//! What this browser remembers per participant (keyed by invite token, so
//! several participants can share one browser while testing).

use leptos::prelude::window;
use rand_core::OsRng;
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
