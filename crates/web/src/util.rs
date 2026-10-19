use leptos::prelude::window;
use wasm_bindgen::{JsCast, JsValue};

pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn from_hex<const N: usize>(hex: &str) -> Option<[u8; N]> {
    if hex.len() != 2 * N {
        return None;
    }
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

/// `2026-10-24` → `Sat 24 Oct`, in the viewer's locale order.
pub fn pretty_date(iso: &str) -> String {
    let date = js_sys::Date::new(&JsValue::from_str(&format!("{iso}T12:00:00")));
    if date.get_time().is_nan() {
        return iso.to_string();
    }
    let opts = js_sys::Object::new();
    for (k, v) in [("weekday", "short"), ("day", "numeric"), ("month", "short")] {
        let _ = js_sys::Reflect::set(&opts, &k.into(), &v.into());
    }
    date.to_locale_date_string("en-GB", &opts).into()
}

pub fn invite_url(poll: &str, token: &str) -> String {
    let origin = window().location().origin().unwrap_or_default();
    format!("{origin}/p/{poll}?t={token}")
}

/// Offers `contents` to the viewer as a file download.
pub fn download(filename: &str, mime: &str, contents: &str) {
    let parts = js_sys::Array::of1(&JsValue::from_str(contents));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type(mime);
    let Ok(blob) = web_sys::Blob::new_with_str_sequence_and_options(&parts, &opts) else {
        return;
    };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else {
        return;
    };
    let link = window()
        .document()
        .and_then(|d| d.create_element("a").ok())
        .and_then(|e| e.dyn_into::<web_sys::HtmlAnchorElement>().ok());
    if let Some(link) = link {
        link.set_href(&url);
        link.set_download(filename);
        link.click();
    }
    let _ = web_sys::Url::revoke_object_url(&url);
}

/// The current time as an iCalendar UTC stamp, `YYYYMMDDTHHMMSSZ`.
pub fn utc_stamp() -> String {
    let iso: String = js_sys::Date::new_0().to_iso_string().into();
    let compact: String = iso
        .split('.')
        .next()
        .unwrap_or_default()
        .chars()
        .filter(|c| *c != '-' && *c != ':')
        .collect();
    format!("{compact}Z")
}

/// A safe file name from a poll title: `Team dinner!` → `team-dinner`.
pub fn file_slug(title: &str) -> String {
    let slug = title
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        "whisperpoll".into()
    } else {
        slug
    }
}

pub fn copy_to_clipboard(text: &str) {
    let _ = window().navigator().clipboard().write_text(text);
}
