use std::time::Duration;

use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use rand_core::OsRng;
use web_sys::HtmlInputElement;
use whisperpoll_core::cast_tiered_ballot;
use whisperpoll_protocol::calendar::{self, Event};
use whisperpoll_protocol::client::{finished_outcomes, Outcome};
use whisperpoll_protocol::slot::Slot;
use whisperpoll_protocol::{ClientMsg, Phase, PollView, TIERS};

use crate::net::{self, Conn};
use crate::{auto, store, util};

#[derive(Clone)]
struct Session {
    conn: Conn,
    id: String,
    token: String,
}

fn session() -> Session {
    expect_context::<Session>()
}

/// Reads the current poll view, or `default` while still connecting.
fn read<T>(conn: Conn, default: T, f: impl FnOnce(&PollView) -> T) -> T {
    conn.state.with(|s| s.as_ref().map(f).unwrap_or(default))
}

#[component]
pub fn PollPage(id: String, token: String) -> impl IntoView {
    let conn = Conn::new();
    provide_context(Session {
        conn,
        id: id.clone(),
        token: token.clone(),
    });
    net::connect(id, token.clone(), conn);
    Effect::new(move |_| {
        conn.state.with(|s| {
            if let Some(view) = s {
                auto::run(view, &token);
            }
        })
    });

    let phase = Memo::new(move |_| read(conn, None, |v| Some(v.phase.clone())));

    view! {
        <main class="page">
            <Header />
            <ErrorBanner />
            // If the poll vanishes after we've seen it, keep showing what we
            // have (results already verified here) with a notice on top.
            {move || {
                (conn.gone.get() && phase.get().is_some()).then(|| view! {
                    <p class="banner">
                        "The server no longer has this poll (it restarted or was updated), so this page can't change anymore. What you see is the last state."
                    </p>
                })
            }}
            <Show
                when=move || !conn.gone.get() || phase.get().is_some()
                fallback=|| view! { <PollGone /> }
            >
                <Show
                    when=move || phase.get().is_some()
                    fallback=|| view! { <p class="muted center">"Connecting…"</p> }
                >
                    <Steps />
                    <div class="layout">
                        <section class="stack">
                            {move || match phase.get() {
                                Some(Phase::Inviting) => view! { <Inviting /> }.into_any(),
                                Some(Phase::Gathering) => view! { <Gathering /> }.into_any(),
                                Some(Phase::KeyCeremony) => view! { <KeyCeremony /> }.into_any(),
                                Some(Phase::Voting) => view! { <Voting /> }.into_any(),
                                Some(Phase::Tallying) => view! { <Tallying /> }.into_any(),
                                Some(Phase::Done) => view! { <Results /> }.into_any(),
                                Some(Phase::Failed { reason }) => {
                                    view! { <div class="card error">{reason}</div> }.into_any()
                                }
                                None => ().into_any(),
                            }}
                        </section>
                        <Participants />
                    </div>
                </Show>
            </Show>
        </main>
    }
}

#[component]
fn PollGone() -> impl IntoView {
    view! {
        <div class="card stack">
            <h2>"This poll no longer exists"</h2>
            <p>
                "The server doesn't know this poll or invite link. Polls are only kept in memory, so they are lost whenever the server restarts or is updated."
            </p>
            <p><a class="button-link" href="/">"Start a new poll"</a></p>
        </div>
    }
}

#[component]
fn Header() -> impl IntoView {
    let conn = session().conn;
    let title = move || read(conn, String::new(), |v| v.title.clone());
    let me = move || read(conn, String::new(), |v| v.participants[v.me].name.clone());
    view! {
        <header class="poll-header">
            <div>
                <a class="brand" href="/">"whisperpoll"</a>
                <h1>{title}</h1>
            </div>
            <div class="whoami">
                <span class="dot" class:online=move || conn.connected.get()></span>
                {move || {
                    let name = me();
                    if conn.gone.get() {
                        "Offline".to_string()
                    } else if name.is_empty() {
                        "Connecting".to_string()
                    } else {
                        format!("You're {name}")
                    }
                }}
            </div>
        </header>
    }
}

#[component]
fn ErrorBanner() -> impl IntoView {
    let conn = session().conn;
    move || {
        conn.error.get().map(|e| {
            view! {
                <div class="banner" role="alert">
                    <span>{e}</span>
                    <button class="link" on:click=move |_| conn.error.set(None)>
                        "Dismiss"
                    </button>
                </div>
            }
        })
    }
}

