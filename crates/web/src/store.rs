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

pub fn save_receipt(poll: &str, token: &str, receipt: &Receipt) {
    set(&key(poll, token, "receipt"), &to_hex(&receipt.0));
}

pub fn receipt(poll: &str, token: &str) -> Option<Receipt> {
    get(&key(poll, token, "receipt"))
        .and_then(|hex| from_hex::<32>(&hex))
        .map(Receipt)
}
