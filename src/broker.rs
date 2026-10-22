//! Privileged broker.
//!
//! The API can run as a non-root user while this small root process owns the
//! operations that need privilege:
//!
//! * install a rendered nginx config and reload nginx;
//! * execute a narrow allowlist of `docker` commands (the API does not need the
//!   root-equivalent `docker` group).
//!
//! The API sends JSON over a local Unix socket. Authentication is the same
//! `SWAPDOCK_API_TOKEN` already required by the API, plus filesystem permissions
//! on the socket. The socket is `0666` because the install user cannot be
//! assumed to share a group with root; the token is the actual authorization.
//! Keep `/run/swapdock` non-world-writable.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::apply::{self, ApplyError, ApplyOutcome, ApplyPaths};
use crate::exec::{self, ExecError, Outcome, StepSpec};
use crate::trace::{Run, RunMode, RunStatus, StepRecord, StepStatus, TraceLog};

/// Environment variable carrying the broker socket path.
pub const SOCKET_ENV: &str = "SWAPDOCK_BROKER_SOCKET";
const TOKEN_ENV: &str = "SWAPDOCK_BROKER_TOKEN";
const FALLBACK_TOKEN_ENV: &str = "SWAPDOCK_API_TOKEN";
const MAX_CONFIG: usize = 2 * 1024 * 1024;
const MAX_REQUEST_BYTES: u64 = (MAX_CONFIG + 64 * 1024) as u64;
const MAX_DOCKER_TIMEOUT_MS: u64 = 10 * 60 * 1000;

