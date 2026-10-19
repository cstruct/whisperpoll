//! The trustee's part of the protocol, done by the browser without any
//! clicks (see [`whisperpoll_protocol::client::next_step`]). Runs on every
//! state update.

use std::cell::RefCell;
use std::collections::HashSet;

use rand_core::OsRng;
use whisperpoll_protocol::client::next_step;
use whisperpoll_protocol::PollView;

use crate::{net, store};

thread_local! {
    /// Step keys already sent on this connection.
    static SENT: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// Forget what was sent: after a reconnect, the next state decides afresh.
pub fn reset() {
    SENT.with(|s| s.borrow_mut().clear());
}

pub fn run(view: &PollView, token: &str) {
    let me = store::identity(&view.id, token);
    let sent = |key: &str| SENT.with(|s| s.borrow().contains(key));
    if let Some(step) = next_step(view, &me, sent, &mut OsRng) {
        SENT.with(|s| s.borrow_mut().insert(step.key));
        net::send(&step.msg);
    }
}
