//! Calendar export for the picked slots: an iCalendar (`.ics`) file with one
//! event per slot, and Google Calendar "add event" links. Times are written
//! in UTC, so every calendar shows them in its own time zone.

use crate::slot::{utc_stamp, Slot};

/// One slot to put in the calendar.
pub struct Event {
    pub slot: Slot,
    pub description: String,
}

/// Escapes a TEXT value (RFC 5545 §3.3.11).
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// Folds a content line to at most 75 octets per line (RFC 5545 §3.1),
/// never splitting a UTF-8 character.
fn fold(line: &str, out: &mut String) {
    let mut width = 0;
    for c in line.chars() {
        let len = c.len_utf8();
        if width + len > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += len;
    }
    out.push_str("\r\n");
}

/// An `.ics` file with an event per slot. `stamp` is the creation time as
/// `YYYYMMDDTHHMMSSZ`; `uid_base` should be unique to the poll so that
/// re-importing updates the events instead of duplicating them.
pub fn ics(title: &str, uid_base: &str, stamp: &str, events: &[Event]) -> String {
    let mut out = String::new();
    for line in [
        "BEGIN:VCALENDAR",
        "VERSION:2.0",
        "PRODID:-//whisperpoll//EN",
        "CALSCALE:GREGORIAN",
        "METHOD:PUBLISH",
    ] {
        fold(line, &mut out);
    }
    for event in events {
        let start = utc_stamp(event.slot.start_epoch_minutes());
        let end = utc_stamp(event.slot.end_epoch_minutes());
        for line in [
            "BEGIN:VEVENT".to_string(),
            format!("UID:{uid_base}-{start}@whisperpoll"),
            format!("DTSTAMP:{stamp}"),
            format!("DTSTART:{start}"),
            format!("DTEND:{end}"),
            format!("SUMMARY:{}", escape(title)),
            format!("DESCRIPTION:{}", escape(&event.description)),
            "END:VEVENT".to_string(),
        ] {
            fold(&line, &mut out);
        }
    }
    fold("END:VCALENDAR", &mut out);
    out
}

fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// A link that opens Google Calendar's "new event" form for one slot.
pub fn google_link(title: &str, event: &Event) -> String {
    format!(
        "https://calendar.google.com/calendar/render?action=TEMPLATE&text={}&dates={}/{}&details={}",
        percent_encode(title),
        utc_stamp(event.slot.start_epoch_minutes()),
        utc_stamp(event.slot.end_epoch_minutes()),
        percent_encode(&event.description),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(slot: &str) -> Event {
        Event {
            slot: Slot::parse(slot).unwrap(),
            description: "3 of 3 can make it".into(),
        }
    }

    #[test]
    fn ics_has_one_timed_event_per_slot() {
        let cal = ics(
            "Dinner; drinks, etc.",
            "abc",
            "20261019T120000Z",
            &[
                event("2026-10-25T18:00+02:00/120"),
                event("2026-10-31T23:30+01:00/90"),
            ],
        );
        assert!(cal.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(cal.ends_with("END:VCALENDAR\r\n"));
        assert_eq!(cal.matches("BEGIN:VEVENT").count(), 2);
        assert!(cal.contains("DTSTART:20261025T160000Z\r\nDTEND:20261025T180000Z\r\n"));
        // 23:30 at +01:00 is 22:30 UTC; 90 minutes later is past midnight.
        assert!(cal.contains("DTSTART:20261031T223000Z\r\nDTEND:20261101T000000Z\r\n"));
        assert!(cal.contains("UID:abc-20261025T160000Z@whisperpoll\r\n"));
        assert!(cal.contains("SUMMARY:Dinner\\; drinks\\, etc.\r\n"));
        // Every line ends in CRLF; no bare LFs.
        assert!(!cal.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn long_lines_are_folded_without_splitting_characters() {
        let title = "Très long titre ".repeat(10);
        let cal = ics(
            &title,
            "abc",
            "20261019T120000Z",
            &[event("2026-10-25T18:00Z/60")],
        );
        for line in cal.split("\r\n") {
            assert!(line.len() <= 75, "{line:?}");
        }
        let unfolded = cal.replace("\r\n ", "");
        assert!(unfolded.contains(&format!("SUMMARY:{title}")));
    }

    #[test]
    fn google_link_is_encoded() {
        let link = google_link("Dinner & drinks", &event("2026-10-25T18:00+02:00/120"));
        assert!(link.contains("text=Dinner%20%26%20drinks"));
        assert!(link.contains("dates=20261025T160000Z/20261025T180000Z"));
    }
}
