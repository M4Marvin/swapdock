//! RFC 3339 timestamps with no external dependency.
//!
//! A swapdock log is only useful if every line carries a timestamp that sorts
//! lexicographically and that anyone can read. Storing epoch milliseconds as a
//! separate integer field alongside the formatted string gives us both: exact
//! arithmetic for durations, human correlation across machines.
//!
//! The civil-from-days algorithm is Howard Hinnant's, which is exact for all
//! dates we will ever see and has no leap-second ambiguity to get wrong.

use serde::{Deserialize, Serialize};

/// An instant in time, resolved to UTC at millisecond resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Timestamp {
    epoch_millis: i64,
}

impl Timestamp {
    /// Builds a timestamp from milliseconds since the Unix epoch.
    pub const fn from_epoch_millis(epoch_millis: i64) -> Self {
        Self { epoch_millis }
    }

    /// Milliseconds since the Unix epoch.
    pub const fn epoch_millis(self) -> i64 {
        self.epoch_millis
    }

    /// The current time, read from the system clock.
    pub fn now() -> Self {
        match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => Self {
                epoch_millis: d.as_millis() as i64,
            },
            // Only possible if the clock is set before 1970. A negative value
            // is representable and still formats correctly, so use it.
            Err(e) => Self {
                epoch_millis: -(e.duration().as_millis() as i64),
            },
        }
    }

    /// Formats as `YYYY-MM-DDTHH:MM:SS.mmmZ`.
    ///
    /// Always UTC, always exactly 24 characters, so string sort equals time
    /// sort. That property is what lets us grep a log across machines.
    pub fn to_rfc3339(self) -> String {
        let total_secs = self.epoch_millis.div_euclid(1000);
        let millis = self.epoch_millis.rem_euclid(1000);

        let days = total_secs.div_euclid(86_400);
        let secs_of_day = total_secs.rem_euclid(86_400);

        let (year, month, day) = civil_from_days(days);
        let hour = secs_of_day / 3600;
        let minute = (secs_of_day % 3600) / 60;
        let second = secs_of_day % 60;

        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
    }
}

/// Parses `YYYY-MM-DDTHH:MM:SS[.mmm][Z|±HH:MM]` into milliseconds since epoch.
///
/// Accepts what nginx `$time_iso8601` emits (`2026-10-04T21:02:33+00:00`) as
/// well as the `Z` form this module writes. Returns `None` for anything else
/// rather than guessing: a swapdock gate must not misread a timestamp.
pub fn parse_rfc3339(text: &str) -> Option<i64> {
    let text = text.trim();
    let (datetime, offset) = split_offset(text)?;

    let date = datetime.get(..10)?;
    let time = datetime.get(10..)?;
    let time = time.strip_prefix(['T', 't', ' '])?;

    let year: i64 = date.get(..4)?.parse().ok()?;
    let month: u32 = date.get(5..7)?.parse().ok()?;
    let day: u32 = date.get(8..10)?.parse().ok()?;
    if date.as_bytes().get(4) != Some(&b'-') || date.as_bytes().get(7) != Some(&b'-') {
        return None;
    }

    let hour: i64 = time.get(..2)?.parse().ok()?;
    let minute: i64 = time.get(3..5)?.parse().ok()?;
    let second: i64 = time.get(6..8)?.parse().ok()?;
    if time.as_bytes().get(2) != Some(&b':') || time.as_bytes().get(5) != Some(&b':') {
        return None;
    }

    let mut millis: i64 = 0;
    let rest = time.get(8..).unwrap_or("");
    if let Some(frac) = rest.strip_prefix('.') {
        let digits: String = frac.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() || digits.len() > 3 {
            return None;
        }
        let scale = 10i64.pow(3 - digits.len() as u32);
        millis = digits.parse::<i64>().ok()? * scale;
    }

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    let days = days_from_civil(year, month, day)?;
    let local_ms = ((days * 86_400 + hour * 3600 + minute * 60 + second) * 1000) + millis;
    Some(local_ms - offset * 60_000)
}

/// Splits trailing `Z` or `±HH:MM` off, returning the offset in minutes east.
fn split_offset(text: &str) -> Option<(&str, i64)> {
    if let Some(base) = text.strip_suffix(['Z', 'z']) {
        return Some((base, 0));
    }
    // The sign closest to the end, after the date's own dashes.
    let pos = text.rfind(['+', '-'])?;
    if pos < 10 {
        return None;
    }
    let (base, zone) = text.split_at(pos);
    let (sign, hhmm) = zone.split_at(1);
    let (hh, mm) = hhmm.split_once(':')?;
    if hh.len() != 2 || mm.len() != 2 {
        return None;
    }
    let hours: i64 = hh.parse().ok()?;
    let minutes: i64 = mm.parse().ok()?;
    if hours > 23 || minutes > 59 {
        return None;
    }
    let total = hours * 60 + minutes;
    Some((base, if sign == "-" { -total } else { total }))
}

