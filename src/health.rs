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
pub fn gate_container(
    run: &mut Run,
    step_prefix: &str,
    port: u16,
    path: &str,
) -> Result<String, HealthError> {
    let container = discover(run, step_prefix, port)?;
    wait_healthy(run, step_prefix, &container)?;
    probe(run, step_prefix, port, path)?;
    Ok(container)
}

/// Finds the one container publishing `port`.
///
/// Step names say which phase ran them, so the same docker builder serves both
/// the green gate and any later use without the log going ambiguous.
fn discover(run: &mut Run, step_prefix: &str, port: u16) -> Result<String, HealthError> {
    let mut spec = docker::ps_publishing(port);
    spec.name = match step_prefix {
        "health-wait" => "health-wait-discover",
        "probe-front" => "probe-front-discover",
        _ => spec.name,
    };
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
fn probe(run: &mut Run, step_prefix: &str, port: u16, path: &str) -> Result<(), HealthError> {
    let url = format!("http://127.0.0.1:{port}{path}");
    // Intentional initial value, shown if no poll ever completes: the assignment
    // is dead by construction (every use follows an assignment in the same
    // iteration), but spelling it out beats `#[allow]`.
    #[allow(unused_assignments)]
    let mut last = String::from("<no poll completed>");

    for attempt in 1..=PROBE_ATTEMPTS {
        match http_status("127.0.0.1", port, path) {
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
pub fn probe_path(health_url: &str) -> String {
    let after_scheme = health_url.split("://").nth(1).unwrap_or(health_url);
    match after_scheme.find('/') {
        Some(i) => after_scheme[i..].to_string(),
        None => "/".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn http_status_fails_cleanly_on_a_closed_port() {
        // Port 1 is privileged and (almost) never bound: connect refused fast.
        let err = http_status("127.0.0.1", 1, "/").unwrap_err();
        assert!(err.contains("connect"), "{err}");
    }
}
