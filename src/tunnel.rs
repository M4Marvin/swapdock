//! Reading the tunnel's ingress rules.
//!
//! `validate` needs to know which hostname the tunnel sends to which front port,
//! because that is the join between the registry and the one config file the
//! swapdock tool must not need to restart.
//!
//! ## Not a YAML parser
//!
//! This reads the one shape Cloudflare documents and `cloudflared` writes:
//!
//! ```yaml
//! ingress:
//!   - hostname: m4marvin.com
//!     service: http://localhost:8001
//!   - hostname: chat.m4marvin.com
//!     service: http://localhost:8002
//!   - service: http_status:404        # the catch-all, last
//! ```
//!
//! A line scanner is enough for that and avoids a YAML dependency. The safe
//! failure mode is deliberate: if the `ingress:` key is missing, or no route can
//! be read, the parser reports a warning and returns no routes, and
//! [`crate::validator::validate`] then *skips* the tunnel checks rather than
//! reporting a false pass. A format change must never look like success.

use crate::registry::Problem;
use crate::validator::TunnelRoutes;

/// The result of reading a config: whatever routes were understood, plus
/// anything that looked wrong.
#[derive(Debug, Default)]
pub struct Parsed {
    pub routes: TunnelRoutes,
    pub problems: Vec<Problem>,
}

impl Parsed {
    /// True when at least one route was read.
    pub fn has_routes(&self) -> bool {
        !self.routes.is_empty()
    }
}

/// Extracts `hostname -> front port` from a cloudflared config file.
///
/// Ports come from the origin URL. A rule whose service is not an `http`/`https`
/// origin with an explicit port — the `http_status:404` catch-all, for instance —
/// is not a route and is skipped without complaint.
pub fn parse_ingress(config: &str) -> Parsed {
    let mut out = Parsed::default();

    let lines: Vec<&str> = config.lines().collect();
    let start = match lines
        .iter()
        .position(|l| l.trim_end() == "ingress:" || l.trim_end() == "ingress: []")
    {
        Some(i) => i,
        None => {
            out.problems.push(Problem::warning(
                "no-ingress-section",
                None,
                "no `ingress:` key found, so the tunnel cross-checks were skipped; \
                 this is not a pass",
            ));
            return out;
        }
    };

    // A pending hostname from `- hostname: x`, awaiting its `service:` line.
    let mut pending: Option<String> = None;
    let mut saw_service = false;

    for line in &lines[start + 1..] {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // The ingress list ends at the first key back at column zero.
        if !line.starts_with(char::is_whitespace) && !trimmed.starts_with('-') {
            break;
        }

        let body = trimmed.strip_prefix("- ").unwrap_or(trimmed);

        if let Some(rest) = body.strip_prefix("hostname:") {
            pending = Some(rest.trim().trim_matches(['"', '\'']).to_string());
            continue;
        }

        if let Some(rest) = body.strip_prefix("service:") {
            saw_service = true;
            let value = rest.trim();
            if let Some(host) = pending.take()
                && let Some(port) = origin_port(value)
            {
                out.routes.add(&host, port);
                continue;
            }
            // A catch-all, or an origin we do not model: not a route.
        }
    }

    if !out.has_routes() {
        out.problems.push(Problem::warning(
            "no-ingress-routes",
            None,
            if saw_service {
                "an `ingress:` section was found but no hostname-to-port route could be \
                 read, so the tunnel cross-checks were skipped; this is not a pass"
            } else {
                "the `ingress:` section has no routes, so the tunnel cross-checks were \
                 skipped; this is not a pass"
            },
        ));
    }

    out
}

/// Extracts the port from an origin URL, if it has an explicit one.
///
/// `http://localhost:8001` -> `Some(8001)`
/// `http_status:404`       -> `None`
/// `http://nginx`           -> `None` (implicit port 80; the registry always uses
///                             an explicit port, so this cannot be cross-checked)
fn origin_port(service: &str) -> Option<u16> {
    let value = service.trim().trim_matches(['"', '\'']);
    let rest = value
        .strip_prefix("http://")
        .or_else(|| value.strip_prefix("https://"))?;
    // Strip any path or query before looking for the port.
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let (_, port) = authority.rsplit_once(':')?;
    port.parse().ok()
}