#[derive(Debug, Serialize, Deserialize)]
struct Request {
    token: String,
    #[serde(default)]
    config: Option<String>,
    #[serde(default)]
    argv: Option<Vec<String>>,
    #[serde(default)]
    cwd: Option<PathBuf>,
    #[serde(default)]
    env: Vec<(String, String)>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Response {
    ok: bool,
    outcome: Option<ApplyOutcome>,
    error: Option<String>,
    #[serde(default)]
    exit_code: Option<i32>,
    #[serde(default)]
    stdout: String,
    #[serde(default)]
    stderr: String,
    #[serde(default)]
    duration_ms: u64,
    #[serde(default)]
    timed_out: bool,
}

fn broker_token() -> Option<String> {
    std::env::var(TOKEN_ENV)
        .ok()
        .or_else(|| std::env::var(FALLBACK_TOKEN_ENV).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Sends one rendered config to the broker and records the result in the API's
/// run log. The broker's own trace is separate.
pub fn apply_via_broker(
    run: &mut Run,
    config: &str,
    socket: &Path,
) -> Result<ApplyOutcome, ApplyError> {
    let token = broker_token()
        .ok_or_else(|| ApplyError::Broker("SWAPDOCK_BROKER_TOKEN/API_TOKEN is not set".into()))?;
    apply_via_broker_with_token(run, config, socket, &token)
}

fn apply_via_broker_with_token(
    run: &mut Run,
    config: &str,
    socket: &Path,
    token: &str,
) -> Result<ApplyOutcome, ApplyError> {
    let response = send_request(
        socket,
        &Request {
            token: token.to_string(),
            config: Some(config.to_string()),
            argv: None,
            cwd: None,
            env: Vec::new(),
            timeout_ms: None,
        },
    )
    .map_err(|e| ApplyError::Broker(e.to_string()))?;
    let status = if response.ok {
        StepStatus::Ok
    } else {
        StepStatus::Error
    };
    let seq = run.next_seq();
    run.record_step(
        StepRecord::new(
            seq,
            "broker-apply",
            status,
            &["broker".to_string(), socket.display().to_string()],
        )
        .with_detail(json!({
            "ok": response.ok,
            "error": response.error,
            "reloaded": response.outcome.as_ref().map(|o| o.reloaded),
        })),
    )?;
    match (response.ok, response.outcome) {
        (true, Some(outcome)) => Ok(outcome),
        (_, _) => Err(ApplyError::Broker(
            response
                .error
                .unwrap_or_else(|| "broker returned no outcome".into()),
        )),
    }
}

/// Executes a read-only `docker` command through the broker without opening a
/// trace run. Used by status/inspect/image-list/import paths.
pub fn docker_capture(spec: &StepSpec, socket: &Path) -> Result<Outcome, ExecError> {
    let token = broker_token()
        .ok_or_else(|| ExecError::Broker("SWAPDOCK_BROKER_TOKEN/API_TOKEN is not set".into()))?;
    if spec.stdin.is_some() {
        return Err(ExecError::Broker("docker stdin is not supported".into()));
    }
    let started = Instant::now();
    let argv = spec.full_argv();
    let response = send_request(
        socket,
        &Request {
            token,
            config: None,
            argv: Some(argv.clone()),
            cwd: spec.cwd.clone(),
            env: spec.env.clone(),
            timeout_ms: Some(spec.timeout.as_millis() as u64),
        },
    )
    .map_err(|e| ExecError::Broker(e.to_string()))?;
    if !response.ok {
        return Err(ExecError::Broker(
            response
                .error
                .unwrap_or_else(|| "broker refused docker".into()),
        ));
    }
    Ok(Outcome {
        argv,
        exit_code: response.exit_code,
        stdout: response.stdout,
        stderr: response.stderr,
        duration: started.elapsed(),
        timed_out: response.timed_out,
    })
}

/// Executes a `docker` command through the root broker and records it in the
/// caller's run log. The API user does not need membership in the `docker`
/// group.
pub fn docker_exec(run: &mut Run, spec: &StepSpec, socket: &Path) -> Result<Outcome, ExecError> {
    let token = broker_token()
        .ok_or_else(|| ExecError::Broker("SWAPDOCK_BROKER_TOKEN/API_TOKEN is not set".into()))?;
    docker_exec_with_token(run, spec, socket, &token)
}

fn docker_exec_with_token(
    run: &mut Run,
    spec: &StepSpec,
    socket: &Path,
    token: &str,
) -> Result<Outcome, ExecError> {
    if spec.stdin.is_some() {
        return Err(ExecError::Broker("docker stdin is not supported".into()));
    }
    let started = Instant::now();
    let seq = run.next_seq();
    let argv = spec.full_argv();
    let response = send_request(
        socket,
        &Request {
            token: token.to_string(),
            config: None,
            argv: Some(argv.clone()),
            cwd: spec.cwd.clone(),
            env: spec.env.clone(),
            timeout_ms: Some(spec.timeout.as_millis() as u64),
        },
    );
    let duration = started.elapsed();
    let duration_ms = duration.as_millis() as u64;
    match response {
        Ok(response) if response.ok => {
            let status = if response.exit_code == Some(0) {
                StepStatus::Ok
            } else {
                StepStatus::Error
            };
            run.record_step(
                StepRecord::new(seq, spec.name, status, &argv)
                    .with_env(&spec.env)
                    .with_exit(response.exit_code.unwrap_or(-1))
                    .with_duration(duration_ms)
                    .with_detail(json!({
                        "broker": true,
                        "timed_out": response.timed_out,
                    }))
                    .with_stdout_tail(response.stdout.clone())
                    .with_stderr_tail(response.stderr.clone()),
            )?;
            Ok(Outcome {
                argv,
                exit_code: response.exit_code,
                stdout: response.stdout,
                stderr: response.stderr,
                duration,
                timed_out: response.timed_out,
            })
        }
        Ok(response) => {
            let message = response
                .error
                .unwrap_or_else(|| "broker refused docker".into());
            run.record_step(
                StepRecord::new(seq, spec.name, StepStatus::Error, &argv)
                    .with_env(&spec.env)
                    .with_duration(duration_ms)
                    .with_error(message.clone()),
            )?;
            Err(ExecError::Broker(message))
        }
        Err(e) => {
            run.record_step(
                StepRecord::new(seq, spec.name, StepStatus::Error, &argv)
                    .with_env(&spec.env)
                    .with_duration(duration_ms)
                    .with_error(e.to_string()),
            )?;
            Err(ExecError::Broker(e.to_string()))
        }
    }
}

fn send_request(socket: &Path, request: &Request) -> std::io::Result<Response> {
    let mut stream = UnixStream::connect(socket)?;
    let bytes = serde_json::to_vec(request)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    stream.write_all(&bytes)?;
    stream.write_all(b"\n")?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    serde_json::from_str(line.trim())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Runs the root broker until the process is stopped.
pub fn serve(
    socket: &Path,
    trace: PathBuf,
    paths: ApplyPaths,
    token: String,
    docker_bin: PathBuf,
) -> anyhow::Result<()> {
    if token.trim().is_empty() {
        anyhow::bail!("broker refuses to start without a non-empty token");
    }
    if let Some(parent) = socket.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o666))?;
    tracing::info!(socket = %socket.display(), "swapdock broker listening");
    let paths = Arc::new(paths);
    let trace = Arc::new(trace);
    let token = Arc::new(token);
    let docker_bin = Arc::new(docker_bin);
    let active = Arc::new(AtomicUsize::new(0));
    const MAX_CONNECTIONS: usize = 16;
    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(stream) => stream,
            Err(e) => {
                tracing::warn!("broker accept: {e}");
                continue;
            }
        };
        if active.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
            let _ = stream.write_all(b"{\"ok\":false,\"error\":\"broker busy\"}\n");
            continue;
        }
        active.fetch_add(1, Ordering::Relaxed);
        let paths = Arc::clone(&paths);
        let trace = Arc::clone(&trace);
        let token = Arc::clone(&token);
        let docker_bin = Arc::clone(&docker_bin);
        let active = Arc::clone(&active);
        std::thread::spawn(move || {
            let response = handle(&mut stream, &paths, &trace, &token, &docker_bin);
            match serde_json::to_vec(&response) {
                Ok(bytes) => {
                    if let Err(e) = stream
                        .write_all(&bytes)
                        .and_then(|()| stream.write_all(b"\n"))
                    {
                        tracing::warn!("broker response: {e}");
                    }
                }
                Err(e) => tracing::warn!("broker response encode: {e}"),
            }
            active.fetch_sub(1, Ordering::Relaxed);
        });
    }
    Ok(())
}