/// Days since 1970-01-01 for a proleptic Gregorian date. Inverse of
/// `civil_from_days`; rejects impossible dates like February 30.
fn days_from_civil(year: i64, month: u32, day: u32) -> Option<i64> {
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => return None,
    };
    if day == 0 || day > max_day {
        return None;
    }

    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (month as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Converts a count of days since 1970-01-01 into a proleptic Gregorian date.
///
/// Shift the epoch to 0000-03-01 so that the leap day lands at the end of the
/// 400-year cycle, which removes the special case from the month arithmetic.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // day of era, [0, 146097)

    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399)
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365)
    let mp = (5 * doy + 2) / 153; // [0, 11), March = 0

    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { year + 1 } else { year };

    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_the_unix_epoch() {
        assert_eq!(
            Timestamp::from_epoch_millis(0).to_rfc3339(),
            "1970-01-01T00:00:00.000Z"
        );
    }

    #[test]
    fn formats_a_known_instant() {
        // 2026-02-12T22:22:31.004Z
        assert_eq!(
            Timestamp::from_epoch_millis(1_770_934_951_004).to_rfc3339(),
            "2026-02-12T22:22:31.004Z"
        );
    }

    #[test]
    fn handles_a_leap_day() {
        // 2024-02-29 is the day most date code gets wrong.
        assert_eq!(
            Timestamp::from_epoch_millis(1_709_164_800_000).to_rfc3339(),
            "2024-02-29T00:00:00.000Z"
        );
    }

    #[test]
    fn handles_a_century_non_leap_year() {
        // 1900 is not a leap year; 2000 is. Both are century boundaries.
        assert_eq!(
            Timestamp::from_epoch_millis(-2_208_988_800_000).to_rfc3339(),
            "1900-01-01T00:00:00.000Z"
        );
        assert_eq!(
            Timestamp::from_epoch_millis(946_684_800_000).to_rfc3339(),
            "2000-01-01T00:00:00.000Z"
        );
    }

    #[test]
    fn output_has_a_constant_width_so_strings_sort_as_times() {
        for millis in [0, 1, 999, 1_000, 1_770_934_951_004] {
            assert_eq!(
                Timestamp::from_epoch_millis(millis).to_rfc3339().len(),
                24,
                "width must be constant for millis={millis}"
            );
        }
    }

    #[test]
    fn string_order_equals_time_order() {
        let early = Timestamp::from_epoch_millis(1_700_000_000_000).to_rfc3339();
        let late = Timestamp::from_epoch_millis(1_800_000_000_000).to_rfc3339();
        assert!(early < late, "{early} must sort before {late}");
    }

    #[test]
    fn parse_round_trips_through_format() {
        for millis in [
            0,
            1,
            999,
            1_000,
            1_709_164_800_000, // 2024-02-29, the leap day
            1_770_934_951_004,
            1_800_000_000_123,
        ] {
            let text = Timestamp::from_epoch_millis(millis).to_rfc3339();
            assert_eq!(parse_rfc3339(&text), Some(millis), "{text}");
        }
    }

    #[test]
    fn parse_accepts_nginx_iso8601() {
        // Exactly what $time_iso8601 emits.
        assert_eq!(
            parse_rfc3339("2026-10-04T21:02:33+00:00"),
            parse_rfc3339("2026-10-04T21:02:33Z")
        );
        assert_eq!(
            parse_rfc3339("2026-10-04T21:02:33.123+00:00"),
            Some(parse_rfc3339("2026-10-04T21:02:33.123Z").expect("must parse"))
        );
    }

    #[test]
    fn parse_applies_the_offset() {
        let zulu = parse_rfc3339("2026-10-04T21:02:33Z").unwrap();
        // +02:00 is two hours east, so the instant is two hours earlier in UTC.
        assert_eq!(parse_rfc3339("2026-10-04T23:02:33+02:00"), Some(zulu));
        assert_eq!(parse_rfc3339("2026-10-04T16:02:33-05:00"), Some(zulu));
    }

    #[test]
    fn parse_rejects_impossible_dates() {
        for bad in [
            "",
            "yesterday",
            "2026-10-04",
            "2026-10-04T25:00:00Z",
            "2026-13-01T00:00:00Z",
            "2026-02-30T00:00:00Z",      // February never has 30 days
            "2023-02-29T00:00:00Z",      // 2023 is not a leap year
            "2026-10-04T21:02:33",       // no zone
            "2026-10-04T21:02:33.1234Z", // more than milliseconds
            "2026-10-04T21:02:33+25:00",
        ] {
            assert_eq!(parse_rfc3339(bad), None, "{bad:?} must be rejected");
        }
    }

    #[test]
    fn now_is_after_2020() {
        // Guards against a millis/seconds mix-up in now().
        assert!(Timestamp::now().epoch_millis() > 1_577_836_800_000);
    }
}
