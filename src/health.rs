//! Health gating: is the new container actually serving?
//!
//! Two phases, in order, because each answers a different question:
//!
//! 1. **Container health.** `docker inspect` reads `.State.Health.Status` until it
//!    says `healthy`. An image with no healthcheck renders `<no value>` here,
//!    which is a hard failure, not a pass: deploying without a health signal is
//!    how traffic meets a half-booted process.
//! 2. **Endpoint probe.** A plain HTTP GET against the probed port, over
//!    `TcpStream` from std. No curl, no extra dependency, and full control of
//!    timeouts. Only the status code is read; bodies are irrelevant to readiness.
//!
//! The URL comes from the registry's `health_url`, but only its path is used.
//! The host is always loopback and the port is always the container being gated,
//! so `http://127.0.0.1:3000/api/healthz` in the registry means "GET
//! /api/healthz on the candidate's port". That keeps one registry value correct
//! for both the live and the green container.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use serde_json::json;
use thiserror::Error;

use crate::docker;
use crate::exec::ExecError;
use crate::trace::Run;

/// How often to re-read container health.
pub const HEALTH_POLL: Duration = Duration::from_secs(2);

/// How long to wait for one container to become healthy.
pub const HEALTH_TIMEOUT: Duration = Duration::from_secs(120);

/// Timeout of a single HTTP attempt.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long to wait between HTTP attempts.
pub const PROBE_INTERVAL: Duration = Duration::from_secs(2);

/// How many HTTP attempts before the probe fails.
pub const PROBE_ATTEMPTS: u32 = 30;

/// Why a gate did not open.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum HealthError {
    #[error("no container publishes port {port}; nothing to gate on")]
    NoContainer { port: u16 },

    #[error("port {port} is published by {names:?}; stop the stale one first")]
    MultipleContainers { port: u16, names: Vec<String> },

    #[error(
        "container {container} has no healthcheck (Health.Status is <no value>); \
         add a HEALTHCHECK to the image or the compose file, then redeploy"
    )]
    NoHealthcheck { container: String },

    #[error(
        "container {container} never became healthy within {timeout_ms} ms (last state: {last})"
    )]
    Unhealthy {
        container: String,
        last: String,
        timeout_ms: u64,
    },

    #[error("GET {url} never returned success within {attempts} attempts: {last}")]
    ProbeFailed {
        url: String,
        attempts: u32,
        last: String,
    },

    #[error("could not inspect container health: {0}")]
    Exec(#[from] ExecError),
}

/// Waits for the container publishing `port` to report healthy, then probes
/// `path` on that port. Returns the container name that was gated.
///
/// The Host header sent to the candidate is `127.0.0.1`. Apps whose framework
/// rejects a foreign Host (Django's `ALLOWED_HOSTS`, Rails' host checks) need
/// [`gate_container_with_host`] so a healthy release is not failed for the wrong
/// reason.
pub fn gate_container(
    run: &mut Run,
    step_prefix: &str,
    port: u16,
    path: &str,
) -> Result<String, HealthError> {
    gate_container_with_host(run, step_prefix, port, path, None)
}

/// [`gate_container`] with an explicit `Host` header for the candidate probe.
///
/// The socket still goes to `127.0.0.1:<port>`; only the header changes. `None`
/// keeps the `Host: 127.0.0.1` fallback.
pub fn gate_container_with_host(
    run: &mut Run,
    step_prefix: &str,
    port: u16,
    path: &str,
    host_header: Option<&str>,
) -> Result<String, HealthError> {
    if run.mode() == crate::trace::RunMode::DryRun {
        return record_dry_run_gate(run, step_prefix, port, path, host_header);
    }
    let container = discover(run, step_prefix, port)?;
    wait_healthy(run, step_prefix, &container)?;
    probe(run, step_prefix, port, path, host_header)?;
    Ok(container)
}

