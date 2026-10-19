use leptos::prelude::window;
use wasm_bindgen::{JsCast, JsValue};
use whisperpoll_protocol::slot::Slot;

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

fn js_options(pairs: &[(&str, &str)]) -> js_sys::Object {
    let opts = js_sys::Object::new();
    for (k, v) in pairs {
        let _ = js_sys::Reflect::set(&opts, &(*k).into(), &(*v).into());
    }
    opts
}

/// A slot (`2026-10-30T18:00+02:00/120`) in the viewer's time zone:
/// `Fri 30 Oct, 18:00–20:00`.
pub fn pretty_slot(text: &str) -> String {
    let Some(slot) = Slot::parse(text) else {
        return text.to_string();
    };
    let start = js_sys::Date::new(&JsValue::from_f64(slot.start_epoch_millis() as f64));
    let end = js_sys::Date::new(&JsValue::from_f64(
        (slot.end_epoch_minutes() * 60_000) as f64,
    ));
    let day = js_options(&[("weekday", "short"), ("day", "numeric"), ("month", "short")]);
    let time = js_options(&[("hour", "2-digit"), ("minute", "2-digit")]);
    let day: String = start.to_locale_date_string("en-GB", &day).into();
    let from: String = start
        .to_locale_time_string_with_options("en-GB", &time)
        .into();
    let to: String = end
        .to_locale_time_string_with_options("en-GB", &time)
        .into();
    format!("{day}, {from}–{to}")
}

/// This browser's UTC offset in minutes at a local `date` and `time`
/// (it differs across daylight-saving changes).
pub fn local_offset_minutes(date: &str, time: &str) -> i32 {
    let local = js_sys::Date::new(&JsValue::from_str(&format!("{date}T{time}:00")));
    -(local.get_timezone_offset() as i32)
}

/// `90` → `1 h 30 min`.
pub fn duration(minutes: u32) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
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
