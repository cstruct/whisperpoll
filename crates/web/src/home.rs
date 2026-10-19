use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;
use whisperpoll_protocol::{CreatePoll, Created};

async fn create_poll(req: CreatePoll) -> Result<Created, String> {
    let resp = gloo_net::http::Request::post("/api/polls")
        .json(&req)
        .map_err(|e| e.to_string())?
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.ok() {
        return Err(resp
            .text()
            .await
            .unwrap_or_else(|_| "request failed".into()));
    }
    resp.json().await.map_err(|e| e.to_string())
}

#[component]
pub fn Home() -> impl IntoView {
    let title = RwSignal::new(String::new());
    let name = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        error.set(None);
        let req = CreatePoll {
            title: title.get(),
            name: name.get(),
        };
        spawn_local(async move {
            match create_poll(req).await {
                Ok(c) => {
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
            <header class="hero">
                <h1>"whisperpoll"</h1>
                <p class="lede">
                    "Find a day that works — with votes nobody can read, not even the server, "
                    "and a result everyone can check."
                </p>
            </header>
            <form class="card stack" on:submit=submit>
                <h2>"Start a poll"</h2>
                <label class="field">
                    <span>"What are you planning?"</span>
                    <input
                        placeholder="Team dinner"
                        required
                        prop:value=move || title.get()
                        on:input=move |ev| title.set(event_target_value(&ev))
                    />
                </label>
                <label class="field">
                    <span>"Your name"</span>
                    <input
                        placeholder="Alex"
                        required
                        prop:value=move || name.get()
                        on:input=move |ev| name.set(event_target_value(&ev))
                    />
                </label>
                {move || error.get().map(|e| view! { <p class="error">{e}</p> })}
                <button type="submit" disabled=move || busy.get()>
                    "Create poll"
                </button>
            </form>
            <ol class="how card">
                <li><strong>"Invite"</strong>" — send everyone their own link."</li>
                <li><strong>"Suggest dates"</strong>" — anyone can add a day."</li>
                <li><strong>"Shared key"</strong>" — browsers create a key no single person holds."</li>
                <li><strong>"Vote"</strong>" — rate each day; ballots are encrypted in your browser."</li>
                <li><strong>"Tally"</strong>" — only the total is decrypted, with proofs anyone can check."</li>
            </ol>
        </main>
    }
}