/// Records the four gate phases without probing or spawning anything.
///
/// A dry run has no container and no listener, so the real gate would fail at
/// `docker ps` with "nothing to gate on". The plan is still worth showing:
/// these are the exact steps a live run would take.
fn record_dry_run_gate(
    run: &mut Run,
    step_prefix: &str,
    port: u16,
    path: &str,
    host_header: Option<&str>,
) -> Result<String, HealthError> {
    let names = [
        discover_step_name(step_prefix),
        "health-wait-inspect",
        "health-wait-healthy",
        probe_step_name(step_prefix),
    ];
    for name in names {
        let seq = run.next_seq();
        run.record_step(
            crate::trace::StepRecord::new(seq, name, crate::trace::StepStatus::DryRun, &[])
                .with_detail(json!({
                    "dry_run": true,
                    "port": port,
                    "path": path,
                    "host": host_header,
                })),
        )
        .map_err(|e| HealthError::Exec(ExecError::Trace(e)))?;
    }
    Ok(format!("<dry-run>:{port}"))
}

/// Step name for the container-discovery phase of a gate.
fn discover_step_name(step_prefix: &str) -> &'static str {
    match step_prefix {
        "health-wait" => "health-wait-discover",
        "probe-front" => "probe-front-discover",
        _ => "docker-ps",
    }
}

/// Finds the one container publishing `port`.
///
/// Step names say which phase ran them, so the same docker builder serves both
/// the green gate and any later use without the log going ambiguous.
fn discover(run: &mut Run, step_prefix: &str, port: u16) -> Result<String, HealthError> {
    let mut spec = docker::ps_publishing(port);
    spec.name = discover_step_name(step_prefix);
    let outcome = run.exec(&spec)?;
    let mut names: Vec<String> = outcome
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    names.sort();

    match names.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(HealthError::NoContainer { port }),
        many => Err(HealthError::MultipleContainers {
            port,
            names: many.to_vec(),
        }),
    }
}

/// Polls health status until `healthy`.
fn wait_healthy(run: &mut Run, step_prefix: &str, container: &str) -> Result<(), HealthError> {
    let _ = step_prefix;
    let started = Instant::now();
    // Intentional initial value, shown if no poll ever completes: the assignment
    // is dead by construction (every use follows an assignment in the same
    // iteration), but spelling it out beats `#[allow]`.
    #[allow(unused_assignments)]
    let mut last = String::from("<no poll completed>");

    loop {
        // `last` is assigned before the first poll so the timeout error always
        // has something to report, even if no poll ever completed.
        let mut spec = docker::inspect_health(container);
        spec.name = "health-wait-inspect";
        let outcome = run.exec(&spec)?;
        let fields: Vec<&str> = outcome.stdout_trimmed().split_whitespace().collect();
        // Format is "<health> <state>", e.g. "healthy running" or "<no value> running".
        let (health, state) = match fields.as_slice() {
            [h, s, ..] => (*h, *s),
            [h] => (*h, ""),
            [] => ("", ""),
        };
        last = format!("{health} {state}").trim().to_string();

        if health == "<no"
            || health == "<no value>"
            || outcome.stdout_trimmed().starts_with("<no value>")
        {
            return Err(HealthError::NoHealthcheck {
                container: container.to_string(),
            });
        }
        if health == "healthy" {
            let seq = run.next_seq();
            run.record_step(
                crate::trace::StepRecord::new(
                    seq,
                    "health-wait-healthy",
                    crate::trace::StepStatus::Ok,
                    &[],
                )
                .with_detail(json!({
                    "container": container,
                    "waited_ms": started.elapsed().as_millis() as u64,
                })),
            )
            .map_err(|e| HealthError::Exec(ExecError::Trace(e)))?;
            return Ok(());
        }

        if started.elapsed() >= HEALTH_TIMEOUT {
            return Err(HealthError::Unhealthy {
                container: container.to_string(),
                last: last.clone(),
                timeout_ms: HEALTH_TIMEOUT.as_millis() as u64,
            });
        }
        std::thread::sleep(HEALTH_POLL);
    }
}

