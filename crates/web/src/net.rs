//! The websocket to the server: one per page, reconnecting on drop.

use std::cell::RefCell;

use leptos::prelude::*;
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
}

impl Conn {
    pub fn new() -> Self {
        Conn {
            state: RwSignal::new(None),
            error: RwSignal::new(None),
            connected: RwSignal::new(false),
        }
    }
}

pub fn connect(id: String, token: String, conn: Conn) {
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

    let onopen = Closure::<dyn FnMut()>::new(move || {
        crate::auto::reset();
        conn.connected.set(true);
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
        let retry = Closure::once_into_js(move || connect(id, token, conn));
        let _ = window()
            .set_timeout_with_callback_and_timeout_and_arguments_0(retry.unchecked_ref(), 1500);
    });
    ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
    onclose.forget();

    SOCKET.with(|s| *s.borrow_mut() = Some(ws));
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