fn handle(
    stream: &mut UnixStream,
    paths: &ApplyPaths,
    trace: &Path,
    token: &str,
    docker_bin: &Path,
) -> Response {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let mut line = String::new();
    let mut reader = match stream.try_clone() {
        Ok(stream) => BufReader::new(stream).take(MAX_REQUEST_BYTES),
        Err(e) => return response_error(format!("clone socket: {e}")),
    };
    match reader.read_line(&mut line) {
        Ok(_) if line.len() as u64 <= MAX_REQUEST_BYTES => {}
        Ok(_) => return response_error("request too large"),
        Err(e) => return response_error(format!("read: {e}")),
    }
    let request: Request = match serde_json::from_str(line.trim()) {
        Ok(request) => request,
        Err(e) => return response_error(format!("bad request: {e}")),
    };
    if !crate::auth::constant_time_eq(request.token.as_bytes(), token.as_bytes()) {
        return response_error("unauthorized");
    }
    if let Some(config) = request.config {
        if config.len() > MAX_CONFIG {
            return response_error("config too large");
        }
        let log = match TraceLog::open(trace) {
            Ok(log) => log,
            Err(e) => return response_error(format!("open broker trace: {e}")),
        };
        let mut run = match Run::start(
            log,
            RunMode::Live,
            None,
            &["swapdock".to_string(), "broker".to_string()],
            crate::redact::Redactor::new(),
        ) {
            Ok(run) => run,
            Err(e) => return response_error(format!("start broker run: {e}")),
        };
        let result = apply::apply_local(&mut run, &config, paths);
        let _ = run.finish(if result.is_ok() {
            RunStatus::Ok
        } else {
            RunStatus::Failed
        });
        return match result {
            Ok(outcome) => Response {
                ok: true,
                outcome: Some(outcome),
                ..Response::default()
            },
            Err(e) => response_error(e.to_string()),
        };
    }
    let Some(argv) = request.argv else {
        return response_error("request has neither config nor argv");
    };
    if let Some(problem) = docker_argv_problem(&argv) {
        return response_error(problem);
    }
    if let Some(problem) = docker_env_problem(&request.env) {
        return response_error(problem);
    }
    let timeout_ms = request
        .timeout_ms
        .unwrap_or(exec::DEFAULT_TIMEOUT.as_millis() as u64)
        .min(MAX_DOCKER_TIMEOUT_MS);
    let mut spec = StepSpec::new("broker-docker", docker_bin.display().to_string())
        .args(argv[1..].to_vec())
        .timeout(Duration::from_millis(timeout_ms));
    spec.cwd = request.cwd;
    spec.env = request.env;
    let started = Instant::now();
    match exec::spawn_and_capture(&spec) {
        Ok((code, stdout, stderr, _, _)) => Response {
            ok: true,
            exit_code: Some(code),
            stdout,
            stderr,
            duration_ms: started.elapsed().as_millis() as u64,
            ..Response::default()
        },
        Err(ExecError::Timeout { stdout, stderr, .. }) => Response {
            ok: true,
            exit_code: None,
            stdout,
            stderr,
            timed_out: true,
            duration_ms: started.elapsed().as_millis() as u64,
            ..Response::default()
        },
        Err(e) => response_error(e.to_string()),
    }
}