/// GETs `path` on `port` until it returns 2xx.
///
/// `host_header` is what the candidate sees in the `Host` header; the connection
/// always goes to `127.0.0.1`. `None` sends `127.0.0.1`, the old behaviour.
fn probe(
    run: &mut Run,
    step_prefix: &str,
    port: u16,
    path: &str,
    host_header: Option<&str>,
) -> Result<(), HealthError> {
    let url = format!("http://127.0.0.1:{port}{path}");
    let header = host_header.unwrap_or("127.0.0.1");
    // Intentional initial value, shown if no poll ever completes: the assignment
    // is dead by construction (every use follows an assignment in the same
    // iteration), but spelling it out beats `#[allow]`.
    #[allow(unused_assignments)]
    let mut last = String::from("<no poll completed>");

    for attempt in 1..=PROBE_ATTEMPTS {
        match http_status_with_host("127.0.0.1", port, path, header) {
            Ok(code) if (200..300).contains(&code) => {
                let seq = run.next_seq();
                run.record_step(
                    crate::trace::StepRecord::new(
                        seq,
                        probe_step_name(step_prefix),
                        crate::trace::StepStatus::Ok,
                        &[],
                    )
                    .with_detail(json!({
                        "url": url,
                        "status": code,
                        "attempt": attempt,
                        "host": header,
                    })),
                )
                .map_err(|e| HealthError::Exec(ExecError::Trace(e)))?;
                return Ok(());
            }
            Ok(code) if (300..400).contains(&code) => {
                last = format!(
                    "HTTP {code} redirect; a health endpoint must answer directly, without one"
                )
            }
            Ok(code) => last = format!("HTTP {code}"),
            Err(e) => last = e,
        }
        if attempt < PROBE_ATTEMPTS {
            std::thread::sleep(PROBE_INTERVAL);
        }
    }

    Err(HealthError::ProbeFailed {
        url,
        attempts: PROBE_ATTEMPTS,
        last,
    })
}

fn probe_step_name(prefix: &str) -> &'static str {
    match prefix {
        "health-wait" => "health-wait-probe",
        "probe-front" => "probe-front",
        _ => "probe",
    }
}

/// Minimal HTTP/1.0 GET. Returns the status code.
///
/// Deliberately small: status line only, no chunked encoding, no redirects, no
/// TLS. Health endpoints on loopback need none of that, and every line of HTTP
/// client not written is a dependency not taken.
pub fn http_status(host: &str, port: u16, path: &str) -> Result<u16, String> {
    http_status_with_host(host, port, path, host)
}

