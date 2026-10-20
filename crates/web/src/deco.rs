//! Decoration: the scrolling ticker, the animated icons and the 88×31
//! badges. All of it is `aria-hidden`; the animations stop when the browser
//! asks for reduced motion (see style.css).

use leptos::prelude::*;

/// A strip that scrolls `text` across the top of the page.
#[component]
pub fn Ticker(#[prop(into)] text: Signal<String>) -> impl IntoView {
    // Twice, so the loop (which moves by half its width) is seamless.
    let run = move || {
        let text = text.get();
        format!("★ {text} ★ {text} ★ {text} ★ {text} ")
    };
    view! {
        <div class="ticker" aria-hidden="true">
            <div class="ticker-run">{run}</div>
        </div>
    }
}

#[component]
pub fn Badges() -> impl IntoView {
    view! {
        <footer class="badges" aria-hidden="true">
            <span class="badge88 pink rainbow">"BEST VIEWED IN ANY BROWSER"</span>
            <span class="badge88 black">"100% ENCRYPTED"</span>
            <span class="badge88 cyan">"KEYS NEVER LEAVE"</span>
        </footer>
    }
}

/// The spinning star on the current step.
#[component]
pub fn Star(#[prop(default = "star spin")] class: &'static str) -> impl IntoView {
    view! {
        <svg class=class width="26" height="26" viewBox="0 0 24 24" aria-hidden="true">
            <path
                d="M12 2l2.9 6.6 7.1.6-5.4 4.7 1.6 7-6.2-3.7-6.2 3.7 1.6-7L2 9.2l7.1-.6z"
                fill="#ffe600"
                stroke="#000"
                stroke-width="1.6"
                stroke-linejoin="round"
            ></path>
        </svg>
    }
}

#[component]
pub fn Envelope() -> impl IntoView {
    view! {
        <svg class="icon bob" width="32" height="32" viewBox="0 0 24 24" fill="#fff" stroke="#000" stroke-width="1.8" stroke-linejoin="round" aria-hidden="true">
            <rect x="2.5" y="5" width="19" height="14"></rect>
            <path d="M2.5 6l9.5 7 9.5-7" fill="none"></path>
        </svg>
    }
}

#[component]
pub fn Clock() -> impl IntoView {
    view! {
        <svg class="icon" width="32" height="32" viewBox="0 0 24 24" aria-hidden="true">
            <circle cx="12" cy="12" r="10" fill="#fff" stroke="#000" stroke-width="1.8"></circle>
            <path d="M12 12V7.5" stroke="#000" stroke-width="2" stroke-linecap="round"></path>
            <path class="hand" d="M12 12h5" stroke="#ff2bd6" stroke-width="2" stroke-linecap="round"></path>
        </svg>
    }
}

/// A padlock; `class` picks its animation and `color` its fill.
#[component]
pub fn Lock(
    #[prop(default = "icon pulse")] class: &'static str,
    #[prop(default = "#c6ff00")] fill: &'static str,
    #[prop(default = "#000")] stroke: &'static str,
) -> impl IntoView {
    view! {
        <svg class=class width="30" height="30" viewBox="0 0 24 24" fill=fill stroke=stroke stroke-width="2" stroke-linejoin="round" aria-hidden="true">
            <rect x="4" y="10.5" width="16" height="11"></rect>
            <path d="M8 10.5V7a4 4 0 0 1 8 0v3.5" fill="none"></path>
        </svg>
    }
}

/// The arrow on buttons that move the poll on.
#[component]
pub fn Arrow() -> impl IntoView {
    view! {
        <svg class="icon nudge" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="square" aria-hidden="true">
            <path d="M4 12h15M13 5l7 7-7 7"></path>
        </svg>
    }
}

#[component]
pub fn Calendar() -> impl IntoView {
    view! {
        <svg class="icon bob" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" aria-hidden="true">
            <rect x="3.5" y="5" width="17" height="15.5"></rect>
            <path d="M3.5 10h17M8 3v4M16 3v4"></path>
        </svg>
    }
}
