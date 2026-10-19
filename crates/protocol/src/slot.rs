//! A candidate meeting time: a start (the local date and time on the
//! suggester's device, with its UTC offset) and a duration.
//!
//! Its canonical text, `2026-10-30T18:00+02:00/120`, is what a poll's options
//! hold. The offset pins the start to one instant, so everyone sees it in
//! their own local time and calendar events land at the right moment.

use std::fmt;

/// Shortest and longest slot, in minutes.
pub const MIN_MINUTES: u32 = 15;
pub const MAX_MINUTES: u32 = 24 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub year: u32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    /// Local time minus UTC, in minutes (`+02:00` is 120).
    pub offset_minutes: i32,
    /// Duration.
    pub minutes: u32,
}

pub(crate) fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        2 if (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 (Howard Hinnant's `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date of a day count since 1970-01-01.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// `YYYYMMDDTHHMMSSZ` for minutes since the epoch, as iCalendar wants.
pub fn utc_stamp(epoch_minutes: i64) -> String {
    let (y, m, d) = civil_from_days(epoch_minutes.div_euclid(24 * 60));
    let of_day = epoch_minutes.rem_euclid(24 * 60);
    format!("{y:04}{m:02}{d:02}T{:02}{:02}00Z", of_day / 60, of_day % 60)
}

fn two(s: &str) -> Option<u32> {
    (s.len() == 2 && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}

impl Slot {
    /// From the parts a form collects: `2026-10-30`, `18:00`, the device's
    /// offset at that moment, and a duration. `None` if anything's invalid.
    pub fn new(date: &str, time: &str, offset_minutes: i32, minutes: u32) -> Option<Slot> {
        if !crate::is_iso_date(date) {
            return None;
        }
        let (year, month, day) = (date[..4].parse().ok()?, two(&date[5..7])?, two(&date[8..])?);
        let (h, m) = time.split_once(':')?;
        let (hour, minute) = (two(h)?, two(m)?);
        let slot = Slot {
            year,
            month,
            day,
            hour,
            minute,
            offset_minutes,
            minutes,
        };
        let valid = day <= days_in_month(year, month)
            && hour < 24
            && minute < 60
            && offset_minutes.abs() <= 14 * 60
            && (MIN_MINUTES..=MAX_MINUTES).contains(&minutes);
        valid.then_some(slot)
    }

    /// Parses the canonical text, `2026-10-30T18:00+02:00/120` (or `…Z/120`).
    pub fn parse(text: &str) -> Option<Slot> {
        let (start, minutes) = text.split_once('/')?;
        let minutes = minutes.parse().ok()?;
        let (date, rest) = start.split_once('T')?;
        if rest.len() < 6 {
            return None;
        }
        let (time, offset) = rest.split_at(5);
        let offset_minutes = match offset {
            "Z" => 0,
            _ => {
                let sign = match offset.as_bytes().first()? {
                    b'+' => 1,
                    b'-' => -1,
                    _ => return None,
                };
                let (h, m) = offset[1..].split_once(':')?;
                sign * (two(h)? * 60 + two(m)?) as i32
            }
        };
        Slot::new(date, time, offset_minutes, minutes)
    }

    pub fn date(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// Minutes since the Unix epoch at which the slot starts, in UTC.
    pub fn start_epoch_minutes(&self) -> i64 {
        let days = days_from_civil(self.year as i64, self.month as i64, self.day as i64);
        days * 24 * 60 + (self.hour * 60 + self.minute) as i64 - self.offset_minutes as i64
    }

    pub fn end_epoch_minutes(&self) -> i64 {
        self.start_epoch_minutes() + self.minutes as i64
    }

    /// Milliseconds since the epoch, for clients' date libraries.
    pub fn start_epoch_millis(&self) -> i64 {
        self.start_epoch_minutes() * 60_000
    }

    /// `Fri 30 Oct, 18:00–20:00`, in the slot's own offset. For text the
    /// server writes (like push notifications); clients format slots in the
    /// viewer's own time zone instead.
    pub fn short(&self) -> String {
        const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let days = days_from_civil(self.year as i64, self.month as i64, self.day as i64);
        let weekday = DAYS[days.rem_euclid(7) as usize];
        let end = (self.hour * 60 + self.minute + self.minutes) % (24 * 60);
        format!(
            "{weekday} {} {}, {:02}:{:02}–{:02}:{:02}",
            self.day,
            MONTHS[self.month as usize - 1],
            self.hour,
            self.minute,
            end / 60,
            end % 60
        )
    }
}

impl fmt::Display for Slot {
    /// The canonical text.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.offset_minutes < 0 { '-' } else { '+' };
        let off = self.offset_minutes.unsigned_abs();
        write!(
            f,
            "{}T{:02}:{:02}{sign}{:02}:{:02}/{}",
            self.date(),
            self.hour,
            self.minute,
            off / 60,
            off % 60,
            self.minutes
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_text_roundtrips() {
        let slot = Slot::new("2026-10-30", "18:00", 120, 120).unwrap();
        assert_eq!(slot.to_string(), "2026-10-30T18:00+02:00/120");
        assert_eq!(Slot::parse("2026-10-30T18:00+02:00/120"), Some(slot));
        let west = Slot::new("2026-10-30", "09:30", -330, 45).unwrap();
        assert_eq!(west.to_string(), "2026-10-30T09:30-05:30/45");
        assert_eq!(Slot::parse(&west.to_string()), Some(west));
        assert_eq!(
            Slot::parse("2026-10-30T18:00Z/60").unwrap().to_string(),
            "2026-10-30T18:00+00:00/60"
        );
    }

    #[test]
    fn invalid_slots_are_rejected() {
        for bad in [
            "2026-10-30T18:00+02:00",
            "2026-10-30T24:00+02:00/60",
            "2026-10-30T18:60+02:00/60",
            "2026-02-30T18:00+02:00/60",
            "2026-10-30T18:00+15:00/60",
            "2026-10-30T18:00+02:00/10",
            "2026-10-30T18:00+02:00/1441",
            "2026-10-30 18:00+02:00/60",
            "2026-10-30T8:00+02:00/60",
            "nonsense",
        ] {
            assert_eq!(Slot::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn instants_account_for_the_offset() {
        // 18:00 at +02:00 is 16:00 UTC.
        let slot = Slot::parse("2026-10-30T18:00+02:00/120").unwrap();
        assert_eq!(utc_stamp(slot.start_epoch_minutes()), "20261030T160000Z");
        assert_eq!(utc_stamp(slot.end_epoch_minutes()), "20261030T180000Z");
        // Crossing midnight backwards and forwards in UTC.
        let early = Slot::parse("2026-01-01T01:00+03:00/30").unwrap();
        assert_eq!(utc_stamp(early.start_epoch_minutes()), "20251231T220000Z");
        let late = Slot::parse("2026-02-28T23:00-02:00/90").unwrap();
        assert_eq!(utc_stamp(late.start_epoch_minutes()), "20260301T010000Z");
        assert_eq!(
            Slot::parse("1970-01-01T00:00Z/60")
                .unwrap()
                .start_epoch_millis(),
            0
        );
    }

    #[test]
    fn short_text_has_weekday_and_times() {
        let slot = Slot::parse("2026-10-30T18:00+02:00/120").unwrap();
        assert_eq!(slot.short(), "Fri 30 Oct, 18:00–20:00");
        let overnight = Slot::parse("2026-10-31T23:00+02:00/120").unwrap();
        assert_eq!(overnight.short(), "Sat 31 Oct, 23:00–01:00");
    }
}
