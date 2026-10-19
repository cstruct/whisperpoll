//! The websocket to the server: one per page, reconnecting on drop.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CloseEvent, MessageEvent, WebSocket};
use whisperpoll_protocol::{ClientMsg, PollView, ServerMsg};

thread_local! {
    static SOCKET: RefCell<Option<WebSocket>> = const { RefCell::new(None) };
}

/// Reactive connection state shared by the whole poll page.
#[derive(Clone, Copy)]
pub struct Conn {
    pub state: RwSignal<Option<PollView>>,
    pub error: RwSignal<Option<String>>,
    pub connected: RwSignal<bool>,
    /// The server doesn't know this poll (or invite), e.g. because it
    /// restarted and lost its in-memory polls. Retrying stops.
    pub gone: RwSignal<bool>,
}

impl Conn {
    pub fn new() -> Self {
        Conn {
            state: RwSignal::new(None),
            error: RwSignal::new(None),
            connected: RwSignal::new(false),
            gone: RwSignal::new(false),
        }
    }
}

/// First retry delay; doubles on each failed attempt up to `MAX_RETRY_MS`.
const RETRY_MS: i32 = 1500;
const MAX_RETRY_MS: i32 = 30_000;

pub fn connect(id: String, token: String, conn: Conn) {
    open(id, token, conn, RETRY_MS);
}

/// Opens the socket; `delay` is how long to wait before retrying if this
/// attempt never opens.
fn open(id: String, token: String, conn: Conn, delay: i32) {
    let location = window().location();
    let scheme = if location.protocol().as_deref() == Ok("https:") {
        "wss"
    } else {
        "ws"
    };
    let host = location.host().unwrap_or_default();
    let Ok(ws) = WebSocket::new(&format!("{scheme}://{host}/ws/{id}?t={token}")) else {
        conn.error.set(Some("couldn't open a connection".into()));
        return;
    };

    let opened = Rc::new(Cell::new(false));
    let onopen = Closure::<dyn FnMut()>::new({
        let opened = opened.clone();
        move || {
            opened.set(true);
            crate::auto::reset();
            conn.connected.set(true);
        }
    });
    ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
    onopen.forget();

    let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |ev: MessageEvent| {
        let Some(text) = ev.data().as_string() else {
            return;
        };
        match serde_json::from_str::<ServerMsg>(&text) {
            Ok(ServerMsg::State(view)) => conn.state.set(Some(*view)),
            Ok(ServerMsg::Error(e)) => conn.error.set(Some(e)),
            Err(e) => conn
                .error
                .set(Some(format!("unexpected message from server: {e}"))),
        }
    });
    ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    onmessage.forget();

    let onclose = Closure::<dyn FnMut(CloseEvent)>::new(move |_: CloseEvent| {
        conn.connected.set(false);
        let (id, token) = (id.clone(), token.clone());
        if opened.get() {
            // A drop after a working connection: retry soon.
            retry_after(id, token, conn, RETRY_MS);
            return;
        }
        // Never opened: the server may be down, or may have forgotten the
        // poll. Browsers hide the upgrade's status, so ask separately.
        spawn_local(async move {
            if poll_missing(&id, &token).await {
                conn.gone.set(true);
            } else {
                retry_after(id, token, conn, delay);
            }
        });
    });
    ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
    onclose.forget();

    SOCKET.with(|s| *s.borrow_mut() = Some(ws));
}

fn retry_after(id: String, token: String, conn: Conn, delay: i32) {
    let next = (delay * 2).min(MAX_RETRY_MS);
    let retry = Closure::once_into_js(move || open(id, token, conn, next));
    let _ = window()
        .set_timeout_with_callback_and_timeout_and_arguments_0(retry.unchecked_ref(), delay);
}

/// True only when the server answers that it doesn't know the poll or
/// invite; any other outcome (offline, 5xx) means "try again later".
async fn poll_missing(id: &str, token: &str) -> bool {
    let url = format!("/api/polls/{id}/check?t={token}");
    matches!(
        gloo_net::http::Request::get(&url).send().await,
        Ok(resp) if resp.status() == 404
    )
}

pub fn send(msg: &ClientMsg) {
    SOCKET.with(|s| {
        if let Some(ws) = s.borrow().as_ref() {
            if ws.ready_state() == WebSocket::OPEN {
                let _ = ws.send_with_str(&serde_json::to_string(msg).unwrap());
            }
        }
    });
}