#[component]
fn Steps() -> impl IntoView {
    let conn = session().conn;
    let current = move || {
        read(conn, 0, |v| match v.phase {
            Phase::Inviting => 0,
            Phase::Gathering => 1,
            // Creating the key is part of opening the vote, not a step of its own.
            Phase::KeyCeremony | Phase::Voting => 2,
            Phase::Tallying | Phase::Done | Phase::Failed { .. } => 3,
        })
    };
    let steps = ["Invite", "Times", "Vote", "Result"];
    view! {
        <ol class="steps">
            {steps
                .into_iter()
                .enumerate()
                .map(|(i, label)| {
                    view! {
                        <li class:done=move || i < current() class:current=move || i == current()>
                            <span class="num">{i + 1}</span>
                            {label}
                        </li>
                    }
                })
                .collect_view()}
        </ol>
    }
}

#[component]
fn Participants() -> impl IntoView {
    let conn = session().conn;
    let rows = move || {
        read(conn, Vec::new(), |v| {
            let trustee = |i: usize| {
                v.ceremony
                    .as_ref()
                    .and_then(|c| c.trustees.iter().position(|&p| p == i))
            };
            v.participants
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let status = match (&v.phase, trustee(i)) {
                        (Phase::Inviting | Phase::Gathering, _) if p.registration.is_some() => {
                            "joined"
                        }
                        (Phase::Inviting | Phase::Gathering, _) => "not joined yet",
                        (Phase::KeyCeremony, Some(j)) => {
                            let c = v.ceremony.as_ref().unwrap();
                            let dealt = c.dealings.iter().any(|d| d.dealer as usize == j + 1);
                            let all_dealt = c.dealings.len() == c.trustees.len();
                            if c.checked[j] {
                                "ready"
                            } else if dealt && !all_dealt {
                                "waiting for others"
                            } else if p.online {
                                "setting up…"
                            } else {
                                "needs to open their link"
                            }
                        }
                        (Phase::Voting, _) if p.voted => "voted",
                        (Phase::Voting, _) if p.has_ballot => "deciding to keep or update",
                        (Phase::Voting, _) => "not voted",
                        (Phase::Tallying | Phase::Done, Some(j)) => {
                            let partials = v.rounds.last().map(|r| &r.partials[..]).unwrap_or(&[]);
                            if partials.iter().any(|q| q.trustee as usize == j + 1) {
                                "decrypted"
                            } else if v.phase == Phase::Tallying {
                                "decrypting…"
                            } else {
                                ""
                            }
                        }
                        (_, None) if !matches!(v.phase, Phase::Inviting | Phase::Gathering) => {
                            "not a trustee"
                        }
                        _ => "",
                    };
                    (p.name.clone(), p.creator, p.online, i == v.me, status)
                })
                .collect::<Vec<_>>()
        })
    };
    view! {
        <aside class="card people">
            <h2>"People"</h2>
            <ul>
                {move || {
                    rows()
                        .into_iter()
                        .map(|(name, creator, online, me, status)| {
                            view! {
                                <li>
                                    <span class="dot" class:online=online></span>
                                    <span class="name">
                                        {name} {me.then_some(" (you)")}
                                        {creator.then(|| view! { <span class="tag">"creator"</span> })}
                                    </span>
                                    <span class="status">{status}</span>
                                </li>
                            }
                        })
                        .collect_view()
                }}
            </ul>
        </aside>
    }
}

/// Durations offered when suggesting a slot, in minutes.
const DURATIONS: [u32; 10] = [30, 60, 90, 120, 150, 180, 240, 300, 360, 480];

