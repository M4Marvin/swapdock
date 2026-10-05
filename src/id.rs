//! Run identifiers.
//!
//! A run id is 27 lowercase hex characters: 11 of milliseconds-since-epoch
//! followed by 16 of randomness. That shape buys three properties a swapdock tool
//! needs:
//!
//! * **Sortable** — ids sort in the order the runs were created, so `ls` on a
//!   directory of them is a timeline.
//! * **Typed** — 27 characters is short enough to type after a paste, and long
//!   enough that a typo is caught by the log lookup.
//! * **Grouped** — every step record of one run carries the same id, so
//!   `grep <id> swapdock.jsonl` reconstructs the whole run in order.

use std::fmt;

use crate::time::Timestamp;

/// Length of the random suffix, in hex characters.
const RANDOM_HEX_LEN: usize = 16;

/// A unique, time-sortable identifier for one invocation of the tool.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RunId(String);

impl RunId {
    /// Generates a new id from the system clock and the kernel CSPRNG.
    pub fn generate() -> Self {
        let millis = Timestamp::now().epoch_millis().max(0) as u64;
        Self(format!("{millis:011x}{}", random_hex(RANDOM_HEX_LEN / 2)))
    }

    /// Wraps an id that came from outside, for resuming a run.
    ///
    /// Returns `None` for anything that is not 27 lowercase hex characters, so
    /// a typo from the command line fails with a clear message instead of
    /// silently matching nothing.
    pub fn parse(raw: &str) -> Option<Self> {
        let valid = raw.len() == 11 + RANDOM_HEX_LEN
            && raw
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
        valid.then(|| Self(raw.to_string()))
    }

    /// The id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Reads exactly `n` bytes from the kernel CSPRNG and returns them as hex.
///
/// The read must be bounded. `/dev/urandom` is an endless stream, so
/// `fs::read("/dev/urandom")` never reaches EOF and allocates until the process
/// is killed. `read_exact` asks for the byte count we want and stops.
///
/// `/dev/urandom` is the right source here: ids do not need the fork-safety of
/// getrandom, and this avoids a dependency for ten lines of arithmetic.
fn random_hex(n: usize) -> String {
    use std::io::Read as _;

    let mut buf = vec![0u8; n];
    let filled = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .is_ok();

    if !filled {
        // Should not happen on Linux. Mix the clock and the pid rather than
        // returning a constant, which would collide across processes.
        let seed = Timestamp::now().epoch_millis() as u64
            ^ ((std::process::id() as u64) << 32)
            ^ (buf[0] as u64);
        for (i, b) in buf.iter_mut().enumerate() {
            *b = (seed >> (i % 8 * 8)) as u8 ^ (i as u8);
        }
    }

    buf.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::time::{Duration, Instant};

    #[test]
    fn has_the_expected_shape() {
        let id = RunId::generate();
        assert_eq!(id.as_str().len(), 27);
        assert!(
            id.as_str()
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "must be lowercase hex: {}",
            id.as_str()
        );
    }

    #[test]
    fn round_trips_through_parse() {
        let id = RunId::generate();
        assert_eq!(RunId::parse(id.as_str()), Some(id.clone()));
    }

    #[test]
    fn rejects_malformed_input() {
        for bad in [
            "",
            "short",
            "01JQ7FABCDEFGHJKMNPQRSTUVWXYZ", // 27 chars but not hex
            "01jq7fabcdefghjkmnpqrstuvwxy",  // 26 chars
            "01JQ7FABCDEFGHJKMNPQRSTUVWXY",  // uppercase
        ] {
            assert_eq!(RunId::parse(bad), None, "should reject {bad:?}");
        }
    }

    #[test]
    fn ids_are_unique_across_many_generations() {
        let ids: HashSet<String> = (0..10_000).map(|_| RunId::generate().to_string()).collect();
        assert_eq!(ids.len(), 10_000, "no collisions allowed");
    }

    #[test]
    fn the_timestamp_prefix_never_goes_backwards() {
        // Ids sort by time, so the prefix must be monotonic. Comparing full ids
        // would be flaky: two ids created in the same millisecond are ordered by
        // their random suffix, not by time.
        let stamps: Vec<u64> = (0..500)
            .map(|_| {
                let id = RunId::generate();
                u64::from_str_radix(&id.as_str()[..11], 16).expect("hex prefix")
            })
            .collect();

        for pair in stamps.windows(2) {
            assert!(
                pair[0] <= pair[1],
                "time prefix went backwards: {} then {}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn random_hex_produces_the_requested_length() {
        assert_eq!(random_hex(8).len(), 16);
        assert_eq!(random_hex(4).len(), 8);
    }

    #[test]
    fn random_hex_returns_instead_of_reading_an_endless_device() {
        // Regression: `fs::read("/dev/urandom")` never sees EOF, so it hangs
        // here and grows until the process is OOM-killed. The read must be
        // bounded to exactly the requested byte count.
        let started = Instant::now();
        let hex = random_hex(8);
        assert_eq!(hex.len(), 16);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "random_hex must return promptly, took {:?}",
            started.elapsed()
        );
        assert!(
            hex.chars().all(|c| c.is_ascii_hexdigit()),
            "expected hex, got {hex:?}"
        );
    }

    #[test]
    fn random_hex_gives_different_values_each_call() {
        let a = random_hex(8);
        let b = random_hex(8);
        assert_ne!(a, b, "the CSPRNG must not return a constant");
    }
}