/// Like [`http_status`], but the Host header differs from the address dialed.
///
/// Probing a front port needs this: connect to loopback, ask for the hostname.
pub fn http_status_with_host(
    dial: &str,
    port: u16,
    path: &str,
    host_header: &str,
) -> Result<u16, String> {
    // The path goes onto the request line verbatim. Refuse anything that could
    // split the request before a single byte reaches the socket.
    if !is_safe_probe_path(path) {
        return Err(format!(
            "refusing unsafe health path {path:?}: CR, LF, space or control character"
        ));
    }
    if !is_safe_host_header(host_header) {
        return Err(format!(
            "refusing unsafe Host header {host_header:?}: empty, CR, LF, space or control character"
        ));
    }
    let host = dial;
    let addr: SocketAddr = format!("{host}:{port}")
        .to_socket_addrs()
        .map_err(|e| format!("resolve {host}: {e}"))?
        .next()
        .ok_or_else(|| format!("resolve {host}: no address"))?;

    let mut stream = TcpStream::connect_timeout(&addr, PROBE_TIMEOUT)
        .map_err(|e| format!("connect {addr}: {e}"))?;
    stream
        .set_read_timeout(Some(PROBE_TIMEOUT))
        .map_err(|e| format!("set timeout: {e}"))?;

    let request =
        format!("GET {path} HTTP/1.0\r\nHost: {host_header}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("write: {e}"))?;

    let mut buf = vec![0u8; 4096];
    let mut total = 0usize;
    loop {
        if total >= buf.len() {
            break;
        }
        match stream.read(&mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(e) => return Err(format!("read: {e}")),
        }
        if buf[..total].windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }

    parse_status(&buf[..total])
}

/// Reads the status code from `HTTP/1.x <code> ...`.
pub fn parse_status(response: &[u8]) -> Result<u16, String> {
    let head = std::str::from_utf8(response).map_err(|_| "response is not UTF-8".to_string())?;
    let line = head.lines().next().unwrap_or("").trim();
    let mut parts = line.split_whitespace();
    match (parts.next(), parts.next()) {
        (Some(version), Some(code))
            if version.starts_with("HTTP/")
                && code.len() == 3
                && code.bytes().all(|b| b.is_ascii_digit()) =>
        {
            code.parse().map_err(|_| format!("bad status {code:?}"))
        }
        _ => Err(format!("not an HTTP status line: {line:?}")),
    }
}

/// Splits a registry `health_url` into the path to probe.
///
/// Only the path and query are kept. Host and port always come from the
/// container being gated, so one registry value serves both blue and green.
/// A URL with a query but no path (`http://host?x=1`) keeps the query and gains
/// a leading `/`, because an HTTP request line needs one.
pub fn probe_path(health_url: &str) -> String {
    let after_scheme = health_url.split("://").nth(1).unwrap_or(health_url);
    match after_scheme.find(['/', '?']) {
        // A `?` can only start the query; the path is implicitly `/`.
        Some(i) if after_scheme.as_bytes()[i] == b'?' => format!("/{}", &after_scheme[i..]),
        Some(i) => after_scheme[i..].to_string(),
        None => "/".to_string(),
    }
}

/// True when `path` is safe to place on a raw HTTP request line.
///
/// The path is registry data, but it is written verbatim into a request. A CR,
/// LF, space or other control character would let it split the request and
/// inject a header or a second request, so those are refused.
pub fn is_safe_probe_path(path: &str) -> bool {
    !path.is_empty()
        && !path
            .chars()
            .any(|c| c == '\r' || c == '\n' || c == ' ' || c.is_control())
}

/// True when `host` is safe to place in a `Host:` header.
fn is_safe_host_header(host: &str) -> bool {
    !host.is_empty()
        && !host
            .chars()
            .any(|c| c == '\r' || c == '\n' || c == ' ' || c.is_control())
}

/// [`probe_path`] with the header-injection check applied.
///
/// The API uses this so a hostile `health_url` becomes a 409 rather than a
/// crafted request; the deploy path is additionally guarded inside
/// [`http_status_with_host`].
pub fn checked_probe_path(health_url: &str) -> Result<String, String> {
    let path = probe_path(health_url);
    if !is_safe_probe_path(&path) {
        return Err(format!(
            "health path {path:?} contains CR, LF, a space or a control character"
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::{RunMode, RunStatus, StepStatus, TraceEvent, TraceLog};
    use std::net::TcpListener;

    // ---- pure helpers ----

    #[test]
    fn probe_path_keeps_path_and_query_only() {
        assert_eq!(probe_path("http://127.0.0.1/"), "/");
        assert_eq!(
            probe_path("http://127.0.0.1:3000/api/healthz"),
            "/api/healthz"
        );
        assert_eq!(probe_path("https://host:8443/a/b?x=1&y=2"), "/a/b?x=1&y=2");
        assert_eq!(probe_path("http://127.0.0.1:8001"), "/");
        assert_eq!(probe_path("not-a-url"), "/");
        // A query with no path still needs a leading slash.
        assert_eq!(probe_path("http://host?x=1"), "/?x=1");
        assert_eq!(probe_path("https://host:8443?x=1&y=2"), "/?x=1&y=2");
    }

    #[test]
    fn checked_probe_path_rejects_header_injection() {
        // A CRLF in the path would otherwise split the request and inject a
        // header (or a whole second request).
        let err = checked_probe_path("http://127.0.0.1/\r\nX-Evil: 1").unwrap_err();
        assert!(err.contains("health path"), "{err}");
        assert!(checked_probe_path("http://127.0.0.1/a b").is_err());
        assert!(checked_probe_path("http://127.0.0.1/a\tb").is_err());
        // The ordinary values still pass.
        assert_eq!(checked_probe_path("http://127.0.0.1/up").unwrap(), "/up");
        assert_eq!(checked_probe_path("http://host?x=1").unwrap(), "/?x=1");
    }

    #[test]
    fn http_status_refuses_a_header_injection_path() {
        // Port 1 is never bound: if the guard did not fire first, this would
        // fail with a connect error instead.
        let err = http_status("127.0.0.1", 1, "/\r\nHost: evil").unwrap_err();
        assert!(err.contains("unsafe health path"), "{err}");
    }

    #[test]
    fn parse_status_reads_the_code() {
        assert_eq!(parse_status(b"HTTP/1.1 200 OK\r\n\r\n"), Ok(200));
        assert_eq!(parse_status(b"HTTP/1.0 302 Found\r\nX: y\r\n\r\n"), Ok(302));
        assert_eq!(
            parse_status(b"HTTP/1.1 503 Service Unavailable\r\n"),
            Ok(503)
        );
    }

    #[test]
    fn parse_status_rejects_garbage() {
        assert!(parse_status(b"").is_err());
        assert!(parse_status(b"hello").is_err());
        assert!(parse_status(b"HTTP/1.1 OK\r\n").is_err());
        assert!(parse_status(b"HTTP/1.1 20 OK\r\n").is_err());
        assert!(parse_status(&[0xff, 0xfe]).is_err());
    }

    // ---- live socket tests ----

    /// Serves one canned response, then exits.
    fn serve_once(response: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(response.as_bytes());
        });
        port
    }

    #[test]
    fn http_status_reads_a_live_server() {
        let port = serve_once("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi");
        assert_eq!(http_status("127.0.0.1", port, "/"), Ok(200));
    }

    #[test]
    fn http_status_reports_a_500_as_a_code_not_an_error() {
        // A 500 is an answer, not a transport failure: the caller decides.
        let port = serve_once("HTTP/1.1 500 Broken\r\n\r\n");
        assert_eq!(http_status("127.0.0.1", port, "/healthz"), Ok(500));
    }

    #[test]
    fn http_status_sends_the_path_and_host() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).unwrap_or(0);
            *seen2.lock().unwrap() = buf[..n].to_vec();
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n");
        });

        assert_eq!(http_status("127.0.0.1", port, "/api/healthz"), Ok(200));
        let raw = String::from_utf8(seen.lock().unwrap().clone()).unwrap();
        assert!(raw.starts_with("GET /api/healthz HTTP/1.0\r\n"), "{raw:?}");
        assert!(raw.contains("Host: 127.0.0.1\r\n"), "{raw:?}");
    }

    #[test]
    fn http_status_with_host_dials_loopback_but_sends_the_app_hostname() {
        // The candidate is reachable on 127.0.0.1:<back port>, but frameworks
        // like Django/Rails reject a Host that is not in their allowlist.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).unwrap_or(0);
            *seen2.lock().unwrap() = buf[..n].to_vec();
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n");
        });

        assert_eq!(
            http_status_with_host("127.0.0.1", port, "/up", "app.example.com"),
            Ok(200)
        );
        let raw = String::from_utf8(seen.lock().unwrap().clone()).unwrap();
        assert!(raw.starts_with("GET /up HTTP/1.0\r\n"), "{raw:?}");
        assert!(raw.contains("Host: app.example.com\r\n"), "{raw:?}");
        assert!(
            !raw.contains("Host: 127.0.0.1"),
            "the app hostname replaces the fallback: {raw:?}"
        );
    }

    #[test]
    fn http_status_refuses_an_unsafe_host_header() {
        // Host header injection is the same class as path injection: refuse it
        // before a byte reaches the socket (port 1 is never bound).
        let err =
            http_status_with_host("127.0.0.1", 1, "/", "app.example.com\r\nX-Evil: 1").unwrap_err();
        assert!(err.contains("unsafe Host header"), "{err}");
        assert!(http_status_with_host("127.0.0.1", 1, "/", "").is_err());
    }

    #[test]
    fn a_dry_run_gate_records_the_plan_without_dialing_or_spawning() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = TraceLog::open(dir.path().join("swapdock.jsonl")).unwrap();
        let mut run = Run::start(
            log,
            RunMode::DryRun,
            Some("test".into()),
            &["swapdock".to_string()],
            crate::redact::Redactor::new(),
        )
        .unwrap();

        // Port 1 is not bound and docker is never invoked: the dry-run gate
        // returns before both the probe and the process spawn.
        let container = gate_container(&mut run, "health-wait", 1, "/up").unwrap();
        assert!(container.contains('1'), "{container}");
        run.finish(RunStatus::DryRun).unwrap();

        let read = TraceLog::read(dir.path().join("swapdock.jsonl")).unwrap();
        let steps: Vec<(&str, StepStatus)> = read
            .events
            .iter()
            .filter_map(|e| match e {
                TraceEvent::Step { step, status, .. } if step.starts_with("health-wait") => {
                    Some((step.as_str(), *status))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            steps,
            vec![
                ("health-wait-discover", StepStatus::DryRun),
                ("health-wait-inspect", StepStatus::DryRun),
                ("health-wait-healthy", StepStatus::DryRun),
                ("health-wait-probe", StepStatus::DryRun),
            ],
            "{steps:?}"
        );
    }

    #[test]
    fn http_status_fails_cleanly_on_a_closed_port() {
        // Port 1 is privileged and (almost) never bound: connect refused fast.
        let err = http_status("127.0.0.1", 1, "/").unwrap_err();
        assert!(err.contains("connect"), "{err}");
    }
}
