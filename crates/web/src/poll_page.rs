use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use rand_core::OsRng;
use web_sys::HtmlInputElement;
use whisperpoll_core::{cast_tiered_ballot, verify_tally, TieredResult};
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
            <Show
                when=move || phase.get().is_some()
                fallback=|| view! { <p class="muted center">"Connecting…"</p> }
            >
                <Steps />
                <div class="layout">
                    <section class="stack">
                        {move || match phase.get() {
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
        </main>
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
                    if name.is_empty() { "Connecting".to_string() } else { format!("You're {name}") }
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
            Phase::Gathering => 0,
            Phase::KeyCeremony => 1,
            Phase::Voting => 2,
            Phase::Tallying | Phase::Done | Phase::Failed { .. } => 3,
        })
    };
    let steps = ["Invite & dates", "Shared key", "Vote", "Result"];
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
                        (Phase::Gathering, _) if p.registration.is_some() => "has a key",
                        (Phase::Gathering, _) => "not joined yet",
                        (Phase::KeyCeremony, Some(j)) => {
                            let c = v.ceremony.as_ref().unwrap();
                            if c.checked[j] {
                                "key ready"
                            } else if c.dealings.iter().any(|d| d.dealer as usize == j + 1) {
                                "shares sent"
                            } else {
                                "creating key…"
                            }
                        }
                        (Phase::Voting, _) if p.voted => "voted",
                        (Phase::Voting, _) => "not voted",
                        (Phase::Tallying | Phase::Done, Some(j)) => {
                            if v.partials.iter().any(|q| q.trustee as usize == j + 1) {
                                "decrypted"
                            } else if v.phase == Phase::Tallying {
                                "decrypting…"
                            } else {
                                ""
                            }
                        }
                        (_, None) if !matches!(v.phase, Phase::Gathering) => "not a trustee",
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