#[component]
fn Gathering() -> impl IntoView {
    let conn = session().conn;
    let date = RwSignal::new(String::new());
    // Time and duration stay as they are after adding a slot (and are
    // remembered for next time), so several days only need their dates.
    let (last_time, last_minutes) = store::slot_defaults();
    let time = RwSignal::new(last_time);
    let minutes = RwSignal::new(last_minutes);
    let suggest = move |ev: SubmitEvent| {
        ev.prevent_default();
        let (d, t, m) = (date.get(), time.get(), minutes.get());
        let offset = util::local_offset_minutes(&d, &t);
        match Slot::new(&d, &t, offset, m) {
            Some(slot) => {
                net::send(&ClientMsg::SuggestSlot {
                    slot: slot.to_string(),
                });
                store::save_slot_defaults(&t, m);
                date.set(String::new());
            }
            None => conn
                .error
                .set(Some("Pick a date, a start time and a duration.".into())),
        }
    };
    let dates = move || read(conn, Vec::new(), |v| v.slots.clone());
    let is_creator = move || read(conn, false, PollView::is_creator);
    let creator_name = move || {
        read(conn, String::new(), |v| {
            v.participants
                .iter()
                .find(|p| p.creator)
                .map(|p| p.name.clone())
                .unwrap_or_default()
        })
    };

    view! {
        <div class="card stack">
            <h2>"Suggest times"</h2>
            <p class="muted">"Anyone can add a time. The list is locked when voting opens."</p>
            <ul class="dates">
                {move || {
                    let ds = dates();
                    if ds.is_empty() {
                        view! { <li class="muted">"No times yet."</li> }.into_any()
                    } else {
                        ds.into_iter()
                            .map(|d| {
                                view! {
                                    <li>
                                        <span class="date">{util::pretty_slot(&d.slot)}</span>
                                        <span class="muted">{format!("suggested by {}", d.by)}</span>
                                    </li>
                                }
                            })
                            .collect_view()
                            .into_any()
                    }
                }}
            </ul>
            <form class="row slot-form" on:submit=suggest>
                <input
                    type="date"
                    aria-label="Date"
                    prop:value=move || date.get()
                    on:input=move |ev| date.set(event_target_value(&ev))
                />
                <input
                    type="time"
                    aria-label="Start time"
                    prop:value=move || time.get()
                    on:input=move |ev| time.set(event_target_value(&ev))
                />
                <select
                    aria-label="Duration"
                    on:change=move |ev| minutes.set(event_target_value(&ev).parse().unwrap_or(120))
                >
                    {DURATIONS
                        .iter()
                        .map(|&m| {
                            view! {
                                <option value=m.to_string() prop:selected=move || minutes.get() == m>
                                    {util::duration(m)}
                                </option>
                            }
                        })
                        .collect_view()}
                </select>
                <button type="submit" class="secondary">"Add time"</button>
            </form>
        </div>
        <p class="hint">
            "Your part of the poll's key is kept in this browser. Opening your link in another browser moves it there, which is fine until voting opens. After that, this is the browser that can help reveal the result."
        </p>
        <Show
            when=is_creator
            fallback=move || {
                view! {
                    <div class="card muted">
                        {move || format!(
                            "Waiting for {} to lock the times and open voting.",
                            creator_name(),
                        )}
                    </div>
                }
            }
        >
            <StartCeremony />
            // Someone usually gets forgotten.
            <details class="invite-more">
                <summary>"Invite more people"</summary>
                <Invites />
            </details>
        </Show>
    }
}

/// The first step: the creator invites people, then moves everyone on to
/// suggesting times. Invitees who open their link meanwhile join (their
/// browser registers its key) and wait.
#[component]
fn Inviting() -> impl IntoView {
    let conn = session().conn;
    let is_creator = move || read(conn, false, PollView::is_creator);
    let creator_name = move || {
        read(conn, String::new(), |v| {
            v.participants
                .iter()
                .find(|p| p.creator)
                .map(|p| p.name.clone())
                .unwrap_or_default()
        })
    };
    let joined = move || {
        read(conn, 0, |v| {
            v.participants
                .iter()
                .filter(|p| !p.creator && p.registration.is_some())
                .count()
        })
    };
    view! {
        <Show
            when=is_creator
            fallback=move || {
                view! {
                    <div class="card stack">
                        <h2>{move || format!("{} is inviting people", creator_name())}</h2>
                        <p class="muted">"Next, everyone suggests times that could work. This page moves on by itself."</p>
                    </div>
                }
            }
        >
            <Invites />
            <div class="card stack">
                <h2>"Done inviting?"</h2>
                <p class="muted">
                    {move || {
                        let n = joined();
                        let opened = match n {
                            0 => "Nobody has opened their link yet".to_string(),
                            1 => "1 person has opened their link".to_string(),
                            n => format!("{n} people have opened their links"),
                        };
                        format!("{opened}. Next, everyone suggests times that could work; you can still invite people then.")
                    }}
                </p>
                <button on:click=move |_| net::send(&ClientMsg::FinishInviting)>
                    "Next: suggest times"
                </button>
            </div>
        </Show>
        <p class="hint">
            "Your part of the poll's key is kept in this browser. Opening your link in another browser moves it there, which is fine until voting opens. After that, this is the browser that can help reveal the result."
        </p>
    }
}

fn select_all(ev: leptos::ev::FocusEvent) {
    event_target::<HtmlInputElement>(&ev).select();
}

