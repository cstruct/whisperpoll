use leptos::prelude::window;
use wasm_bindgen::JsValue;

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

pub fn copy_to_clipboard(text: &str) {
    let _ = window().navigator().clipboard().write_text(text);
}
