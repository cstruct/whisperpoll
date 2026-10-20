//! The open invite: the creator's QR sticker, and the page it leads to
//! (`/j/{poll}?c={code}`), where the invitee joins under their own name.

use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;
use whisperpoll_protocol::qr::{self, Piece, Qr, QUIET};
use whisperpoll_protocol::{Created, Invitation, JoinPoll};

use crate::deco::{Badges, Envelope, Star, Ticker};
use crate::store;

/// Module colors: all dark, so the code scans.
const TONES: [&str; 3] = ["#000000", "#1a0b3b", "#3b0f73"];

/// The QR code for `link` as a stylized SVG (see `protocol::qr`).
#[component]
pub fn QrCode(link: String) -> impl IntoView {
    let Some(code) = Qr::new(&link) else {
        return ().into_any();
    };
    let edge = (code.size + 2 * QUIET) as f64;
    let q = QUIET as f64;
    let mut paths = [String::new(), String::new(), String::new()];
    let mut finders = Vec::new();
    let mut badge = (0.0, 0.0);
    for piece in code.pieces() {
        match piece {
            Piece::Module { x, y, tone } => {
                let (x, y) = (x as f64 + q, y as f64 + q);
                paths[tone as usize].push_str(&format!(
                    "M{:.2} {:.2}h.84v.84h-.84z",
                    x + 0.08,
                    y + 0.08
                ));
            }
            Piece::Finder { x, y } => finders.push((x as f64 + q, y as f64 + q)),
            Piece::Badge { at, len } => badge = (at as f64 + q, len as f64),
        }
    }
    let (b, len) = badge;
    view! {
        <svg
            class="qr"
            viewBox=format!("0 0 {edge} {edge}")
            role="img"
            aria-label="QR code that opens the invite"
            shape-rendering="crispEdges"
        >
            <rect width=edge height=edge fill="#fff"></rect>
            {paths
                .into_iter()
                .zip(TONES)
                .map(|(d, fill)| view! { <path d=d fill=fill></path> })
                .collect_view()}
            {finders
                .into_iter()
                .map(|(x, y)| {
                    view! {
                        <rect x=x y=y width="7" height="7" fill="#000"></rect>
                        <rect x=x + 1.0 y=y + 1.0 width="5" height="5" fill="#fff"></rect>
                        <rect x=x + 2.0 y=y + 2.0 width="3" height="3" fill="#1a0b3b"></rect>
                    }
                })
                .collect_view()}
            <rect x=b y=b width=len height=len fill="#fff"></rect>
            <g transform=format!("translate({b} {b}) scale({})", len / 24.0)>
                <path
                    d="M12 1.5l3.1 7 7.6.7-5.8 5 1.8 7.5-6.7-4-6.7 4 1.8-7.5-5.8-5 7.6-.7z"
                    fill="#c6ff00"
                    stroke="#000"
                    stroke-width="1.6"
                    stroke-linejoin="round"
                ></path>
            </g>
        </svg>
    }
    .into_any()
}

/// The creator's QR sticker for the open invite, with its link.
#[component]
pub fn OpenInvite(
    poll: String,
    code: String,
    on_new: impl Fn() + Send + Sync + 'static,
) -> impl IntoView {
    let origin = window().location().origin().unwrap_or_default();
    let link = qr::join_link(&origin, &poll, &code);
    let copy = link.clone();
    view! {
        <div class="open-invite">
            <figure class="qr-sticker">
                <span class="sticker blink scan-me">"Scan me!!"</span>
                <Star class="star spin qr-star" />
                <QrCode link=link.clone() />
                <figcaption>"Anyone who scans this picks their own name and joins."</figcaption>
            </figure>
            <div class="row">
                <button class="secondary small" on:click=move |_| crate::util::copy_to_clipboard(&copy)>
                    "Copy join link"
                </button>
                <button class="secondary small" on:click=move |_| on_new()>
                    "New code"
                </button>
            </div>
            <p class="hint">"A new code retires this QR code: anyone who has it can't join with it anymore."</p>
        </div>
    }
}

async fn fetch_invitation(id: &str, code: &str) -> Option<Invitation> {
    let resp = gloo_net::http::Request::get(&format!("/api/polls/{id}/invitation?c={code}"))
        .send()
        .await
        .ok()?;
    if !resp.ok() {
        return None;
    }
    resp.json().await.ok()
}

async fn join(id: &str, req: JoinPoll) -> Result<Created, String> {
    let resp = gloo_net::http::Request::post(&format!("/api/polls/{id}/join"))
        .json(&req)
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.ok() {
        return Err(resp.text().await.unwrap_or_else(|_| "couldn't join".into()));
    }
    resp.json().await.map_err(|e| e.to_string())
}

/// `/j/{id}?c={code}`: join the poll under your own name.
#[component]
pub fn JoinPage(id: String, code: String) -> impl IntoView {
    // None while loading, Some(None) if the invite doesn't work.
    let invitation = RwSignal::new(None::<Option<Invitation>>);
    {
        let (id, code) = (id.clone(), code.clone());
        spawn_local(async move { invitation.set(Some(fetch_invitation(&id, &code).await)) });
    }
    let name = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        error.set(None);
        let (id, code) = (id.clone(), code.clone());
        let title = invitation
            .get_untracked()
            .flatten()
            .map(|i| i.title)
            .unwrap_or_default();
        spawn_local(async move {
            match join(
                &id,
                JoinPoll {
                    code,
                    name: name.get_untracked(),
                },
            )
            .await
            {
                Ok(c) => {
                    store::remember(&c.id, &c.token, &title);
                    let _ = window()
                        .location()
                        .set_href(&format!("/p/{}?t={}", c.id, c.token));
                }
                Err(e) => {
                    error.set(Some(e));
                    busy.set(false);
                }
            }
        });
    };

    view! {
        <main class="page narrow">
            <Ticker text="YOU'RE INVITED ★ PICK YOUR NAME ★ VOTES NOBODY CAN READ".to_string() />
            <header class="hero">
                <a class="brand" href="/">"~*~ whisperpoll ~*~"</a>
                <h1>"You're invited!!"</h1>
            </header>
            {move || match invitation.get() {
                None => view! { <p class="center">"Checking the invite…"</p> }.into_any(),
                Some(None) => view! {
                    <section class="card stack">
                        <h2>"This invite no longer works"</h2>
                        <p>
                            "Voting may have closed, the poll may be over a week old, or the creator made a new QR code. Ask them for a fresh one."
                        </p>
                    </section>
                }
                .into_any(),
                Some(Some(inv)) => view! {
                    <form class="card stack" on:submit=submit.clone()>
                        <h2><Envelope />{format!("{} invites you to {}", inv.from, inv.title)}</h2>
                        <p>"Pick the name everyone will see. Your device makes its own key; nobody can read your vote."</p>
                        <label class="field">
                            <span>"Your name"</span>
                            <input
                                placeholder="Alex"
                                required
                                autofocus
                                prop:value=move || name.get()
                                on:input=move |ev| name.set(event_target_value(&ev))
                            />
                        </label>
                        {move || error.get().map(|e| view! { <p class="error">{e}</p> })}
                        <button type="submit" disabled=move || busy.get()>
                            "Join the poll"
                        </button>
                    </form>
                }
                .into_any(),
            }}
            <Badges />
        </main>
    }
}