#[component]
fn Invites() -> impl IntoView {
    let s = session();
    let conn = s.conn;
    let name = RwSignal::new(String::new());
    let invite = move |ev: SubmitEvent| {
        ev.prevent_default();
        let n = name.get();
        if !n.trim().is_empty() {
            net::send(&ClientMsg::Invite { name: n });
            name.set(String::new());
        }
    };
    let links = move || {
        read(conn, Vec::new(), |v| {
            v.invites
                .iter()
                .map(|i| (i.name.clone(), util::invite_url(&v.id, &i.token)))
                .collect::<Vec<_>>()
        })
    };
    view! {
        <div class="card stack">
            <h2>"Invite people"</h2>
            <p class="muted">"Each person gets their own link. Send it to them privately."</p>
            <form class="row" on:submit=invite>
                <input
                    placeholder="Their name"
                    prop:value=move || name.get()
                    on:input=move |ev| name.set(event_target_value(&ev))
                />
                <button type="submit" class="secondary">"Create link"</button>
            </form>
            <ul class="invites">
                {move || {
                    links()
                        .into_iter()
                        .map(|(name, url)| {
                            let copy = url.clone();
                            view! {
                                <li>
                                    <span class="name">{name}</span>
                                    <input
                                        readonly
                                        prop:value=url
                                        on:focus=select_all
                                    />
                                    <button
                                        class="secondary small"
                                        on:click=move |_| util::copy_to_clipboard(&copy)
                                    >
                                        "Copy"
                                    </button>
                                </li>
                            }
                        })
                        .collect_view()
                }}
            </ul>
        </div>
    }
}

#[component]
fn StartCeremony() -> impl IntoView {
    let conn = session().conn;
    let registered = move || {
        read(conn, 0, |v| {
            v.participants
                .iter()
                .filter(|p| p.registration.is_some())
                .count()
        })
    };
    let has_dates = move || read(conn, false, |v| !v.slots.is_empty());
    let date_count = move || read(conn, 0, |v| v.slots.len());
    let days = RwSignal::new(1usize);
    let chosen = RwSignal::new(0usize);
    let threshold = move || {
        let n = registered();
        match chosen.get() {
            t if (1..=n).contains(&t) => t,
            _ => n / 2 + 1,
        }
    };
    let hint = move || {
        let (n, t) = (registered(), threshold());
        if n <= 1 {
            "Only you have joined so far, so you'd hold the whole key. Wait for others to open their links.".to_string()
        } else if t == n {
            format!("All {n} must come back to reveal the result. Nobody short of everyone could read a ballot.")
        } else {
            format!(
                "Any {t} of {n} can reveal the result, so it survives {} no-show{}. But {t} working together could also read individual ballots.",
                n - t,
                if n - t == 1 { "" } else { "s" },
            )
        }
    };
    let start = move |_| {
        net::send(&ClientMsg::StartCeremony {
            threshold: threshold(),
            days: days.get().clamp(1, date_count().max(1)),
        })
    };

    view! {
        <div class="card stack">
            <h2>"Lock times & open voting"</h2>
            <p class="muted">
                {move || format!(
                    "Everyone who has opened their link ({} so far) becomes a trustee: their browsers create the poll's key together, so no single person can unlock a ballot. That needs their pages open for a moment after you lock the times.",
                    registered(),
                )}
            </p>
            <label class="field inline">
                <span>"Trustees needed to reveal the result"</span>
                <select on:change=move |ev| chosen.set(event_target_value(&ev).parse().unwrap_or(0))>
                    {move || {
                        let n = registered().max(1);
                        (1..=n)
                            .map(|t| {
                                view! {
                                    <option value=t.to_string() prop:selected=move || threshold() == t>
                                        {format!("{t} of {n}")}
                                    </option>
                                }
                            })
                            .collect_view()
                    }}
                </select>
            </label>
            <p class="hint">{hint}</p>
            <label class="field inline">
                <span>"Meetings to pick"</span>
                <select on:change=move |ev| days.set(event_target_value(&ev).parse().unwrap_or(1))>
                    {move || {
                        (1..=date_count().max(1))
                            .map(|d| {
                                view! {
                                    <option value=d.to_string() prop:selected=move || days.get() == d>
                                        {if d == 1 { "1 meeting".to_string() } else { format!("{d} meetings") }}
                                    </option>
                                }
                            })
                            .collect_view()
                    }}
                </select>
            </label>
            <p class="hint">
                {move || if days.get() > 1 {
                    "Meetings are picked one at a time. After each pick, everyone can update their vote for the next one, or keep it."
                } else {
                    "Need more than one meeting? They're then picked one at a time."
                }}
            </p>
            <button on:click=start disabled=move || !has_dates() || registered() == 0>
                "Lock times & open voting"
            </button>
        </div>
    }
}

