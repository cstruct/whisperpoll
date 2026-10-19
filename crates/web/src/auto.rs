//! The trustee's part of the protocol, done by the browser without any
//! clicks: register a key, deal shares, check received shares, and
//! partially decrypt the tally. Runs on every state update.

use std::cell::RefCell;
use std::collections::HashSet;

use rand_core::OsRng;
use whisperpoll_core::partial_decrypt;
use whisperpoll_protocol::{ClientMsg, Phase, PollView};

use crate::{net, store};

thread_local! {
    /// Steps already sent on this connection, so a burst of state updates
    /// doesn't send the same step twice before the server has acknowledged it.
    static SENT: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

fn once(step: &str) -> bool {
    SENT.with(|s| s.borrow_mut().insert(step.to_string()))
}

/// Forget what was sent: after a reconnect, the next state decides afresh.
pub fn reset() {
    SENT.with(|s| s.borrow_mut().clear());
}

pub fn run(view: &PollView, token: &str) {
    let me = store::identity(&view.id, token);
    // Trustee keys live in the browser. If this participant registered from
    // another browser (say, a chat app's built-in one), this browser takes
    // over until voting opens; at most once per connection, so two open
    // browsers can't keep taking it from each other.
    let registered_here = view.participants[view.me]
        .registration
        .as_ref()
        .is_some_and(|r| r.key_bytes() == me.public_bytes());
    if matches!(view.phase, Phase::Gathering | Phase::KeyCeremony)
        && !registered_here
        && once("register")
    {
        net::send(&ClientMsg::Register(me.register(&view.id, &mut OsRng)));
        return;
    }
    match view.phase {
        Phase::KeyCeremony => {
            let (Some(ceremony), Some(cv)) = (view.ceremony(), view.ceremony.as_ref()) else {
                return;
            };
            let Some(j) = ceremony.index_of(&me) else {
                return;
            };
            // Steps are per attempt: a restarted ceremony needs them again.
            let attempt = cv.attempt;
            if !cv.dealings.iter().any(|d| d.dealer == j) {
                if once(&format!("deal:{attempt}")) {
                    if let Ok(dealing) = ceremony.deal(&me, &mut OsRng) {
                        net::send(&ClientMsg::Deal(dealing));
                    }
                }
            } else if cv.dealings.len() == cv.trustees.len()
                && !cv.checked[j as usize - 1]
                && once(&format!("check:{attempt}"))
            {
                if let Ok(complaints) = ceremony.check_dealings(&me, &cv.dealings, &mut OsRng) {
                    net::send(&ClientMsg::Checked {
                        attempt,
                        complaints,
                    });
                }
            }
        }
        Phase::Tallying => {
            let (Some(ceremony), Some(cv)) = (view.ceremony(), view.ceremony.as_ref()) else {
                return;
            };
            let Some(j) = ceremony.index_of(&me) else {
                return;
            };
            let round = view.rounds.len();
            let done = view
                .rounds
                .last()
                .is_some_and(|r| r.partials.iter().any(|p| p.trustee == j));
            if done || !once(&format!("partial:{round}")) {
                return;
            }
            let share = ceremony.key_share(&me, &cv.dealings, &cv.complaints);
            if let (Ok(share), Some(board)) = (share, view.board()) {
                net::send(&ClientMsg::Partial(partial_decrypt(
                    &share, &board, &mut OsRng,
                )));
            }
        }
        _ => {}
    }
}