fn response_error(message: impl Into<String>) -> Response {
    Response {
        ok: false,
        error: Some(message.into()),
        ..Response::default()
    }
}

/// Rejects any `docker` invocation outside the operations the deploy engine
/// actually needs. This is deliberately an allowlist, not a denylist: a new
/// Docker capability must be added here with a test.
fn docker_argv_problem(argv: &[String]) -> Option<String> {
    if argv.len() < 2 {
        return Some("docker request has no subcommand".into());
    }
    if argv[0] != "docker" {
        return Some(format!("broker only runs docker, got {:?}", argv[0]));
    }
    match argv[1].as_str() {
        "ps" => docker_ps_problem(&argv[2..]),
        "inspect" => docker_inspect_problem(&argv[2..]),
        "image" => docker_image_problem(&argv[2..]),
        "images" => docker_images_problem(&argv[2..]),
        "pull" => one_ref(&argv[2..], "pull"),
        "tag" => two_refs(&argv[2..], "tag"),
        "rmi" => two_refs_or_one(&argv[2..], "rmi"),
        "stop" => one_ref(&argv[2..], "stop"),
        "rm" => {
            let mut args = &argv[2..];
            if args.first().is_some_and(|a| a == "-f") {
                args = &args[1..];
            }
            one_ref(args, "rm")
        }
        "compose" => docker_compose_problem(&argv[2..]),
        "build" => docker_build_problem(&argv[2..]),
        "push" => one_ref(&argv[2..], "push"),
        other => Some(format!("docker subcommand {other:?} is not allowed")),
    }
}

fn one_ref(args: &[String], kind: &str) -> Option<String> {
    match args {
        [value] if !value.starts_with('-') && !value.is_empty() => None,
        _ => Some(format!(
            "docker {kind} expects exactly one non-option value"
        )),
    }
}

fn two_refs(args: &[String], kind: &str) -> Option<String> {
    match args {
        [a, b] if !a.starts_with('-') && !b.starts_with('-') && !a.is_empty() && !b.is_empty() => {
            None
        }
        _ => Some(format!(
            "docker {kind} expects exactly two non-option values"
        )),
    }
}

fn two_refs_or_one(args: &[String], kind: &str) -> Option<String> {
    if args.len() == 1 {
        one_ref(args, kind)
    } else {
        two_refs(args, kind)
    }
}

fn docker_ps_problem(args: &[String]) -> Option<String> {
    if args.is_empty() {
        return None;
    }
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--format" | "--filter" => {
                if i + 1 >= args.len() || args[i + 1].starts_with('-') {
                    return Some(format!("docker ps {} needs a value", args[i]));
                }
                i += 2;
            }
            "-q" | "--quiet" | "--no-trunc" => i += 1,
            other => return Some(format!("docker ps option {other:?} is not allowed")),
        }
    }
    None
}

fn docker_inspect_problem(args: &[String]) -> Option<String> {
    if args.is_empty() {
        return Some("docker inspect needs a value".into());
    }
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--format" => {
                if i + 1 >= args.len() {
                    return Some("docker inspect --format needs a value".into());
                }
                i += 2;
            }
            value if !value.starts_with('-') => i += 1,
            other => return Some(format!("docker inspect option {other:?} is not allowed")),
        }
    }
    None
}

