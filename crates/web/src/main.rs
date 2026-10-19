//! whisperpoll browser client. All cryptography runs here: trustee keys never
//! leave the browser, and every result is re-verified locally.

mod auto;
mod home;
mod net;
mod poll_page;
mod store;
mod util;

use leptos::prelude::*;

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

#[component]
fn App() -> impl IntoView {
    let location = window().location();
    let path = location.pathname().unwrap_or_default();
    let search = location.search().unwrap_or_default();
    let token = web_sys::UrlSearchParams::new_with_str(&search)
        .ok()
        .and_then(|p| p.get("t"));
    match (path.strip_prefix("/p/"), token) {
        (Some(id), Some(token)) if !id.is_empty() => {
            view! { <poll_page::PollPage id=id.to_string() token=token /> }.into_any()
        }
        _ => view! { <home::Home /> }.into_any(),
    }
}
