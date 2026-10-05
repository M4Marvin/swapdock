//! Verifying a swapdock against the nginx access log.
//!
//! The swapdock tool must not grade its own homework. Exit code zero from a
//! reload means the signal was accepted, not that traffic moved. The access log
//! is the system's own record, so `verify` reads it back:
//!
//! ```text
//! log_format swapdock '$time_iso8601 $host $status rt=$request_time '
//!                    'up=$upstream_addr us=$upstream_status';
//! ```
//!
//! For one app and a start time, the report answers three questions:
//!
//! 1. Did any request fail (5xx) since the swapdock?
//! 2. Which upstreams served, and when did each first appear?
//! 3. At what instant did traffic move from the old upstream to the new one?
//!
//! Malformed lines are counted and skipped, never fatal: a log with one corrupt
//! line still answers for the other ten thousand.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::registry::normalize_hostname;
use crate::time::parse_rfc3339;

/// Default location of the front-door access log on the swapdock host.
pub const DEFAULT_ACCESS_LOG: &str = "/var/log/nginx/front-door.access.log";

/// One parsed access-log line.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub ts_ms: i64,
    pub host: String,
    pub status: u16,
    pub request_time: f64,
    /// `ip:port` of the upstream that answered, when the line has one.
    pub upstream: Option<String>,
    pub upstream_status: Option<u16>,
}

/// Parses one log line. `None` means malformed: counted, not fatal.
pub fn parse_line(line: &str) -> Option<Entry> {
    let mut parts = line.split_whitespace();
    let ts = parts.next()?;
    let host = parts.next()?;
    let status: u16 = parts.next()?.parse().ok()?;
    let ts_ms = parse_rfc3339(ts)?;

    let mut request_time = 0.0;
    let mut upstream = None;
    let mut upstream_status = None;

    for part in parts {
        if let Some(v) = part.strip_prefix("rt=") {
            request_time = v.parse().unwrap_or(0.0);
        } else if let Some(v) = part.strip_prefix("up=") {
            // An upstream set renders as "a:1, b:2"; only single-upstream lines
            // can be attributed, so anything with a comma is left unknown.
            if v != "-" && !v.contains(',') {
                upstream = Some(v.to_string());
            }
        } else if let Some(v) = part.strip_prefix("us=") {
            upstream_status = v.split(',').next()?.parse().ok();
        }
    }

    Some(Entry {
        ts_ms,
        host: host.to_string(),
        status,
        request_time,
        upstream,
        upstream_status,
    })
}

/// What one upstream did in the window.
#[derive(Debug, Clone, PartialEq)]
pub struct UpstreamInfo {
    pub first_ms: i64,
    pub last_ms: i64,
    pub requests: u64,
    pub errors_5xx: u64,
}

/// The verdict for one app over one window.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Report {
    /// Lines read, including ones for other hosts.
    pub lines: u64,
    /// Lines that did not parse.
    pub malformed: u64,
    /// Requests to this app's hostnames since `since_ms`.
    pub requests: u64,
    pub errors_5xx: u64,
    pub by_status: BTreeMap<u16, u64>,
    pub upstreams: BTreeMap<String, UpstreamInfo>,
    /// First instant the serving upstream changed, and from what to what.
    pub flip: Option<Flip>,
}

/// The moment traffic moved from one upstream to another.
#[derive(Debug, Clone, PartialEq)]
pub struct Flip {
    pub at_ms: i64,
    pub from: String,
    pub to: String,
}

/// Builds the report for `hostnames` from lines at or after `since_ms`.
///
/// Hostnames compare case-insensitively after the same normalization the
/// registry uses, because the log records what the client sent.
pub fn verify(hostnames: &[String], since_ms: i64, lines: impl Iterator<Item = String>) -> Report {
    let wanted: BTreeSet<String> = hostnames.iter().map(|h| normalize_hostname(h)).collect();
    let mut report = Report::default();
    // Upstream serving each consecutive request; a change is a flip candidate.
    let mut current: Option<String> = None;

    for line in lines {
        report.lines += 1;
        let Some(entry) = parse_line(&line) else {
            report.malformed += 1;
            continue;
        };
        if entry.ts_ms < since_ms {
            continue;
        }
        if !wanted.contains(&normalize_hostname(&entry.host)) {
            continue;
        }

        report.requests += 1;
        *report.by_status.entry(entry.status).or_default() += 1;
        if (500..600).contains(&entry.status) {
            report.errors_5xx += 1;
        }

        if let Some(up) = entry.upstream.clone() {
            let info = report.upstreams.entry(up.clone()).or_insert(UpstreamInfo {
                first_ms: entry.ts_ms,
                last_ms: entry.ts_ms,
                requests: 0,
                errors_5xx: 0,
            });
            info.last_ms = entry.ts_ms;
            info.requests += 1;
            if (500..600).contains(&entry.status) {
                info.errors_5xx += 1;
            }

            // The first request served by a different upstream is the flip.
            // Later alternations are recorded only as upstream info.
            if current.as_ref() != Some(&up)
                && let Some(from) = current.replace(up.clone())
                && report.flip.is_none()
            {
                report.flip = Some(Flip {
                    at_ms: entry.ts_ms,
                    from,
                    to: up,
                });
            }
        }
    }

    report
}

