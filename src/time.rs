//! RFC 3339 timestamps with no external dependency.
//!
//! A deploy log is only useful if every line carries a timestamp that sorts
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
    fn now_is_after_2020() {
        // Guards against a millis/seconds mix-up in now().
        assert!(Timestamp::now().epoch_millis() > 1_577_836_800_000);
    }
}