fn docker_image_problem(args: &[String]) -> Option<String> {
    match args.first().map(String::as_str) {
        Some("inspect") => docker_inspect_problem(&args[1..]),
        Some("ls") => docker_ps_problem(&args[1..]),
        Some(other) => Some(format!("docker image {other:?} is not allowed")),
        None => Some("docker image needs a subcommand".into()),
    }
}

fn docker_images_problem(args: &[String]) -> Option<String> {
    if args.is_empty() || args[0].starts_with('-') {
        return Some("docker images needs a repository".into());
    }
    docker_ps_problem(&args[1..])
}

fn docker_build_problem(args: &[String]) -> Option<String> {
    // The build engine uses exactly `docker build -t <image> .`; do not expose
    // arbitrary build flags over the socket.
    match args {
        [tag, image, context]
            if tag == "-t" && !image.starts_with('-') && !image.is_empty() && context == "." =>
        {
            None
        }
        _ => Some("docker build must be `build -t <image> .`".into()),
    }
}

fn docker_compose_problem(args: &[String]) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-p" | "-f" | "--project-directory" => {
                if i + 1 >= args.len() || args[i + 1].starts_with('-') {
                    return Some(format!("docker compose {} needs a value", args[i]));
                }
                i += 2;
            }
            value if value.starts_with('-') => {
                return Some(format!("docker compose option {value:?} is not allowed"));
            }
            _ => break,
        }
    }
    match args.get(i).map(String::as_str) {
        Some("pull") => one_ref(&args[i + 1..], "compose pull"),
        Some("stop") => one_ref(&args[i + 1..], "compose stop"),
        Some("rm") => {
            let mut rest = &args[i + 1..];
            if rest.first().is_some_and(|a| a == "-f") {
                rest = &rest[1..];
            }
            one_ref(rest, "compose rm")
        }
        Some("up") => docker_compose_up_problem(&args[i + 1..]),
        Some("down") => docker_compose_down_problem(&args[i + 1..]),
        Some("config") => docker_compose_config_problem(&args[i + 1..]),
        Some(command) => Some(format!("docker compose command {command:?} is not allowed")),
        None => Some("docker compose needs a command".into()),
    }
}

fn docker_compose_up_problem(args: &[String]) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-d" | "--no-deps" | "--no-build" | "--wait" => i += 1,
            value if value.starts_with('-') => {
                return Some(format!("docker compose up option {value:?} is not allowed"));
            }
            _ => {
                return if i + 1 == args.len() {
                    None
                } else {
                    Some("docker compose up takes one service".into())
                };
            }
        }
    }
    Some("docker compose up needs a service".into())
}

fn docker_compose_down_problem(args: &[String]) -> Option<String> {
    for arg in args {
        if !matches!(arg.as_str(), "--volumes" | "-v" | "--remove-orphans") {
            return Some(format!("docker compose down option {arg:?} is not allowed"));
        }
    }
    None
}

fn docker_compose_config_problem(args: &[String]) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--format" => {
                if args.get(i + 1).map(String::as_str) != Some("json") {
                    return Some("docker compose config --format must be json".into());
                }
                i += 2;
            }
            "--quiet" | "--services" => i += 1,
            value if value.starts_with('-') => {
                return Some(format!(
                    "docker compose config option {value:?} is not allowed"
                ));
            }
            _ => i += 1,
        }
    }
    None
}

