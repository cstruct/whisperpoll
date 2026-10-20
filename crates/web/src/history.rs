//! "Your polls": the polls this browser has opened, with their status from
//! the server. Home lists the running ones to resume; `/history` lists all.

use leptos::prelude::*;
use leptos::task::spawn_local;
use whisperpoll_protocol::{Phase, PollSummary};

use crate::deco::{Badges, Ticker};
use crate::store::{self, Visited};
use crate::util::pretty_slot;

#[derive(Clone, PartialEq)]
enum Status {
    Loading,
    Live(PollSummary),
    /// The server no longer has it: a week without changes, or a restart.
    Expired,
    Unreachable,
}

impl Status {
    fn running(&self) -> bool {
        match self {
            Status::Live(s) => !matches!(s.phase, Phase::Done | Phase::Failed { .. }),
            // Maybe; worth a try.
            Status::Unreachable => true,
            Status::Loading | Status::Expired => false,
        }
    }
}

async fn fetch_summary(poll: &Visited) -> Status {
    let url = format!("/api/polls/{}/summary?t={}", poll.id, poll.token);
    match gloo_net::http::Request::get(&url).send().await {
        Ok(resp) if resp.ok() => match resp.json().await {
            Ok(summary) => Status::Live(summary),
            Err(_) => Status::Unreachable,
        },
        Ok(resp) if resp.status() == 404 => Status::Expired,
        _ => Status::Unreachable,
    }
}

fn phase_text(phase: &Phase) -> &'static str {
    match phase {
        Phase::Inviting => "Inviting people",
        Phase::Gathering => "Suggesting times",
        Phase::KeyCeremony => "Creating the shared key",
        Phase::Voting => "Voting",
        Phase::Tallying => "Counting votes",
        Phase::Done => "Done",
        Phase::Failed { .. } => "Failed",
    }
}

/// The remembered polls, each with its status (fetched in the background).
/// `running_only` shows just the ones still in progress.
#[component]
pub fn PollList(running_only: bool) -> impl IntoView {
    let polls = RwSignal::new(
        store::visited()
            .into_iter()
            .map(|p| (p, Status::Loading))
            .collect::<Vec<_>>(),
    );
    for poll in polls.get_untracked().into_iter().map(|(p, _)| p) {
        spawn_local(async move {
            let status = fetch_summary(&poll).await;
            polls.update(|ps| {
                if let Some(entry) = ps
                    .iter_mut()
                    .find(|(p, _)| p.id == poll.id && p.token == poll.token)
                {
                    entry.1 = status;
                }
            });
        });
    }
    let forget = move |poll: &Visited| {
        store::forget(&poll.id, &poll.token);
        let (id, token) = (poll.id.clone(), poll.token.clone());
        polls.update(|ps| ps.retain(|(p, _)| !(p.id == id && p.token == token)));
    };
    let shown = move || {
        polls
            .get()
            .into_iter()
            .filter(|(_, s)| !running_only || s.running())
            .collect::<Vec<_>>()
    };

    move || {
        let rows = shown();
        if rows.is_empty() {
            let loading = polls.with(|ps| ps.iter().any(|(_, s)| *s == Status::Loading));
            let text = match (running_only, loading) {
                (_, true) => "Checking…",
                (true, false) => "Nothing running right now.",
                (false, false) => {
                    "No polls yet. Polls you create or open in this browser show up here."
                }
            };
            return view! { <p class="muted">{text}</p> }.into_any();
        }
        view! {
            <ul class="poll-list">
                {rows
                    .into_iter()
                    .map(|(poll, status)| view! { <PollRow poll status forget /> })
                    .collect_view()}
            </ul>
        }
        .into_any()
    }
}

#[component]
fn PollRow(
    poll: Visited,
    status: Status,
    forget: impl Fn(&Visited) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let href = format!("/p/{}?t={}", poll.id, poll.token);
    let (title, details) = match &status {
        Status::Live(s) => (s.title.clone(), live_details(s)),
        Status::Loading => (
            poll.title.clone(),
            view! { <p class="status">"…"</p> }.into_any(),
        ),
        Status::Unreachable => (
            poll.title.clone(),
            view! { <p class="status">"Couldn't reach the server"</p> }.into_any(),
        ),
        Status::Expired => (
            poll.title.clone(),
            view! {
                <p class="status">"Expired — polls are kept for a week after their last change"</p>
            }
            .into_any(),
        ),
    };
    let expired = status == Status::Expired;
    view! {
        <li class:expired=expired>
            <div class="poll-main">
                {if expired {
                    view! { <span class="poll-title">{title}</span> }.into_any()
                } else {
                    view! { <a class="poll-title" href=href>{title}</a> }.into_any()
                }}
                {details}
            </div>
            {expired.then(|| {
                let poll = poll.clone();
                view! {
                    <button class="secondary small" on:click=move |_| forget(&poll)>
                        "Remove"
                    </button>
                }
            })}
        </li>
    }
}

fn live_details(s: &PollSummary) -> AnyView {
    let mut status = format!(
        "{} · {} · {}",
        s.you,
        phase_text(&s.phase),
        match s.participants {
            1 => "just you".to_string(),
            n => format!("{n} people"),
        }
    );
    if let Phase::Failed { reason } = &s.phase {
        status = format!("{status}: {reason}");
    }
    let your_turn = s
        .your_turn
        .then(|| view! { <span class="tag">"Your turn"</span> });
    let picked = (!s.picked.is_empty()).then(|| {
        view! {
            <ul class="picked">
                {s.picked.iter().map(|p| view! { <li>{pretty_slot(p)}</li> }).collect_view()}
            </ul>
        }
    });
    view! {
        <p class="status">{status}{your_turn}</p>
        {picked}
    }
    .into_any()
}

#[component]
pub fn History() -> impl IntoView {
    view! {
        <main class="page narrow">
            <Ticker text="EVERY POLL YOU'VE OPENED IN THIS BROWSER ★ KEPT FOR A WEEK AFTER THE LAST CHANGE".to_string() />
            <header class="poll-header">
                <div>
                    <nav class="crumbs">
                        <a class="brand" href="/">"~*~ whisperpoll ~*~"</a>
                    </nav>
                    <h1>"Your polls"</h1>
                </div>
            </header>
            <section class="card stack">
                <p class="hint">
                    "Polls you've created or opened in this browser. The server keeps a poll for a week after its last change."
                </p>
                <PollList running_only=false />
            </section>
            <Badges />
        </main>
    }
}
