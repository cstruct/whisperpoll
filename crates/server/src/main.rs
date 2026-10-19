//! whisperpoll server: serves the web client and relays poll messages over
//! websockets. State is in memory only.

mod room;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::broadcast;
use tower_http::services::{ServeDir, ServeFile};
use whisperpoll_protocol::{ClientMsg, CreatePoll, Created, ServerMsg};

use room::{random_token, Room};

struct Entry {
    room: Room,
    /// Pinged after every change; each connection then sends its own view.
    changed: broadcast::Sender<()>,
}

#[derive(Clone, Default)]
struct AppState {
    polls: Arc<Mutex<HashMap<String, Entry>>>,
}

#[tokio::main]
async fn main() {
    let dist = std::env::var("WHISPERPOLL_DIST")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../web/dist")));
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);

    let static_files = ServeDir::new(&dist).fallback(ServeFile::new(dist.join("index.html")));
    let app = Router::new()
        .route("/api/polls", post(create_poll))
        .route("/ws/{id}", get(connect))
        .fallback_service(static_files)
        .with_state(AppState::default());

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    println!("whisperpoll on http://{addr} (serving {})", dist.display());
    axum::serve(listener, app).await.unwrap();
}

async fn create_poll(
    State(state): State<AppState>,
    Json(req): Json<CreatePoll>,
) -> Result<Json<Created>, (StatusCode, String)> {
    let id = random_token()[..12].to_string();
    let (room, token) =
        Room::new(id.clone(), &req.title, &req.name).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let (changed, _) = broadcast::channel(64);
    state
        .polls
        .lock()
        .unwrap()
        .insert(id.clone(), Entry { room, changed });
    Ok(Json(Created { id, token }))
}

#[derive(Deserialize)]
struct Join {
    t: String,
}

async fn connect(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(join): Query<Join>,
) -> Response {
    let who = {
        let polls = state.polls.lock().unwrap();
        polls.get(&id).and_then(|e| e.room.member_by_token(&join.t))
    };
    match who {
        Some(who) => ws.on_upgrade(move |socket| session(socket, state, id, who)),
        None => (StatusCode::NOT_FOUND, "unknown poll or invite").into_response(),
    }
}

/// Runs `f` on the room and returns its result plus the change notifier.
fn with_room<T>(
    state: &AppState,
    id: &str,
    f: impl FnOnce(&mut Room) -> T,
) -> Option<(T, broadcast::Sender<()>)> {
    let mut polls = state.polls.lock().unwrap();
    let entry = polls.get_mut(id)?;
    Some((f(&mut entry.room), entry.changed.clone()))
}

fn encode(msg: &ServerMsg) -> Message {
    Message::Text(serde_json::to_string(msg).unwrap().into())
}

async fn session(socket: WebSocket, state: AppState, id: String, who: usize) {
    let (mut sink, mut stream) = socket.split();
    let Some(((), changed)) = with_room(&state, &id, |r| r.set_online(who, true)) else {
        return;
    };
    let mut updates = changed.subscribe();
    let _ = changed.send(());

    loop {
        tokio::select! {
            incoming = stream.next() => {
                let text = match incoming {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => continue,
                };
                let result = match serde_json::from_str::<ClientMsg>(&text) {
                    Ok(msg) => with_room(&state, &id, |r| r.apply(who, msg)).map(|(r, _)| r),
                    Err(e) => Some(Err(format!("malformed message: {e}"))),
                };
                match result {
                    Some(Ok(())) => { let _ = changed.send(()); }
                    Some(Err(e)) => {
                        if sink.send(encode(&ServerMsg::Error(e))).await.is_err() { break; }
                    }
                    None => break,
                }
            }
            update = updates.recv() => {
                if let Err(broadcast::error::RecvError::Closed) = update { break; }
                let Some((view, _)) = with_room(&state, &id, |r| r.view(who)) else { break };
                if sink.send(encode(&ServerMsg::State(Box::new(view)))).await.is_err() { break; }
            }
        }
    }

    if with_room(&state, &id, |r| r.set_online(who, false)).is_some() {
        let _ = changed.send(());
    }
}