/// Creating the key normally takes a split second, so this only appears if
/// it's slow — which means a trustee's page isn't open — and says who
/// everyone is waiting for.
#[component]
fn KeyCeremony() -> impl IntoView {
    let s = session();
    let conn = s.conn;
    let (id, token) = (s.id.clone(), s.token.clone());
    let slow = RwSignal::new(false);
    set_timeout(
        move || {
            slow.try_set(true);
        },
        Duration::from_millis(1500),
    );
    let waiting = move || {
        read(conn, Vec::new(), |v| {
            let c = v.ceremony.as_ref().unwrap();
            let all_dealt = c.dealings.len() == c.trustees.len();
            c.trustees
                .iter()
                .enumerate()
                .filter(|&(j, _)| {
                    if all_dealt {
                        !c.checked[j]
                    } else {
                        !c.dealings.iter().any(|d| d.dealer as usize == j + 1)
                    }
                })
                .map(|(_, &p)| {
                    let who = &v.participants[p];
                    (who.name.clone(), who.online, p == v.me)
                })
                .collect::<Vec<_>>()
        })
    };
    let am_trustee = Memo::new(move |_| {
        read(conn, false, |v| {
            v.ceremony()
                .is_some_and(|c| c.index_of(&store::identity(&id, &token)).is_some())
        })
    });
    move || {
        slow.get().then(|| {
            view! {
                <div class="card stack">
                    <h2>"Preparing the ballot box"</h2>
                    <p>
                        "Before anyone votes, everyone's browsers create the poll's key together, so no single person can unlock a ballot. "
                        "It takes a moment once everyone's page is open."
                    </p>
                    <ul class="waiting">
                        {move || {
                            waiting()
                                .into_iter()
                                .map(|(name, online, me)| {
                                    let status = match (me, online) {
                                        (true, _) => "you — keep this tab open",
                                        (false, true) => "page open, setting up…",
                                        (false, false) => "needs to open their link",
                                    };
                                    view! {
                                        <li>
                                            <span class="dot" class:online=online></span>
                                            <strong>{name}</strong>
                                            <span class="muted">{status}</span>
                                        </li>
                                    }
                                })
                                .collect_view()
                        }}
                    </ul>
                    <p class="hint">
                        {move || if am_trustee.get() {
                            "Voting opens by itself as soon as everyone's browser has done its part."
                        } else {
                            "Adding this browser as a trustee…"
                        }}
                    </p>
                </div>
            }
        })
    }
}

/// Shown after voting opens if this participant's trustee key is in another
/// browser: that browser has to be opened to help reveal the result.
#[component]
fn KeyElsewhere() -> impl IntoView {
    let s = session();
    let conn = s.conn;
    let ids = StoredValue::new((s.id, s.token));
    let elsewhere = Memo::new(move |_| {
        read(conn, false, |v| {
            let is_trustee = v
                .ceremony
                .as_ref()
                .is_some_and(|c| c.trustees.contains(&v.me));
            let mine = ids.with_value(|(id, token)| store::identity(id, token).public_bytes());
            is_trustee
                && v.participants[v.me]
                    .registration
                    .as_ref()
                    .is_some_and(|r| r.key_bytes() != mine)
        })
    });
    move || {
        elsewhere.get().then(|| {
            view! {
                <p class="hint">
                    "Your part of the poll's key is in the browser you set this poll up in. You can vote from here, but to help reveal the result, open your link in that browser."
                </p>
            }
        })
    }
}

fn day_list(options: &[String], picked: &[usize]) -> String {
    picked
        .iter()
        .map(|&o| util::pretty_slot(&options[o]))
        .collect::<Vec<_>>()
        .join(", ")
}