/// Reads a log file into lines. A missing file is empty, not an error: there
/// may simply have been no traffic yet.
pub fn read_lines(path: &Path) -> std::io::Result<Vec<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text.lines().map(str::to_string).collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(ts: &str, host: &str, status: u16, up: &str) -> String {
        format!("{ts} {host} {status} rt=0.012 up={up} us={status}")
    }

    #[test]
    fn parses_a_complete_line() {
        let entry = parse_line(
            "2026-10-04T21:02:33+00:00 shop.example.com 200 rt=0.012 up=127.0.0.1:9004 us=200",
        )
        .expect("must parse");
        assert_eq!(entry.host, "shop.example.com");
        assert_eq!(entry.status, 200);
        assert_eq!(entry.upstream.as_deref(), Some("127.0.0.1:9004"));
        assert_eq!(entry.upstream_status, Some(200));
        assert!((entry.request_time - 0.012).abs() < 1e-9);
        assert_eq!(
            entry.ts_ms,
            parse_rfc3339("2026-10-04T21:02:33+00:00").unwrap()
        );
    }

    #[test]
    fn parses_a_line_with_no_upstream() {
        // Static files and the 404 catch-all have no upstream.
        let entry = parse_line("2026-10-04T21:02:33Z example.com 404 rt=0.001 up=- us=-")
            .expect("must parse");
        assert_eq!(entry.upstream, None);
        assert_eq!(entry.upstream_status, None);
        assert_eq!(entry.status, 404);
    }

    #[test]
    fn a_multi_upstream_line_is_not_attributed() {
        let entry = parse_line(
            "2026-10-04T21:02:33Z h.com 200 rt=0.1 up=127.0.0.1:9001,127.0.0.1:9002 us=200",
        )
        .expect("must parse");
        assert_eq!(
            entry.upstream, None,
            "cannot attribute one request to two upstreams"
        );
    }

    #[test]
    fn malformed_lines_are_rejected() {
        for bad in [
            "",
            "not a log line",
            "2026-10-04T21:02:33Z only-host",
            "not-a-time h.com 200 rt=0.1 up=127.0.0.1:1 us=200",
            "2026-10-04T21:02:33Z h.com ok rt=0.1 up=127.0.0.1:1 us=200",
        ] {
            assert_eq!(parse_line(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn verify_counts_requests_statuses_and_upstreams() {
        let lines = vec![
            line(
                "2026-10-04T21:00:00+00:00",
                "shop.example.com",
                200,
                "127.0.0.1:9002",
            ),
            line(
                "2026-10-04T21:01:00+00:00",
                "shop.example.com",
                200,
                "127.0.0.1:9002",
            ),
            line(
                "2026-10-04T21:02:00+00:00",
                "other.example.com",
                200,
                "127.0.0.1:9002",
            ),
            line(
                "2026-10-04T21:03:00+00:00",
                "shop.example.com",
                502,
                "127.0.0.1:9002",
            ),
            "garbage line".to_string(),
        ];
        let report = verify(
            &["shop.example.com".to_string()],
            parse_rfc3339("2026-10-04T21:00:00+00:00").unwrap(),
            lines.into_iter(),
        );

        assert_eq!(report.lines, 5);
        assert_eq!(report.malformed, 1);
        assert_eq!(report.requests, 3, "other host excluded");
        assert_eq!(report.errors_5xx, 1);
        assert_eq!(report.by_status.get(&200), Some(&2));
        assert_eq!(report.by_status.get(&502), Some(&1));
        assert_eq!(report.flip, None, "one upstream means no flip");
    }

    #[test]
    fn verify_finds_the_flip_and_ignores_earlier_traffic() {
        let t0 = parse_rfc3339("2026-10-04T21:00:00+00:00").unwrap();
        let lines = vec![
            line(
                "2026-10-04T20:59:00+00:00",
                "example.com",
                200,
                "127.0.0.1:9000",
            ),
            line(
                "2026-10-04T21:00:00+00:00",
                "example.com",
                200,
                "127.0.0.1:9001",
            ),
            line(
                "2026-10-04T21:01:00+00:00",
                "example.com",
                200,
                "127.0.0.1:9004",
            ),
            line(
                "2026-10-04T21:02:00+00:00",
                "example.com",
                200,
                "127.0.0.1:9004",
            ),
        ];
        let report = verify(&["example.com".to_string()], t0, lines.into_iter());

        assert_eq!(report.requests, 3, "the 20:59 line predates the window");
        let flip = report.flip.expect("must find the flip");
        assert_eq!(flip.from, "127.0.0.1:9001");
        assert_eq!(flip.to, "127.0.0.1:9004");
        assert_eq!(
            flip.at_ms,
            parse_rfc3339("2026-10-04T21:01:00+00:00").unwrap()
        );
        assert_eq!(report.upstreams.len(), 2);
        assert_eq!(report.upstreams["127.0.0.1:9004"].requests, 2);
    }

    #[test]
    fn hostnames_match_case_insensitively() {
        let lines = vec![line(
            "2026-10-04T21:00:00+00:00",
            "Shop.Example.COM",
            200,
            "127.0.0.1:9002",
        )];
        let report = verify(&["shop.example.com".to_string()], 0, lines.into_iter());
        assert_eq!(report.requests, 1);
    }

    #[test]
    fn an_empty_log_is_a_clean_report_not_an_error() {
        let report = verify(&["example.com".to_string()], 0, Vec::new().into_iter());
        assert_eq!(report.requests, 0);
        assert_eq!(report.errors_5xx, 0);
        assert_eq!(report.flip, None);
    }

    #[test]
    fn read_lines_tolerates_a_missing_file() {
        let lines = read_lines(Path::new("/nonexistent-xyz/access.log")).unwrap();
        assert!(lines.is_empty());
    }
}