#[component]
fn Gathering() -> impl IntoView {
    let conn = session().conn;
    let date = RwSignal::new(String::new());
    let suggest = move |ev: SubmitEvent| {
        ev.prevent_default();
        let d = date.get();
        if !d.is_empty() {
            net::send(&ClientMsg::SuggestDate { date: d });
            date.set(String::new());
        }
    };
    let dates = move || read(conn, Vec::new(), |v| v.dates.clone());
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
            <h2>"Suggest dates"</h2>
            <p class="muted">"Anyone can add a day. The list is locked when the key is created."</p>
            <ul class="dates">
                {move || {
                    let ds = dates();
                    if ds.is_empty() {
                        view! { <li class="muted">"No dates yet."</li> }.into_any()
                    } else {
                        ds.into_iter()
                            .map(|d| {
                                view! {
                                    <li>
                                        <span class="date">{util::pretty_date(&d.date)}</span>
                                        <span class="muted">{format!("suggested by {}", d.by)}</span>
                                    </li>
                                }
                            })
                            .collect_view()
                            .into_any()
                    }
                }}
            </ul>
            <form class="row" on:submit=suggest>
                <input
                    type="date"
                    prop:value=move || date.get()
                    on:input=move |ev| date.set(event_target_value(&ev))
                />
                <button type="submit" class="secondary">"Add date"</button>
            </form>
        </div>
        <Show
            when=is_creator
            fallback=move || {
                view! {
                    <div class="card muted">
                        {move || format!(
                            "Waiting for {} to lock the dates and create the shared key.",
                            creator_name(),
                        )}
                    </div>
                }
            }
        >
            <Invites />
            <StartCeremony />
        </Show>
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
    let has_dates = move || read(conn, false, |v| !v.dates.is_empty());
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
        })
    };

    view! {
        <div class="card stack">
            <h2>"Lock dates & create the shared key"</h2>
            <p class="muted">
                {move || format!(
                    "Everyone who has opened their link ({} so far) becomes a trustee. Their browsers create a key together that none of them holds alone.",
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
            <button on:click=start disabled=move || !has_dates() || registered() == 0>
                "Lock dates & create key"
            </button>
        </div>
    }
}

#[component]
fn KeyCeremony() -> impl IntoView {
    let s = session();
    let conn = s.conn;
    let (id, token) = (s.id.clone(), s.token.clone());
    let progress = move || {
        read(conn, (0, 0, 0, 0), |v| {
            let c = v.ceremony.as_ref().unwrap();
            let n = c.trustees.len();
            (
                c.dealings.len(),
                c.checked.iter().filter(|&&d| d).count(),
                n,
                c.threshold,
            )
        })
    };
    let am_trustee = move || {
        read(conn, false, |v| {
            v.ceremony()
                .is_some_and(|c| c.index_of(&store::identity(&id, &token)).is_some())
        })
    };
    view! {
        <div class="card stack">
            <h2>"Creating the shared key"</h2>
            <p>
                "Each trustee's browser splits a secret of its own and sends everyone an encrypted share. "
                "The poll key is the sum of all of them — nobody ever sees it whole."
            </p>
            {move || {
                let (dealt, checked, n, t) = progress();
                view! {
                    <div class="meter-row">
                        <span>"Shares sent"</span>
                        <progress max=n value=dealt></progress>
                        <span class="muted">{format!("{dealt}/{n}")}</span>
                    </div>
                    <div class="meter-row">
                        <span>"Shares checked"</span>
                        <progress max=n value=checked></progress>
                        <span class="muted">{format!("{checked}/{n}")}</span>
                    </div>
                    <p class="muted">{format!("Any {t} of the {n} trustees will be able to reveal the result.")}</p>
                }
            }}
            <p class="hint">
                {move || if am_trustee() {
                    "Your browser does its part automatically — keep this tab open until the key is ready."
                } else {
                    "You're not a trustee (you joined after the key was started, or from another browser). You can still vote."
                }}
            </p>
        </div>
    }
}

#[component]
fn Voting() -> impl IntoView {
    let s = session();
    let conn = s.conn;
    let options = conn
        .state
        .with_untracked(|st| st.as_ref().map(|v| v.options.clone()).unwrap_or_default());
    let choices = RwSignal::new(vec![None::<usize>; options.len()]);
    let voted = move || read(conn, false, |v| v.participants[v.me].voted);
    let progress = move || {
        read(conn, (0, 0), |v| {
            (
                v.participants.iter().filter(|p| p.voted).count(),
                v.participants.len(),
            )
        })
    };
    let is_creator = move || read(conn, false, PollView::is_creator);
    let ids = StoredValue::new((s.id.clone(), s.token.clone()));
    let (id, token) = (s.id.clone(), s.token.clone());
    let receipt = {
        let (id, token) = (id.clone(), token.clone());
        move || {
            // Re-read on every state change so it appears right after voting.
            conn.state.track();
            store::receipt(&id, &token).map(|r| util::to_hex(&r.0)[..16].to_string())
        }
    };

    let submit = move |_| {
        let Some(poll) = conn
            .state
            .with_untracked(|st| st.as_ref().and_then(|v| v.poll()))
        else {
            conn.error.set(Some("the poll key isn't ready yet".into()));
            return;
        };
        match cast_tiered_ballot(&poll, &choices.get_untracked(), &mut OsRng) {
            Ok(ballot) => {
                ids.with_value(|(id, token)| store::save_receipt(id, token, &ballot.receipt()));
                net::send(&ClientMsg::Vote(ballot));
            }
            Err(e) => conn.error.set(Some(e.to_string())),
        }
    };

    let rows = options
        .iter()
        .enumerate()
        .map(|(i, date)| {
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
            view! {
                <div class="ballot-row">
                    <span class="date">{util::pretty_date(date)}</span>
                    <div class="seg">{buttons}</div>
                </div>
            }
        })
        .collect_view();

    view! {
        <Show
            when=move || !voted()
            fallback=move || {
                view! {
                    <div class="card stack">
                        <h2>"Your vote is in"</h2>
                        <p>
                            "It was encrypted in this browser before it was sent. Your receipt: "
                            <code>{receipt.clone()}</code>
                        </p>
                        <p class="muted">"Once the result is out, you'll see whether your ballot was counted."</p>
                    </div>
                }
            }
        >
            <div class="card stack">
                <h2>"How well does each day work for you?"</h2>
                <p class="muted">
                    "Your choices are encrypted here, with proofs that they're well-formed. Nobody — not the server, not any single trustee — can read them."
                </p>
                <div class="ballot">{rows.clone()}</div>
                <button on:click=submit>"Cast encrypted vote"</button>
            </div>
        </Show>
        <div class="card stack">
            {move || {
                let (k, n) = progress();
                view! {
                    <div class="meter-row">
                        <span>"Votes"</span>
                        <progress max=n value=k></progress>
                        <span class="muted">{format!("{k}/{n}")}</span>
                    </div>
                }
            }}
            <p class="muted">"Voting closes by itself once everyone has voted."</p>
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
        read(conn, (0, 0), |v| {
            (
                v.partials.len(),
                v.ceremony.as_ref().map(|c| c.threshold).unwrap_or(0),
            )
        })
    };
    view! {
        <div class="card stack">
            <h2>"Revealing the result"</h2>
            <p>
                "Trustees' browsers each decrypt a share of the total — never an individual ballot — and prove they did it honestly."
            </p>
            {move || {
                let (k, t) = progress();
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

#[derive(Clone, PartialEq)]
struct Outcome {
    rows: Vec<(String, Vec<u64>, u64, u64)>,
    ballots: u64,
    verified: Result<(), String>,
    decrypted_by: Vec<String>,
    mine: Option<bool>,
}

#[component]
fn Results() -> impl IntoView {
    let s = session();
    let conn = s.conn;
    let (id, token) = (s.id.clone(), s.token.clone());
    let outcome = Memo::new(move |_| {
        conn.state.with(|st| {
            let v = st.as_ref()?;
            let board = v.board()?;
            let tally = v.tally.as_ref()?;
            let verified = verify_tally(&board, tally).map_err(|e| e.to_string());
            let result = TieredResult::from_tally(&board.poll, tally).ok()?;
            let cv = v.ceremony.as_ref()?;
            Some(Outcome {
                rows: result
                    .order()
                    .into_iter()
                    .map(|o| {
                        (
                            v.options[o].clone(),
                            result.counts[o].clone(),
                            result.available[o],
                            result.score[o],
                        )
                    })
                    .collect(),
                ballots: tally.ballots,
                verified,
                decrypted_by: tally
                    .partials
                    .iter()
                    .map(|p| {
                        v.participants[cv.trustees[p.trustee as usize - 1]]
                            .name
                            .clone()
                    })
                    .collect(),
                mine: store::receipt(&id, &token).map(|r| board.contains(&r)),
            })
        })
    });

    move || {
        let Some(o) = outcome.get() else {
            return view! { <div class="card error">"Couldn't rebuild the result from the board."</div> }
                .into_any();
        };
        let winner = o.rows.first().cloned();
        view! {
            {winner.map(|(date, _, available, _)| view! {
                <div class="card winner">
                    <span class="muted">"Best day"</span>
                    <h2>{util::pretty_date(&date)}</h2>
                    <p>{format!("{available} of {} can make it", o.ballots)}</p>
                </div>
            })}
            <div class="card stack">
                <h2>"Results"</h2>
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
                        {o.rows.iter().map(|(date, counts, available, score)| view! {
                            <tr>
                                <th>{util::pretty_date(date)}</th>
                                {counts.iter().map(|c| view! { <td>{*c}</td> }).collect_view()}
                                <td>{*available}</td>
                                <td>{*score}</td>
                            </tr>
                        }).collect_view()}
                    </tbody>
                </table>
                <p class="muted">"Days are ranked by how many can come, then by preference (3 / 2 / 1 points)."</p>
            </div>
            <div class="card stack">
                <h2>"Verification"</h2>
                {match &o.verified {
                    Ok(()) => view! {
                        <p class="ok">{format!(
                            "✓ Checked in your browser: all {} ballots are valid, and the totals are exactly what the decryption proofs from {} show.",
                            o.ballots,
                            o.decrypted_by.join(" and "),
                        )}</p>
                    }.into_any(),
                    Err(e) => view! { <p class="error">{format!("✗ Verification failed: {e}")}</p> }.into_any(),
                }}
                {match o.mine {
                    Some(true) => view! { <p class="ok">"✓ Your ballot is on the board and was counted."</p> }.into_any(),
                    Some(false) => view! { <p class="error">"✗ Your ballot is missing from the board!"</p> }.into_any(),
                    None => view! { <p class="muted">"You didn't vote from this browser."</p> }.into_any(),
                }}
            </div>
        }
        .into_any()
    }
}
