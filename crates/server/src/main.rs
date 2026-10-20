//! whisperpoll server: serves the web client and relays poll messages over
//! websockets. Polls live in Redis when `REDIS_URL` is set (so several
//! machines can serve them), in memory otherwise; see `store`.

mod push;
mod room;
mod store;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

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
use whisperpoll_protocol::{
    ClientMsg, CreatePoll, Created, Invitation, JoinPoll, PollSummary, ServerMsg,
};

use room::{random_token, Room};
use store::{Store, HEARTBEAT};

#[derive(Clone)]
struct AppState {
    store: Store,
    notifier: Arc<dyn push::Notifier>,
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
    let store = Store::from_env().await.expect("couldn't connect to Redis");
    let polls = store.describe();

    let static_files = ServeDir::new(&dist).fallback(ServeFile::new(dist.join("index.html")));
    let app = Router::new()
        .route("/api/polls", post(create_poll))
        .route("/api/polls/{id}/check", get(check))
        .route("/api/polls/{id}/summary", get(summary))
        .route("/api/polls/{id}/invitation", get(invitation))
        .route("/api/polls/{id}/join", post(join))
        .route("/ws/{id}", get(connect))
        .fallback_service(static_files)
        .with_state(AppState {
            store,
            notifier: push::from_env(),
        });

    // Local by default; containers set HOST=0.0.0.0.
    let host: std::net::IpAddr = std::env::var("HOST")
        .ok()
        .and_then(|h| h.parse().ok())
        .unwrap_or([127, 0, 0, 1].into());
    let addr = SocketAddr::from((host, port));
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    println!(
        "whisperpoll on http://{addr} (serving {}, polls {polls})",
        dist.display()
    );
    axum::serve(listener, app).await.unwrap();
}

fn storage_error(e: String) -> (StatusCode, String) {
    eprintln!("{e}");
    (StatusCode::SERVICE_UNAVAILABLE, e)
}

async fn create_poll(
    State(state): State<AppState>,
    Json(req): Json<CreatePoll>,
) -> Result<Json<Created>, (StatusCode, String)> {
    let id = random_token()[..12].to_string();
    let (room, token) =
        Room::new(id.clone(), &req.title, &req.name).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    state
        .store
        .create(&id, &room)
        .await
        .map_err(storage_error)?;
    Ok(Json(Created { id, token }))
}

#[derive(Deserialize)]
struct Join {
    t: String,
}

/// Whether `id` is a poll the server knows and `t` one of its invites. The
/// client asks this when its websocket fails, since browsers hide the 404.
async fn check(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(join): Query<Join>,
) -> StatusCode {
    match state
        .store
        .read(&id, |r, _| r.member_by_token(&join.t))
        .await
    {
        Ok(Some(Some(_))) => StatusCode::NO_CONTENT,
        Ok(_) => StatusCode::NOT_FOUND,
        Err(e) => storage_error(e).0,
    }
}

/// A short status of the poll for invite `t`, for history and resume lists.
async fn summary(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(join): Query<Join>,
) -> Result<Json<PollSummary>, (StatusCode, String)> {
    let summary = state
        .store
        .read(&id, |r, _| Some(r.summary(r.member_by_token(&join.t)?)))
        .await
        .map_err(storage_error)?;
    match summary.flatten() {
        Some(summary) => Ok(Json(summary)),
        None => Err((StatusCode::NOT_FOUND, "unknown poll or invite".into())),
    }
}

#[derive(Deserialize)]
struct Code {
    c: String,
}

/// What the open invite (QR code) `c` is for; 404 if it doesn't work (any
/// more).
async fn invitation(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(code): Query<Code>,
) -> Result<Json<Invitation>, (StatusCode, String)> {
    let found = state
        .store
        .read(&id, |r, _| r.invitation(&code.c))
        .await
        .map_err(storage_error)?;
    match found.flatten() {
        Some(invitation) => Ok(Json(invitation)),
        None => Err((StatusCode::NOT_FOUND, "this invite no longer works".into())),
    }
}

/// Joins through the open invite under a name of the joiner's choosing.
async fn join(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<JoinPoll>,
) -> Result<Json<Created>, (StatusCode, String)> {
    let joined = state
        .store
        .update(&id, |r, _| r.join(&req.code, &req.name))
        .await
        .map_err(storage_error)?;
    match joined {
        Some(Ok(token)) => Ok(Json(Created { id, token })),
        Some(Err(e)) => Err((StatusCode::BAD_REQUEST, e)),
        None => Err((StatusCode::NOT_FOUND, "this poll no longer exists".into())),
    }
}

async fn connect(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(join): Query<Join>,
) -> Response {
    match state
        .store
        .read(&id, |r, _| r.member_by_token(&join.t))
        .await
    {
        Ok(Some(Some(who))) => ws.on_upgrade(move |socket| session(socket, state, id, who)),
        Ok(_) => (StatusCode::NOT_FOUND, "unknown poll or invite").into_response(),
        Err(e) => storage_error(e).into_response(),
    }
}

fn encode(msg: &ServerMsg) -> Message {
    Message::Text(serde_json::to_string(msg).unwrap().into())
}

async fn session(socket: WebSocket, state: AppState, id: String, who: usize) {
    let (mut sink, mut stream) = socket.split();
    let store = &state.store;
    let conn_id = random_token();
    let mut updates = store.subscribe(&id);
    store.set_online(&id, who, &conn_id, true).await;
    // Everyone sees them come online, and they get their first view.
    store.notify(&id).await;
    let mut heartbeat = tokio::time::interval(HEARTBEAT);

    loop {
        tokio::select! {
            incoming = stream.next() => {
                let text = match incoming {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => continue,
                };
                let result = match serde_json::from_str::<ClientMsg>(&text) {
                    Ok(msg) => store
                        .update(&id, |r, online| (r.apply(who, msg), r.pushes(online)))
                        .await
                        .map(|done| {
                            done.map(|(result, pushes)| {
                                send_pushes(&state, pushes);
                                result
                            })
                        }),
                    Err(e) => Ok(Some(Err(format!("malformed message: {e}")))),
                };
                let error = match result {
                    Ok(Some(Ok(()))) => continue,
                    Ok(Some(Err(e))) | Err(e) => e,
                    // The poll expired under us.
                    Ok(None) => break,
                };
                if sink.send(encode(&ServerMsg::Error(error))).await.is_err() { break; }
            }
            update = updates.recv() => {
                if let Err(broadcast::error::RecvError::Closed) = update { break; }
                let view = match store.read(&id, |r, online| r.view(who, online)).await {
                    Ok(Some(view)) => view,
                    Ok(None) => break,
                    Err(e) => { eprintln!("{e}"); continue; }
                };
                if sink.send(encode(&ServerMsg::State(Box::new(view)))).await.is_err() { break; }
            }
            _ = heartbeat.tick() => store.set_online(&id, who, &conn_id, true).await,
        }
    }

    store.set_online(&id, who, &conn_id, false).await;
    // They may have left with something to do: that's what pushes are for.
    // The update also tells everyone they went offline.
    if let Ok(Some(pushes)) = store.update(&id, |r, online| r.pushes(online)).await {
        send_pushes(&state, pushes);
    }
}

fn send_pushes(state: &AppState, pushes: Vec<(Vec<push::Device>, push::Push)>) {
    for (devices, push) in pushes {
        state.notifier.send(&devices, &push);
    }
}