#[component]
fn Voting() -> impl IntoView {
    let s = session();
    let conn = s.conn;
    let ids = StoredValue::new((s.id.clone(), s.token.clone()));
    let (options, round, days, picked) = conn.state.with_untracked(|st| {
        st.as_ref()
            .map(|v| (v.options.clone(), v.rounds.len(), v.days, v.picked()))
            .unwrap_or_default()
    });

    // Later rounds start from your last vote; picked times can't win again.
    let remembered = ids
        .with_value(|(id, token)| store::choices(id, token))
        .filter(|c| round > 1 && c.len() == options.len())
        .unwrap_or_else(|| vec![None; options.len()]);
    let initial: Vec<Option<usize>> = remembered
        .into_iter()
        .enumerate()
        .map(|(i, c)| if picked.contains(&i) { None } else { c })
        .collect();
    let choices = RwSignal::new(initial);
    let editing = RwSignal::new(false);
    let mine = Memo::new(move |_| {
        read(conn, (false, false), |v| {
            let p = &v.participants[v.me];
            (p.has_ballot, p.voted)
        })
    });
    let progress = move || {
        read(conn, (0, 0), |v| {
            (
                v.participants.iter().filter(|p| p.voted).count(),
                v.participants.len(),
            )
        })
    };
    let is_creator = move || read(conn, false, PollView::is_creator);
    let receipt = move || {
        // Re-read on every state change so it appears right after voting.
        conn.state.track();
        ids.with_value(|(id, token)| store::receipt_for(id, token, round))
            .map(|r| util::to_hex(&r.0)[..16].to_string())
    };

    let submit = move |_| {
        let Some(poll) = conn
            .state
            .with_untracked(|st| st.as_ref().and_then(|v| v.poll()))
        else {
            conn.error.set(Some("the poll key isn't ready yet".into()));
            return;
        };
        let picks = choices.get_untracked();
        match cast_tiered_ballot(&poll, &picks, &mut OsRng) {
            Ok(ballot) => {
                ids.with_value(|(id, token)| {
                    store::save_receipt(id, token, round, &ballot.receipt());
                    store::save_choices(id, token, &picks);
                });
                net::send(&ClientMsg::Vote(ballot));
                editing.set(false);
            }
            Err(e) => conn.error.set(Some(e.to_string())),
        }
    };
    let keep = move |_| net::send(&ClientMsg::KeepVote);

    let taken = day_list(&options, &picked);
    let heading = if days > 1 {
        format!("Meeting {round} of {days}: how well does each time work for you?")
    } else {
        "How well does each time work for you?".to_string()
    };

    let form = {
        let (options, picked) = (options.clone(), picked.clone());
        move || {
            let rows = options
                .iter()
                .enumerate()
                .map(|(i, date)| {
                    let control = if let Some(k) = picked.iter().position(|&p| p == i) {
                        view! { <span class="muted">{format!("Picked for meeting {}", k + 1)}</span> }
                            .into_any()
                    } else {
                        let buttons = (0..=TIERS.len())
                            .map(|t| {
                                let value = (t < TIERS.len()).then_some(t);
                                let label = TIERS.get(t).copied().unwrap_or("Can't");
                                view! {
                                    <button
                                        type="button"
                                        class:on=move || choices.with(|c| c[i] == value)
                                        class:cant=value.is_none()
                                        on:click=move |_| choices.update(|c| c[i] = value)
                                    >
                                        {label}
                                    </button>
                                }
                            })
                            .collect_view();
                        view! { <div class="seg">{buttons}</div> }.into_any()
                    };
                    view! {
                        <div class="ballot-row" class:taken=picked.contains(&i)>
                            <span class="date">{util::pretty_slot(date)}</span>
                            {control}
                        </div>
                    }
                })
                .collect_view();
            view! {
                <div class="card stack">
                    <h2>{heading.clone()}</h2>
                    <p class="muted">
                        "Your choices are encrypted here, with proofs that they're well-formed. Nobody — not the server, not any single trustee — can read them."
                    </p>
                    <div class="ballot">{rows}</div>
                    <div class="row">
                        <button on:click=submit>"Cast encrypted vote"</button>
                        <Show when=move || editing.get()>
                            <button class="secondary" on:click=move |_| editing.set(false)>
                                "Cancel"
                            </button>
                        </Show>
                    </div>
                </div>
            }
        }
    };

    let body = move || {
        let (has_ballot, voted) = mine.get();
        if editing.get() || (!has_ballot && !voted) {
            form().into_any()
        } else if voted {
            view! {
                <div class="card stack">
                    <h2>
                        {if days > 1 {
                            format!("You're set for meeting {round}")
                        } else {
                            "Your vote is in".to_string()
                        }}
                    </h2>
                    <p>
                        "It was encrypted in this browser before it was sent. Your receipt: "
                        <code>{receipt}</code>
                    </p>
                    <p class="muted">"You can still change it until voting closes."</p>
                    <div class="row">
                        <button class="secondary" on:click=move |_| editing.set(true)>
                            "Change my vote"
                        </button>
                    </div>
                </div>
            }
            .into_any()
        } else {
            view! {
                <div class="card stack">
                    <h2>{format!("Meeting {round} of {days}")}</h2>
                    <p>
                        {format!(
                            "Your vote from the last round still counts. Keep it, or update it now that {} is taken.",
                            taken,
                        )}
                    </p>
                    <div class="row">
                        <button on:click=keep>"Keep my votes"</button>
                        <button class="secondary" on:click=move |_| editing.set(true)>
                            "Update my votes"
                        </button>
                    </div>
                </div>
            }
            .into_any()
        }
    };

    view! {
        <PickedSoFar />
        <KeyElsewhere />
        {body}
        <div class="card stack">
            {move || {
                let (k, n) = progress();
                view! {
                    <div class="meter-row">
                        <span>"Ready"</span>
                        <progress max=n value=k></progress>
                        <span class="muted">{format!("{k}/{n}")}</span>
                    </div>
                }
            }}
            <p class="muted">
                {if round > 1 {
                    "Voting closes by itself once everyone has kept or updated their vote."
                } else {
                    "Voting closes by itself once everyone has voted."
                }}
            </p>
            <Show when=is_creator>
                <button class="secondary" on:click=move |_| net::send(&ClientMsg::CloseVoting)>
                    "Close voting now"
                </button>
            </Show>
        </div>
    }
}

