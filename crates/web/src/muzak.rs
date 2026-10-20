//! Waiting music: procedurally composed lounge muzak (`whisperpoll-muzak`),
//! a new piece after each one ends, behind a sticker button that mutes it.
//! Browsers only allow sound after the first click or key press, so that's
//! when it starts; muting is remembered.

use std::cell::RefCell;

use leptos::ev;
use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{AudioContext, GainNode};

use crate::store;

struct Player {
    ctx: AudioContext,
    out: GainNode,
}

thread_local! {
    static PLAYER: RefCell<Option<Player>> = const { RefCell::new(None) };
}

/// Starts (or resumes) the music; `title` gets each piece's name.
fn start(title: RwSignal<Option<String>>) {
    let resumed = PLAYER.with(|p| {
        p.borrow().as_ref().map(|p| {
            let _ = p.ctx.resume();
        })
    });
    if resumed.is_some() {
        return;
    }
    let Ok(ctx) = AudioContext::new() else { return };
    let Ok(out) = ctx.create_gain() else { return };
    out.gain().set_value(0.5);
    if out.connect_with_audio_node(&ctx.destination()).is_err() {
        return;
    }
    PLAYER.with(|p| *p.borrow_mut() = Some(Player { ctx, out }));
    play_next(title);
}

fn stop() {
    PLAYER.with(|p| {
        if let Some(p) = p.borrow().as_ref() {
            let _ = p.ctx.suspend();
        }
    });
}

/// Composes a fresh piece and plays it; when it ends, the next one.
fn play_next(title: RwSignal<Option<String>>) {
    PLAYER.with(|p| {
        let p = p.borrow();
        let Some(player) = p.as_ref() else { return };
        let seed = (js_sys::Math::random() * 9_007_199_254_740_991.0) as u64;
        let piece = whisperpoll_muzak::compose(seed);
        let rate = whisperpoll_muzak::SAMPLE_RATE as f32;
        let Ok(buffer) = player
            .ctx
            .create_buffer(1, piece.samples.len() as u32, rate)
        else {
            return;
        };
        if buffer.copy_to_channel(&piece.samples, 0).is_err() {
            return;
        }
        let Ok(source) = player.ctx.create_buffer_source() else {
            return;
        };
        source.set_buffer(Some(&buffer));
        if source.connect_with_audio_node(&player.out).is_err() {
            return;
        }
        let ended = Closure::once_into_js(move || play_next(title));
        let scheduled: &web_sys::AudioScheduledSourceNode = &source;
        scheduled.set_onended(Some(ended.unchecked_ref()));
        if source.start().is_ok() {
            title.set(Some(piece.title));
        }
    });
}

/// The floating mute button. Mount once per page.
#[component]
pub fn MuzakButton() -> impl IntoView {
    let muted = RwSignal::new(store::muzak_muted());
    let title = RwSignal::new(None::<String>);

    // The first gesture anywhere on the page may start the music.
    let first = StoredValue::new(Vec::<WindowListenerHandle>::new());
    let begin = move || {
        first.update_value(|handles| handles.drain(..).for_each(|h| h.remove()));
        if !muted.get_untracked() {
            start(title);
        }
    };
    first.update_value(|handles| {
        handles.push(window_event_listener(ev::pointerdown, move |_| begin()));
        handles.push(window_event_listener(ev::keydown, move |_| begin()));
    });

    let toggle = move |_| {
        let now_muted = !muted.get_untracked();
        muted.set(now_muted);
        store::set_muzak_muted(now_muted);
        if now_muted {
            stop();
        } else {
            start(title);
        }
    };
    let playing = move || !muted.get() && title.get().is_some();
    let label = move || {
        if muted.get() {
            "Muzak is off. Turn it on".to_string()
        } else {
            match title.get() {
                Some(t) => format!("Now playing: {t}. Mute the muzak"),
                None => "Mute the muzak".to_string(),
            }
        }
    };

    view! {
        <button
            class="muzak"
            class:off=move || muted.get()
            on:click=toggle
            aria-pressed=move || (!muted.get()).to_string()
            aria-label=label
            title=label
        >
            <svg
                class="icon"
                class:spin=playing
                width="28"
                height="28"
                viewBox="0 0 24 24"
                aria-hidden="true"
            >
                <circle cx="12" cy="12" r="10.5" fill="#000" stroke="currentColor" stroke-width="1.5"></circle>
                <circle cx="12" cy="12" r="6.5" fill="none" stroke="currentColor" stroke-width="0.8" stroke-dasharray="2 2"></circle>
                <circle cx="12" cy="12" r="2.6" fill="#ff2bd6"></circle>
            </svg>
            <span class="muzak-text">
                <span>{move || if muted.get() { "Muzak: off" } else { "Muzak: on" }}</span>
                <span class="muzak-title">
                    {move || match (muted.get(), title.get()) {
                        (true, _) => "tap for elevator vibes".to_string(),
                        (false, Some(t)) => format!("♪ {t}"),
                        (false, None) => "starts on your first click".to_string(),
                    }}
                </span>
            </span>
        </button>
    }
}
