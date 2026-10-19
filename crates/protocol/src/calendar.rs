//! Calendar export for the picked days: an iCalendar (`.ics`) file with one
//! all-day event per day, and Google Calendar "add event" links.

/// One day to put in the calendar.
pub struct Event<'a> {
    /// ISO `YYYY-MM-DD`.
    pub date: &'a str,
    pub description: String,
}

fn parse(date: &str) -> Option<(u32, u32, u32)> {
    if !crate::is_iso_date(date) {
        return None;
    }
    Some((
        date[..4].parse().ok()?,
        date[5..7].parse().ok()?,
        date[8..].parse().ok()?,
    ))
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        2 if (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// `YYYYMMDD` for `date` and for the day after, which is where an all-day
/// event ends.
fn day_and_next(date: &str) -> Option<(String, String)> {
    let (y, m, d) = parse(date)?;
    let (ny, nm, nd) = if d < days_in_month(y, m) {
        (y, m, d + 1)
    } else if m < 12 {
        (y, m + 1, 1)
    } else {
        (y + 1, 1, 1)
    };
    Some((
        format!("{y:04}{m:02}{d:02}"),
        format!("{ny:04}{nm:02}{nd:02}"),
    ))
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

/// An `.ics` file with an all-day event per day. `stamp` is the creation time
/// as `YYYYMMDDTHHMMSSZ`; `uid_base` should be unique to the poll so that
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
        let Some((start, end)) = day_and_next(event.date) else {
            continue;
        };
        for line in [
            "BEGIN:VEVENT".to_string(),
            format!("UID:{uid_base}-{start}@whisperpoll"),
            format!("DTSTAMP:{stamp}"),
            format!("DTSTART;VALUE=DATE:{start}"),
            format!("DTEND;VALUE=DATE:{end}"),
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

/// A link that opens Google Calendar's "new event" form for one all-day event.
pub fn google_link(title: &str, event: &Event) -> Option<String> {
    let (start, end) = day_and_next(event.date)?;
    Some(format!(
        "https://calendar.google.com/calendar/render?action=TEMPLATE&text={}&dates={start}/{end}&details={}",
        percent_encode(title),
        percent_encode(&event.description),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(date: &str) -> Event<'_> {
        Event {
            date,
            description: "3 of 3 can make it".into(),
        }
    }

    #[test]
    fn all_day_events_end_the_next_day() {
        for (date, start, end) in [
            ("2026-10-24", "20261024", "20261025"),
            ("2026-10-31", "20261031", "20261101"),
            ("2026-12-31", "20261231", "20270101"),
            ("2028-02-28", "20280228", "20280229"),
            ("2026-02-28", "20260228", "20260301"),
            ("2100-02-28", "21000228", "21000301"),
            ("2000-02-28", "20000228", "20000229"),
        ] {
            assert_eq!(
                day_and_next(date),
                Some((start.into(), end.into())),
                "{date}"
            );
        }
        assert_eq!(day_and_next("not a date"), None);
    }

    #[test]
    fn ics_has_one_event_per_day() {
        let cal = ics(
            "Dinner; drinks, etc.",
            "abc",
            "20261019T120000Z",
            &[event("2026-10-25"), event("2026-10-31")],
        );
        assert!(cal.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(cal.ends_with("END:VCALENDAR\r\n"));
        assert_eq!(cal.matches("BEGIN:VEVENT").count(), 2);
        assert!(cal.contains("DTSTART;VALUE=DATE:20261031\r\nDTEND;VALUE=DATE:20261101\r\n"));
        assert!(cal.contains("UID:abc-20261025@whisperpoll\r\n"));
        assert!(cal.contains("SUMMARY:Dinner\\; drinks\\, etc.\r\n"));
        // Every line ends in CRLF; no bare LFs.
        assert!(!cal.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn long_lines_are_folded_without_splitting_characters() {
        let title = "Très long titre ".repeat(10);
        let cal = ics(&title, "abc", "20261019T120000Z", &[event("2026-10-25")]);
        for line in cal.split("\r\n") {
            assert!(line.len() <= 75, "{line:?}");
        }
        let unfolded = cal.replace("\r\n ", "");
        assert!(unfolded.contains(&format!("SUMMARY:{title}")));
    }

    #[test]
    fn google_link_is_encoded() {
        let link = google_link("Dinner & drinks", &event("2026-10-25")).unwrap();
        assert!(link.contains("text=Dinner%20%26%20drinks"));
        assert!(link.contains("dates=20261025/20261026"));
    }
}