#[component]
fn Tallying() -> impl IntoView {
    let conn = session().conn;
    let progress = move || {
        read(conn, (0, 0, 0, 0), |v| {
            (
                v.rounds.last().map_or(0, |r| r.partials.len()),
                v.ceremony.as_ref().map_or(0, |c| c.threshold),
                v.rounds.len(),
                v.days,
            )
        })
    };
    view! {
        <PickedSoFar />
        <KeyElsewhere />
        <div class="card stack">
            <h2>
                {move || {
                    let (_, _, round, days) = progress();
                    if days > 1 {
                        format!("Revealing meeting {round} of {days}")
                    } else {
                        "Revealing the result".to_string()
                    }
                }}
            </h2>
            <p>
                "Trustees' browsers each decrypt a share of the total — never an individual ballot — and prove they did it honestly."
            </p>
            {move || {
                let (k, t, _, _) = progress();
                view! {
                    <div class="meter-row">
                        <span>"Trustees"</span>
                        <progress max=t value=k></progress>
                        <span class="muted">{format!("{k} of {t} needed")}</span>
                    </div>
                }
            }}
        </div>
    }
}

/// Outcomes of every finished round, recomputed when the state changes.
fn finished_rounds() -> Memo<Vec<Outcome>> {
    let s = session();
    let conn = s.conn;
    let ids = StoredValue::new((s.id, s.token));
    Memo::new(move |_| {
        read(conn, Vec::new(), |v| {
            ids.with_value(|(id, token)| finished_outcomes(v, |r| store::receipt_for(id, token, r)))
        })
    })
}

#[component]
fn RoundDetails(outcome: Outcome, round: usize, days: usize, open: bool) -> impl IntoView {
    let o = outcome;
    let title = match (&o.winner, days) {
        (Some(pick), 1) => format!("Result: {}", util::pretty_slot(&pick.slot)),
        (Some(pick), _) => format!("Meeting {round}: {}", util::pretty_slot(&pick.slot)),
        (None, _) => format!("Meeting {round}: no time found"),
    };
    let checks_out = o.checks_out();
    view! {
        <details class="card round" open=open>
            <summary>
                <span>{title}</span>
                <span class=if checks_out { "ok" } else { "error" }>
                    {if checks_out { "✓ verified" } else { "✗ check failed" }}
                </span>
            </summary>
            <div class="stack">
                <table class="results">
                    <thead>
                        <tr>
                            <th></th>
                            {TIERS.iter().map(|t| view! { <th>{*t}</th> }).collect_view()}
                            <th>"Can come"</th>
                            <th>"Score"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {o.rows.iter().map(|row| view! {
                            <tr class:earlier=row.picked_earlier>
                                <th>
                                    {util::pretty_slot(&row.slot)}
                                    {row.picked_earlier.then_some(view! { <span class="tag">"picked"</span> })}
                                </th>
                                {row.counts.iter().map(|c| view! { <td>{*c}</td> }).collect_view()}
                                <td>{row.available}</td>
                                <td>{row.score}</td>
                            </tr>
                        }).collect_view()}
                    </tbody>
                </table>
                <p class="muted">
                    "Times are ranked by how many can come, then by preference (3 / 2 / 1 points)."
                    {(round > 1).then_some(" Times picked in earlier rounds can't win again.")}
                </p>
                {match &o.verification_error {
                    None => view! {
                        <p class="ok">{format!(
                            "✓ Checked in your browser: all {} ballots are valid, and the totals are exactly what the decryption proofs from {} show.",
                            o.ballots,
                            o.decrypted_by.join(" and "),
                        )}</p>
                    }.into_any(),
                    Some(e) => view! { <p class="error">{format!("✗ Verification failed: {e}")}</p> }.into_any(),
                }}
                {(!o.winner_checks_out).then(|| view! {
                    <p class="error">"✗ The server's pick for this round doesn't match these totals!"</p>
                })}
                {match o.mine {
                    Some(true) => view! { <p class="ok">"✓ Your ballot is on the board and was counted."</p> }.into_any(),
                    Some(false) => view! { <p class="error">"✗ Your ballot is missing from the board!"</p> }.into_any(),
                    None => view! { <p class="muted">"You didn't vote from this browser."</p> }.into_any(),
                }}
            </div>
        </details>
    }
}

/// While a later round is running: the times picked so far, with their proofs.
#[component]
fn PickedSoFar() -> impl IntoView {
    let conn = session().conn;
    let outcomes = finished_rounds();
    let days = move || read(conn, 1, |v| v.days);
    move || {
        let days = days();
        outcomes
            .get()
            .into_iter()
            .enumerate()
            .map(|(i, o)| view! { <RoundDetails outcome=o round=i + 1 days=days open=false /> })
            .collect_view()
    }
}