/// Reads and parses a config file.
pub fn read_config(path: &std::path::Path) -> std::io::Result<Parsed> {
    let text = std::fs::read_to_string(path)?;
    Ok(parse_ingress(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real config shape from the swapdock host.
    const REAL: &str = r#"
tunnel: ffd206b8-f950-4b32-bdaa-81e414ee7546
credentials-file: /home/marv/.cloudflared/ffd206b8-f960.json

ingress:
  - hostname: m4marvin.com
    service: http://localhost:8001
  - hostname: chat.m4marvin.com
    service: http://localhost:8002
  - hostname: morphotechdata.com
    service: http://localhost:8012
  - hostname: www.morphotechdata.com
    service: http://localhost:8012
  - service: http_status:404
"#;

    #[test]
    fn reads_the_real_config_shape() {
        let parsed = parse_ingress(REAL);
        assert!(parsed.problems.is_empty(), "{:?}", parsed.problems);
        assert_eq!(parsed.routes.len(), 4);
        assert_eq!(parsed.routes.get("m4marvin.com"), Some(8001));
        assert_eq!(parsed.routes.get("chat.m4marvin.com"), Some(8002));
        assert_eq!(parsed.routes.get("morphotechdata.com"), Some(8012));
        assert_eq!(parsed.routes.get("www.morphotechdata.com"), Some(8012));
    }

    #[test]
    fn the_catch_all_is_not_a_route() {
        let parsed = parse_ingress(REAL);
        assert!(
            !parsed.routes.hostnames().any(|h| h.contains("http_status")),
            "{:?}",
            parsed.routes
        );
    }

    #[test]
    fn hostnames_are_normalised() {
        let parsed = parse_ingress(
            "ingress:\n  - hostname: M4Marvin.COM.\n    service: http://localhost:8001\n",
        );
        assert_eq!(parsed.routes.get("m4marvin.com"), Some(8001));
    }

    #[test]
    fn an_https_origin_and_a_loopback_address_both_parse() {
        let cfg = "ingress:\n  - hostname: a.example.com\n    service: https://127.0.0.1:8443\n";
        assert_eq!(parse_ingress(cfg).routes.get("a.example.com"), Some(8443));
    }

    #[test]
    fn a_path_on_the_origin_is_ignored_when_finding_the_port() {
        let cfg = "ingress:\n  - hostname: a.example.com\n    service: http://localhost:8001/api\n";
        assert_eq!(parse_ingress(cfg).routes.get("a.example.com"), Some(8001));
    }

    #[test]
    fn an_origin_without_a_port_yields_no_route_and_warns() {
        let cfg = "ingress:\n  - hostname: a.example.com\n    service: http://nginx\n";
        let parsed = parse_ingress(cfg);
        assert!(!parsed.has_routes());
        assert!(
            parsed
                .problems
                .iter()
                .any(|p| p.code == "no-ingress-routes"),
            "{:?}",
            parsed.problems
        );
    }

    #[test]
    fn a_missing_ingress_key_warns_rather_than_passing_silently() {
        let parsed = parse_ingress("tunnel: abc\ncredentials-file: /x\n");
        assert!(!parsed.has_routes());
        assert!(
            parsed
                .problems
                .iter()
                .any(|p| p.code == "no-ingress-section"),
            "{:?}",
            parsed.problems
        );
    }

    #[test]
    fn an_empty_ingress_list_warns() {
        let parsed = parse_ingress("ingress: []\n");
        assert!(!parsed.has_routes());
        assert!(!parsed.problems.is_empty());
    }

    #[test]
    fn a_hostname_with_no_service_is_ignored() {
        let cfg = "ingress:\n  - hostname: orphan.example.com\n  - service: http_status:404\n";
        let parsed = parse_ingress(cfg);
        assert!(!parsed.has_routes());
    }

    #[test]
    fn the_list_stops_at_the_next_top_level_key() {
        let cfg = "ingress:\n  - hostname: a.example.com\n    service: http://localhost:8001\nmetrics: 127.0.0.1:20241\n";
        let parsed = parse_ingress(cfg);
        assert_eq!(parsed.routes.len(), 1);
        assert!(parsed.problems.is_empty(), "{:?}", parsed.problems);
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let cfg = "ingress:\n  # the real routes\n\n  - hostname: a.example.com\n    service: http://localhost:8001\n";
        assert_eq!(parse_ingress(cfg).routes.get("a.example.com"), Some(8001));
    }

    #[test]
    fn origin_port_rejects_things_that_are_not_origins() {
        assert_eq!(origin_port("http://localhost:8001"), Some(8001));
        assert_eq!(origin_port("http://localhost:8001/"), Some(8001));
        assert_eq!(origin_port("http_status:404"), None);
        assert_eq!(origin_port("http://localhost"), None);
        assert_eq!(origin_port("http://localhost:notaport"), None);
        assert_eq!(origin_port("unix:///var/run/x.sock"), None);
        assert_eq!(origin_port(""), None);
    }
}