fn docker_env_problem(env: &[(String, String)]) -> Option<String> {
    for (key, _) in env {
        let allowed = key == "IMAGE" || key.ends_with("_PORT") || key.starts_with("SWAPDOCK_");
        if !allowed
            || key.is_empty()
            || !key
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        {
            return Some(format!("docker environment key {key:?} is not allowed"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn with_server<F: FnOnce(&Path)>(f: F) {
        let dir = tempfile::TempDir::new().unwrap();
        let nginx = dir.path().join("fake-nginx");
        std::fs::write(
            &nginx,
            "#!/bin/sh\ncase \"$1\" in -t|-s) exit 0;; esac\nexit 0\n",
        )
        .unwrap();
        std::fs::set_permissions(&nginx, std::fs::Permissions::from_mode(0o755)).unwrap();
        let main_config = dir.path().join("nginx.conf");
        std::fs::write(&main_config, "events {}\nhttp {}\n").unwrap();
        let paths = ApplyPaths {
            nginx_bin: nginx,
            main_config,
            target: dir.path().join("front-door.conf"),
            pid_file: None,
        };
        let socket = dir.path().join("broker.sock");
        let trace = dir.path().join("broker.jsonl");
        let docker = dir.path().join("fake-docker");
        std::fs::write(&docker, "#!/bin/sh\necho fake-docker\n").unwrap();
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755)).unwrap();
        let token = "0123456789abcdef";
        let served = {
            let socket = socket.clone();
            let trace = trace.clone();
            let paths = paths.clone();
            let docker = docker.clone();
            std::thread::spawn(move || serve(&socket, trace, paths, token.to_string(), docker))
        };
        for _ in 0..50 {
            if socket.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(socket.exists(), "broker socket");
        f(&socket);
        let _ = served;
    }

    #[test]
    fn a_broker_applies_a_config_for_an_unprivileged_client() {
        with_server(|socket| {
            let dir = tempfile::TempDir::new().unwrap();
            let log = TraceLog::open(dir.path().join("client.jsonl")).unwrap();
            let mut run = Run::start(
                log,
                RunMode::Live,
                None,
                &["swapdock".to_string()],
                crate::redact::Redactor::new(),
            )
            .unwrap();
            let text = "# generated\nserver { listen 127.0.0.1:1; }\n";
            let outcome =
                apply_via_broker_with_token(&mut run, text, socket, "0123456789abcdef").unwrap();
            assert!(outcome.reloaded);
        });
    }

    #[test]
    fn a_broker_executes_an_allowlisted_docker_command() {
        with_server(|socket| {
            let dir = tempfile::TempDir::new().unwrap();
            let log = TraceLog::open(dir.path().join("client.jsonl")).unwrap();
            let mut run = Run::start(
                log,
                RunMode::Live,
                None,
                &["swapdock".to_string()],
                crate::redact::Redactor::new(),
            )
            .unwrap();
            let outcome = docker_exec_with_token(
                &mut run,
                &StepSpec::new("docker-test", "docker").args(["ps"]),
                socket,
                "0123456789abcdef",
            )
            .unwrap();
            assert!(outcome.success(), "{outcome:?}");
            assert!(outcome.stdout.contains("fake-docker"), "{}", outcome.stdout);
        });
    }

    #[test]
    fn docker_allowlist_rejects_run_exec_and_shells() {
        for argv in [
            vec!["docker", "run", "alpine"],
            vec!["docker", "exec", "x", "sh"],
            vec!["docker", "compose", "run", "web", "sh"],
            vec!["docker", "build", "--build-arg", "X=1", "."],
        ] {
            let strings: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
            assert!(docker_argv_problem(&strings).is_some(), "{argv:?}");
        }
        assert!(
            docker_argv_problem(
                &[
                    "docker",
                    "compose",
                    "-p",
                    "p",
                    "-f",
                    "c.yml",
                    "up",
                    "-d",
                    "--no-deps",
                    "web"
                ]
                .map(String::from)
            )
            .is_none()
        );
    }

    #[test]
    fn broker_accepts_image_inspect_format_and_compose_services() {
        let image = [
            "docker",
            "image",
            "inspect",
            "--format",
            "{{.Id}}",
            "nginx:tag",
        ]
        .map(String::from);
        assert!(docker_argv_problem(&image).is_none(), "{image:?}");
        let config = [
            "docker",
            "compose",
            "-p",
            "p",
            "-f",
            "c.yml",
            "config",
            "--services",
        ]
        .map(String::from);
        assert!(docker_argv_problem(&config).is_none(), "{config:?}");
    }

    #[test]
    fn docker_environment_is_restricted() {
        assert!(
            docker_env_problem(&[
                ("IMAGE".into(), "x".into()),
                ("WEB_PORT".into(), "9000".into())
            ])
            .is_none()
        );
        assert!(docker_env_problem(&[("LD_PRELOAD".into(), "/tmp/x".into())]).is_some());
    }
}