/// "Add to calendar" for the picked times: one `.ics` with all of them, plus a
/// Google Calendar link per time. Built entirely in the browser.
#[component]
fn CalendarExport(outcomes: Vec<Outcome>) -> impl IntoView {
    let conn = session().conn;
    let (title, id) = conn.state.with_untracked(|st| {
        st.as_ref()
            .map(|v| (v.title.clone(), v.id.clone()))
            .unwrap_or_default()
    });
    let picks: Vec<(Slot, String)> = outcomes
        .iter()
        .filter_map(|o| {
            let pick = o.winner.as_ref()?;
            let note = format!(
                "{} of {} can make it. Picked with whisperpoll.",
                pick.available, o.ballots
            );
            Some((Slot::parse(&pick.slot)?, note))
        })
        .collect();
    if picks.is_empty() {
        return ().into_any();
    }
    let all = picks.clone();
    let label = if all.len() == 1 {
        "Add to calendar (.ics)".to_string()
    } else {
        format!("Add all {} to calendar (.ics)", all.len())
    };
    let download = {
        let (title, id) = (title.clone(), id.clone());
        move |_| {
            let events: Vec<Event> = all
                .iter()
                .map(|(slot, note)| Event {
                    slot: *slot,
                    description: note.clone(),
                })
                .collect();
            let ics = calendar::ics(&title, &id, &util::utc_stamp(), &events);
            let name = format!("{}.ics", util::file_slug(&title));
            util::download(&name, "text/calendar;charset=utf-8", &ics);
        }
    };
    let google = picks
        .iter()
        .map(|(slot, note)| {
            let event = Event {
                slot: *slot,
                description: note.clone(),
            };
            let href = calendar::google_link(&title, &event);
            let label = if picks.len() == 1 {
                "Google Calendar".to_string()
            } else {
                format!("Google: {}", util::pretty_slot(&slot.to_string()))
            };
            view! {
                <a class="button-link" href=href target="_blank" rel="noopener noreferrer">
                    {label}
                </a>
            }
        })
        .collect_view();
    view! {
        <div class="row calendar">
            <button class="secondary small" on:click=download>
                {label}
            </button>
            {google}
        </div>
    }
    .into_any()
}

#[component]
fn Results() -> impl IntoView {
    let conn = session().conn;
    let outcomes = finished_rounds();
    let days = move || read(conn, 1, |v| v.days);

    move || {
        let (outcomes, days) = (outcomes.get(), days());
        if outcomes.is_empty() {
            return view! { <div class="card error">"Couldn't rebuild the result from the board."</div> }
                .into_any();
        }
        let summary = if days == 1 {
            match &outcomes[0].winner {
                Some(pick) => view! {
                    <div class="card winner">
                        <span class="muted">"Best time"</span>
                        <h2>{util::pretty_slot(&pick.slot)}</h2>
                        <p>{format!("{} of {} can make it", pick.available, outcomes[0].ballots)}</p>
                        <CalendarExport outcomes=outcomes.clone() />
                    </div>
                }
                .into_any(),
                None => view! {
                    <div class="card winner">
                        <h2>"No time works"</h2>
                        <p>"Nobody can make any of the suggested times."</p>
                    </div>
                }
                .into_any(),
            }
        } else {
            let found = outcomes.iter().filter(|o| o.winner.is_some()).count();
            view! {
                <div class="card winner">
                    <span class="muted">
                        {format!("Found {found} of {days} meeting{}", if days == 1 { "" } else { "s" })}
                    </span>
                    <ol class="picked">
                        {outcomes
                            .iter()
                            .map(|o| match &o.winner {
                                Some(pick) => view! {
                                    <li>
                                        <strong>{util::pretty_slot(&pick.slot)}</strong>
                                        <span class="muted">{format!(" — {} of {} can make it", pick.available, o.ballots)}</span>
                                    </li>
                                }
                                .into_any(),
                                None => view! {
                                    <li class="muted">"No remaining time works for anyone, so the search stopped here."</li>
                                }
                                .into_any(),
                            })
                            .collect_view()}
                    </ol>
                    <CalendarExport outcomes=outcomes.clone() />
                </div>
            }
            .into_any()
        };
        let single = outcomes.len() == 1;
        view! {
            {summary}
            {outcomes
                .into_iter()
                .enumerate()
                .map(|(i, o)| view! { <RoundDetails outcome=o round=i + 1 days=days open=single /> })
                .collect_view()}
        }
        .into_any()
    }
}
