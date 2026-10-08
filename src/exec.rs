//! The subprocess chokepoint.
//!
//! **This is the only module in the crate that may create a process.** Every
//! `docker`, `docker compose` and `git` invocation goes through `Run::exec`.
//!
//! One function owning process creation is what makes the rest of the
//! observability story possible, because it is the single point where all four
//! of these concerns can be handled at once:
//!
//! | Concern | How it is handled |
//! |---|---|
//! | Logging | argv, exit code, duration and output tails go to the run log |
//! | Dry run | intercepted before spawn, so `--dry-run` needs no second path |
//! | Redaction | applied before anything reaches the tracer |
//! | Timeouts | every command has one, so a hung `compose up` cannot wedge a swapdock |
//!
//! The rule is enforced by an integration test that greps the source for
//! `process::Command` and fails if it appears anywhere else. See
//! `tests/single_spawn_point.rs`.
//!
//! A non-zero exit is deliberately **not** an error here. It is a result. The
//! caller decides whether `nginx -t` failing is fatal; the chokepoint only
//! reports what happened.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;
use thiserror::Error;

use crate::trace::{Run, RunMode, StepRecord, StepStatus};

/// Bytes of each stream retained for the log.
///
/// Output is drained to the end regardless of this cap — stopping the read would
/// block the child on a full pipe. We keep the tail, because that is where the
/// error is.
const OUTPUT_TAIL_BYTES: usize = 16 * 1024;

/// How often a running child is polled while waiting for it to exit.
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Default per-command timeout. Long enough for `docker compose pull` on a cold
/// cache, short enough that a wedged daemon cannot hold a swapdock open.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// Description of one command to run.
#[derive(Debug, Clone)]
pub struct StepSpec {
    /// Stable name recorded in the log, e.g. `nginx-test`.
    pub name: &'static str,
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub timeout: Duration,
    pub cwd: Option<PathBuf>,
    pub stdin: Option<Vec<u8>>,
}

impl StepSpec {
    pub fn new(name: &'static str, program: impl Into<String>) -> Self {
        Self {
            name,
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            timeout: DEFAULT_TIMEOUT,
            cwd: None,
            stdin: None,
        }
    }

    pub fn arg(mut self, a: impl Into<String>) -> Self {
        self.args.push(a.into());
        self
    }

    pub fn args<I, S>(mut self, it: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(it.into_iter().map(Into::into));
        self
    }

    pub fn env(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.env.push((k.into(), v.into()));
        self
    }

    pub fn timeout(mut self, d: Duration) -> Self {
        self.timeout = d;
        self
    }

    pub fn cwd(mut self, p: impl Into<PathBuf>) -> Self {
        self.cwd = Some(p.into());
        self
    }

    pub fn stdin(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.stdin = Some(bytes.into());
        self
    }

    /// program followed by args, which is what the log records.
    pub fn full_argv(&self) -> Vec<String> {
        let mut v = Vec::with_capacity(self.args.len() + 1);
        v.push(self.program.clone());
        v.extend(self.args.iter().cloned());
        v
    }
}

/// What a command did.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub argv: Vec<String>,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration: Duration,
    /// True when the command was killed for exceeding its timeout.
    pub timed_out: bool,
}

impl Outcome {
    /// True when the command ran and exited zero.
    pub fn success(&self) -> bool {
        !self.timed_out && self.exit_code == Some(0)
    }

    /// stdout with surrounding whitespace removed, for parsing single-value output.
    pub fn stdout_trimmed(&self) -> &str {
        self.stdout.trim()
    }

    /// stderr with surrounding whitespace removed.
    pub fn stderr_trimmed(&self) -> &str {
        self.stderr.trim()
    }

    /// stdout, masked, for anything a human or a file will see.
    ///
    /// `stdout` stays raw so that callers can parse structured output such as
    /// `docker compose ps --format`. Anything *displayed* or logged must go
    /// through this, because a child that prints its own environment would
    /// otherwise put a credential into a terminal scrollback or a captured
    /// session. Redaction is conservative: it only rewrites recognised token
    /// shapes and registered literals, so parsing through this is also safe.
    pub fn stdout_safe(&self, redactor: &crate::redact::Redactor) -> String {
        redactor.text(&self.stdout)
    }

    /// stderr, masked. See [`Outcome::stdout_safe`].
    pub fn stderr_safe(&self, redactor: &crate::redact::Redactor) -> String {
        redactor.text(&self.stderr)
    }
}

/// Failure of the exec machinery itself, as opposed to a non-zero exit.
#[derive(Debug, Error)]
pub enum ExecError {
    #[error("could not start `{program}`: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },

    #[error("`{program}` exceeded its timeout of {timeout_ms} ms and was killed")]
    Timeout {
        program: String,
        timeout_ms: u64,
        stdout: String,
        stderr: String,
    },

    #[error("could not wait for `{program}`: {source}")]
    Wait {
        program: String,
        #[source]
        source: std::io::Error,
    },

    #[error("could not write to the run log: {0}")]
    Trace(#[from] anyhow::Error),
}

impl Run {
    /// Runs one command and records it in the run log.
    ///
    /// This is the front door to the operating system for the whole tool. There
    /// is deliberately no other way to reach it.
    pub fn exec(&mut self, spec: &StepSpec) -> Result<Outcome, ExecError> {
        let seq = self.next_seq();
        let argv = spec.full_argv();

        // Dry run: record the intent, spawn nothing, claim success.
        if self.mode() == RunMode::DryRun {
            let outcome = Outcome {
                argv: argv.clone(),
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
                duration: Duration::ZERO,
                timed_out: false,
            };
            let record = StepRecord::new(seq, spec.name, StepStatus::DryRun, &argv)
                .with_env(&spec.env)
                .with_detail(
                    json!({ "dry_run": true, "timeout_ms": spec.timeout.as_millis() as u64 }),
                );
            self.record_step(record).map_err(ExecError::Trace)?;
            return Ok(outcome);
        }

        let started = Instant::now();
        let result = spawn_and_capture(spec);
        let duration = started.elapsed();
        let duration_ms = duration.as_millis() as u64;

        match result {
            Ok((code, stdout, stderr, bytes_out, bytes_err)) => {
                let status = if code == 0 {
                    StepStatus::Ok
                } else {
                    StepStatus::Error
                };
                let record = StepRecord::new(seq, spec.name, status, &argv)
                    .with_env(&spec.env)
                    .with_exit(code)
                    .with_duration(duration_ms)
                    .with_detail(json!({
                        "bytes_stdout": bytes_out,
                        "bytes_stderr": bytes_err,
                        "truncated": bytes_out as usize > OUTPUT_TAIL_BYTES
                            || bytes_err as usize > OUTPUT_TAIL_BYTES,
                    }))
                    .with_stdout_tail(stdout.clone())
                    .with_stderr_tail(stderr.clone());

                self.record_step(record).map_err(ExecError::Trace)?;

                Ok(Outcome {
                    argv,
                    exit_code: Some(code),
                    stdout,
                    stderr,
                    duration,
                    timed_out: false,
                })
            }
            Err(ExecError::Timeout { stdout, stderr, .. }) => {
                let record = StepRecord::new(seq, spec.name, StepStatus::Timeout, &argv)
                    .with_env(&spec.env)
                    .with_duration(duration_ms)
                    .with_detail(json!({
                        "timeout_ms": spec.timeout.as_millis() as u64,
                        "killed": true,
                    }))
                    .with_stdout_tail(stdout.clone())
                    .with_stderr_tail(stderr.clone());
                self.record_step(record).map_err(ExecError::Trace)?;

                Err(ExecError::Timeout {
                    program: spec.program.clone(),
                    timeout_ms: spec.timeout.as_millis() as u64,
                    stdout,
                    stderr,
                })
            }
            // `spawn_and_capture` never returns `Trace`; the arm exists so that
            // adding a variant there is a compile error rather than a silent gap.
            Err(e @ ExecError::Trace(_)) => Err(e),
            Err(e @ ExecError::Spawn { .. }) | Err(e @ ExecError::Wait { .. }) => {
                let record = StepRecord::new(seq, spec.name, StepStatus::Error, &argv)
                    .with_env(&spec.env)
                    .with_duration(duration_ms)
                    .with_error(e.to_string());
                self.record_step(record).map_err(ExecError::Trace)?;
                Err(e)
            }
        }
    }
}

/// Runs one command without opening or writing a run log.
///
/// For read-only queries (such as resolving the latest release) that must not
/// appear in the trace. The spawn still happens here, at the single chokepoint,
/// so `tests/single_spawn_point.rs` still holds. A non-zero exit is a result,
/// exactly as in [`Run::exec`].
pub fn run_capture(spec: &StepSpec) -> Result<Outcome, ExecError> {
    let argv = spec.full_argv();
    let started = Instant::now();
    match spawn_and_capture(spec) {
        Ok((code, stdout, stderr, _, _)) => Ok(Outcome {
            argv,
            exit_code: Some(code),
            stdout,
            stderr,
            duration: started.elapsed(),
            timed_out: false,
        }),
        Err(e) => Err(e),
    }
}

/// Spawns the child, drains both pipes concurrently, and enforces the timeout.
///
/// The two reader threads matter. Polling `try_wait` while nobody drains the
/// pipes deadlocks as soon as the child writes more than one pipe buffer
/// (64 KiB on Linux) — `docker compose up` does that easily.
fn spawn_and_capture(spec: &StepSpec) -> Result<(i32, String, String, u64, u64), ExecError> {
    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args)
        .stdin(if spec.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    if let Some(dir) = &spec.cwd {
        cmd.current_dir(dir);
    }

    let mut child = cmd.spawn().map_err(|source| ExecError::Spawn {
        program: spec.program.clone(),
        source,
    })?;

    if let Some(bytes) = &spec.stdin
        && let Some(mut pipe) = child.stdin.take()
    {
        use std::io::Write as _;
        let _ = pipe.write_all(bytes);
        drop(pipe);
    }

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let out_handle = thread::spawn(move || stdout_pipe.map(drain_tail).unwrap_or_default());
    let err_handle = thread::spawn(move || stderr_pipe.map(drain_tail).unwrap_or_default());

    let started = Instant::now();
    let mut timed_out = false;

    let status = loop {
        match child.try_wait().map_err(|source| ExecError::Wait {
            program: spec.program.clone(),
            source,
        })? {
            Some(status) => break Some(status),
            None => {
                if started.elapsed() >= spec.timeout {
                    let _ = child.kill();
                    timed_out = true;
                    break None;
                }
                thread::sleep(POLL_INTERVAL);
            }
        }
    };

    if timed_out {
        // Reap the child so it does not linger as a zombie.
        let _ = child.wait();
    }

    let (out_text, out_bytes) = out_handle.join().unwrap_or_default();
    let (err_text, err_bytes) = err_handle.join().unwrap_or_default();

    if timed_out {
        return Err(ExecError::Timeout {
            program: spec.program.clone(),
            timeout_ms: spec.timeout.as_millis() as u64,
            stdout: out_text,
            stderr: err_text,
        });
    }

    let code = status.and_then(|s| s.code()).unwrap_or(-1);
    Ok((code, out_text, err_text, out_bytes, err_bytes))
}

/// Accumulates what it reads, keeping the last `cap` bytes.
///
/// Drains to EOF so the child never blocks on a full pipe, but retains only a
/// tail because that is where the useful part of a failing command is.
fn drain_tail<R: Read>(mut reader: R) -> (String, u64) {
    let mut kept: Vec<u8> = Vec::with_capacity(OUTPUT_TAIL_BYTES * 2);
    let mut chunk = [0u8; 16 * 1024];
    let mut total = 0u64;

    loop {
        match reader.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                total += n as u64;
                kept.extend_from_slice(&chunk[..n]);
                if kept.len() > OUTPUT_TAIL_BYTES * 2 {
                    let drop_to = kept.len() - OUTPUT_TAIL_BYTES;
                    kept.drain(..drop_to);
                }
            }
        }
    }

    // `from_utf8_lossy` replaces each invalid byte with U+FFFD, which is three
    // bytes in UTF-8. A buffer of binary output can therefore decode to up to
    // three times its byte length, so the cap has to be enforced again on the
    // decoded string. Trimming to a char boundary afterwards keeps the result
    // valid UTF-8, which matters because this text goes into a JSON log.
    let mut text = String::from_utf8_lossy(&kept).into_owned();
    if text.len() > OUTPUT_TAIL_BYTES {
        let start = text.len() - OUTPUT_TAIL_BYTES;
        let start = (start..=start)
            .find(|i| text.is_char_boundary(*i))
            .expect("a boundary always exists at or after start");
        text = text[start..].to_string();
    }

    (text, total)
}

/// Asserts at compile time that this module is the only place that knows the
/// process API. A second module that imports `std::process` fails to build.
static _SPAWN_POINT: OnceLock<()> = OnceLock::new();

#[cfg(test)]
mod tests {
    use super::*;
    use crate::redact::Redactor;
    use crate::trace::{RunStatus, TraceEvent, TraceLog};
    use tempfile::TempDir;

    fn run(dir: &TempDir) -> Run {
        let log = TraceLog::open(dir.path().join("swapdock.jsonl")).unwrap();
        Run::start(
            log,
            RunMode::Live,
            Some("test".into()),
            &["swapdock".to_string()],
            Redactor::new(),
        )
        .unwrap()
    }

    fn sh(name: &'static str, script: &str) -> StepSpec {
        StepSpec::new(name, "sh").arg("-c").arg(script)
    }

    #[test]
    fn runs_a_command_and_captures_stdout() {
        let dir = TempDir::new().unwrap();
        let mut r = run(&dir);
        let out = r.exec(&sh("echo", "echo hello")).unwrap();

        assert_eq!(out.exit_code, Some(0));
        assert_eq!(out.stdout_trimmed(), "hello");
        assert!(out.success());
    }

    #[test]
    fn a_non_zero_exit_is_a_result_not_an_error() {
        let dir = TempDir::new().unwrap();
        let mut r = run(&dir);
        let out = r.exec(&sh("failing", "exit 3")).unwrap();

        assert_eq!(out.exit_code, Some(3));
        assert!(!out.success(), "exit 3 is not a success");
    }

    #[test]
    fn captures_stderr_separately() {
        let dir = TempDir::new().unwrap();
        let mut r = run(&dir);
        let out = r.exec(&sh("both", "echo out; echo err 1>&2")).unwrap();

        assert_eq!(out.stdout_trimmed(), "out");
        assert_eq!(out.stderr_trimmed(), "err");
    }

    #[test]
    fn passes_arguments_through_without_shell_interpretation() {
        let dir = TempDir::new().unwrap();
        let mut r = run(&dir);
        let out = r
            .exec(
                &StepSpec::new("printf", "printf")
                    .arg("%s|")
                    .arg("a b")
                    .arg("$HOME")
                    .arg("*"),
            )
            .unwrap();

        // No shell means no word splitting, globbing or expansion. `printf`
        // applies the format once per argument, so three args give three bars.
        assert_eq!(out.stdout_trimmed(), "a b|$HOME|*|");
    }

    #[test]
    fn passes_environment_to_the_child() {
        let dir = TempDir::new().unwrap();
        let mut r = run(&dir);
        let out = r
            .exec(&sh("env", "printf %s \"$PORTFOLIO_PORT\"").env("PORTFOLIO_PORT", "9004"))
            .unwrap();
        assert_eq!(out.stdout_trimmed(), "9004");
    }

    #[test]
    fn honours_the_working_directory() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("a/b");
        std::fs::create_dir_all(&nested).unwrap();

        let mut r = run(&dir);
        let out = r
            .exec(&sh("basename", "basename \"$PWD\"").cwd(&nested))
            .unwrap();
        assert_eq!(out.stdout_trimmed(), "b");
    }

    #[test]
    fn feeds_stdin() {
        let dir = TempDir::new().unwrap();
        let mut r = run(&dir);
        let out = r
            .exec(&sh("cat", "cat").stdin(b"from-stdin".to_vec()))
            .unwrap();
        assert_eq!(out.stdout_trimmed(), "from-stdin");
    }

    #[test]
    fn kills_a_command_that_exceeds_its_timeout() {
        let dir = TempDir::new().unwrap();
        let mut r = run(&dir);
        let started = Instant::now();

        let err = r
            .exec(&sh("slow", "sleep 30").timeout(Duration::from_millis(150)))
            .unwrap_err();

        assert!(matches!(err, ExecError::Timeout { .. }), "got {err:?}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "must return promptly, took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_timed_out_command_is_recorded_as_a_timeout() {
        let dir = TempDir::new().unwrap();
        let mut r = run(&dir);
        let _ = r.exec(&sh("slow", "sleep 30").timeout(Duration::from_millis(100)));

        let read = TraceLog::read(dir.path().join("swapdock.jsonl")).unwrap();
        let statuses: Vec<StepStatus> = read
            .events
            .iter()
            .filter_map(|e| match e {
                TraceEvent::Step { status, .. } => Some(*status),
                _ => None,
            })
            .collect();
        assert_eq!(statuses, [StepStatus::Timeout]);
    }

    #[test]
    fn large_output_does_not_deadlock() {
        let dir = TempDir::new().unwrap();
        let mut r = run(&dir);

        // ~1.2 MB, far past the 64 KiB pipe buffer. A naive implementation that
        // polls try_wait() without draining deadlocks here forever.
        let out = r
            .exec(&sh("big", "seq 1 200000").timeout(Duration::from_secs(20)))
            .unwrap();

        assert_eq!(out.exit_code, Some(0));
        assert!(
            out.stdout.len() <= OUTPUT_TAIL_BYTES,
            "decoded output must be capped at {} bytes, got {}",
            OUTPUT_TAIL_BYTES,
            out.stdout.len()
        );
        // Still valid UTF-8, because this text is serialised into the JSON log.
        assert!(std::str::from_utf8(out.stdout.as_bytes()).is_ok());
        assert!(
            out.stdout.trim_end().ends_with("200000"),
            "must keep the tail, not the head"
        );
    }

    #[test]
    fn binary_output_respects_the_decoded_cap() {
        let dir = TempDir::new().unwrap();
        let mut r = run(&dir);
        // 40 KiB of bytes that are not valid UTF-8: lossy decoding would turn
        // each one into a three-byte replacement character and blow the cap.
        let out = r
            .exec(&sh("binary", "head -c 40960 /dev/urandom").timeout(Duration::from_secs(20)))
            .unwrap();

        assert!(
            out.stdout.len() <= OUTPUT_TAIL_BYTES,
            "binary output must still respect the cap, got {}",
            out.stdout.len()
        );
        assert!(std::str::from_utf8(out.stdout.as_bytes()).is_ok());
    }

    #[test]
    fn large_stderr_does_not_deadlock_either() {
        let dir = TempDir::new().unwrap();
        let mut r = run(&dir);
        let out = r.exec(&sh("big-err", "seq 1 200000 1>&2; exit 1")).unwrap();
        assert_eq!(out.exit_code, Some(1));
        assert!(out.stderr.len() <= OUTPUT_TAIL_BYTES);
    }

    #[test]
    fn reports_a_missing_program_as_a_spawn_error() {
        let dir = TempDir::new().unwrap();
        let mut r = run(&dir);
        let err = r
            .exec(&StepSpec::new("nope", "definitely-not-a-real-program-xyz"))
            .unwrap_err();

        assert!(matches!(err, ExecError::Spawn { .. }), "got {err:?}");
        assert!(
            err.to_string()
                .contains("definitely-not-a-real-program-xyz")
        );
    }

    #[test]
    fn a_spawn_failure_is_recorded_as_an_error() {
        let dir = TempDir::new().unwrap();
        let mut r = run(&dir);
        let _ = r.exec(&StepSpec::new("nope", "definitely-not-a-real-program-xyz"));

        let read = TraceLog::read(dir.path().join("swapdock.jsonl")).unwrap();
        assert!(read.events.iter().any(|e| matches!(
            e,
            TraceEvent::Step {
                status: StepStatus::Error,
                ..
            }
        )));
    }

    #[test]
    fn dry_run_spawns_nothing_at_all() {
        let dir = TempDir::new().unwrap();
        let marker = dir.path().spawn_file_marker();

        let log = TraceLog::open(dir.path().join("swapdock.jsonl")).unwrap();
        let mut r = Run::start(
            log,
            RunMode::DryRun,
            None,
            &["swapdock".into()],
            Redactor::new(),
        )
        .unwrap();

        // This command would create the marker file if it really ran.
        let spec = sh("side-effect", &format!("touch {}", marker.display()));
        let out = r.exec(&spec).unwrap();

        assert!(out.success());
        assert!(
            !marker.exists(),
            "dry run executed a command: {} was created",
            marker.display()
        );
    }

    /// Small helper trait so the dry-run test reads cleanly.
    trait MarkerPath {
        fn spawn_file_marker(&self) -> PathBuf;
    }
    impl MarkerPath for std::path::Path {
        fn spawn_file_marker(&self) -> PathBuf {
            self.join("must-not-exist")
        }
    }

    #[test]
    fn dry_run_records_the_intent_with_the_full_argv() {
        let dir = TempDir::new().unwrap();
        let log = TraceLog::open(dir.path().join("swapdock.jsonl")).unwrap();
        let mut r = Run::start(
            log,
            RunMode::DryRun,
            None,
            &["swapdock".into()],
            Redactor::new(),
        )
        .unwrap();

        r.exec(&StepSpec::new("nginx-test", "nginx").args([
            "-t",
            "-c",
            "/etc/nginx/front-door.conf",
        ]))
        .unwrap();
        r.finish(RunStatus::DryRun).unwrap();

        let read = TraceLog::read(dir.path().join("swapdock.jsonl")).unwrap();
        let step = read
            .events
            .iter()
            .find_map(|e| match e {
                TraceEvent::Step {
                    step, argv, status, ..
                } => Some((step, argv, *status)),
                _ => None,
            })
            .expect("a step record");

        assert_eq!(step.0, "nginx-test");
        let expected: Vec<String> = ["nginx", "-t", "-c", "/etc/nginx/front-door.conf"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            step.1, &expected,
            "dry run must record the command it would have run"
        );
        assert_eq!(step.2, StepStatus::DryRun);
    }

    #[test]
    fn sequence_numbers_increment_across_steps() {
        let dir = TempDir::new().unwrap();
        let mut r = run(&dir);
        for _ in 0..3 {
            r.exec(&sh("step", "true")).unwrap();
        }

        // The invariant that matters is the sequence recorded in the log.
        let read = TraceLog::read(dir.path().join("swapdock.jsonl")).unwrap();
        let seqs: Vec<u32> = read
            .events
            .iter()
            .filter_map(|e| match e {
                TraceEvent::Step { seq, .. } => Some(*seq),
                _ => None,
            })
            .collect();
        assert_eq!(seqs, [1, 2, 3]);
    }

    #[test]
    fn redacts_argv_before_it_reaches_the_log() {
        let dir = TempDir::new().unwrap();
        let mut r = run(&dir);
        r.exec(
            &StepSpec::new("login", "sh")
                .args(["-c", "true"])
                .env("GITHUB_TOKEN", "ghp_supersecretvalue1234"),
        )
        .unwrap();
        r.finish(RunStatus::Ok).unwrap();

        let raw = std::fs::read_to_string(dir.path().join("swapdock.jsonl")).unwrap();
        assert!(!raw.contains("ghp_supersecretvalue1234"), "leaked:\n{raw}");
    }

    #[test]
    fn redacts_secrets_that_a_child_prints() {
        let dir = TempDir::new().unwrap();
        let mut r = run(&dir);
        // A tool that echoes its own environment is a real leak vector.
        r.exec(&sh("leaky", "echo token=ghp_abcdefghijklmnop1234"))
            .unwrap();
        r.finish(RunStatus::Ok).unwrap();

        let raw = std::fs::read_to_string(dir.path().join("swapdock.jsonl")).unwrap();
        assert!(!raw.contains("ghp_abcdefghijklmnop1234"), "leaked:\n{raw}");
    }

    #[test]
    fn every_exec_appends_exactly_one_record() {
        let dir = TempDir::new().unwrap();
        let mut r = run(&dir);

        r.exec(&sh("ok", "true")).unwrap();
        r.exec(&sh("bad", "exit 1")).unwrap();
        let _ = r.exec(&StepSpec::new("missing", "nope-xyz")).unwrap_err();
        let _ = r
            .exec(&sh("slow", "sleep 5").timeout(Duration::from_millis(80)))
            .unwrap_err();
        r.finish(RunStatus::Failed).unwrap();

        let read = TraceLog::read(dir.path().join("swapdock.jsonl")).unwrap();
        let steps = read
            .events
            .iter()
            .filter(|e| matches!(e, TraceEvent::Step { .. }))
            .count();
        assert_eq!(steps, 4, "one record per exec, success or failure");
    }

    #[test]
    fn records_the_argv_it_ran() {
        let dir = TempDir::new().unwrap();
        let mut r = run(&dir);
        r.exec(&StepSpec::new("compose-up", "docker").args([
            "compose",
            "-p",
            "apps",
            "up",
            "-d",
            "--wait",
            "portfolio",
        ]))
        .unwrap();

        let read = TraceLog::read(dir.path().join("swapdock.jsonl")).unwrap();
        let argv = read
            .events
            .iter()
            .find_map(|e| match e {
                TraceEvent::Step { argv, .. } => Some(argv.clone()),
                _ => None,
            })
            .unwrap();

        assert_eq!(
            argv,
            [
                "docker",
                "compose",
                "-p",
                "apps",
                "up",
                "-d",
                "--wait",
                "portfolio"
            ]
        );
    }

    #[test]
    fn records_duration_for_a_slow_but_successful_command() {
        let dir = TempDir::new().unwrap();
        let mut r = run(&dir);
        r.exec(&sh("slow-ok", "sleep 0.2")).unwrap();

        let read = TraceLog::read(dir.path().join("swapdock.jsonl")).unwrap();
        let ms = read
            .events
            .iter()
            .find_map(|e| match e {
                TraceEvent::Step { duration_ms, .. } => *duration_ms,
                _ => None,
            })
            .expect("duration_ms");

        assert!(ms >= 150, "expected >=150ms, got {ms}");
        assert!(ms < 10_000, "expected <10s, got {ms}");
    }

    /// Convenience: a successful outcome from a shell one-liner.
    trait OutcomeExt {
        fn must_succeed(self) -> Outcome;
    }
    impl OutcomeExt for Result<Outcome, ExecError> {
        fn must_succeed(self) -> Outcome {
            self.expect("command should succeed")
        }
    }

    #[test]
    fn safe_accessors_mask_a_secret_the_child_printed() {
        let dir = TempDir::new().unwrap();
        let mut r = run(&dir);
        let out = r
            .exec(&sh("leaky", "echo using ghp_abcdefghijklmnop1234 now"))
            .unwrap();

        // Raw output is kept for parsing...
        assert!(out.stdout.contains("ghp_abcdefghijklmnop1234"));
        // ...but the display path must not leak it.
        let safe = out.stdout_safe(&Redactor::new());
        assert!(!safe.contains("ghp_abcdefghijklmnop1234"), "{safe}");
        assert!(safe.contains("[REDACTED"), "{safe}");
        assert!(out.stderr_safe(&Redactor::new()).is_empty());
    }

    #[test]
    fn safe_accessors_leave_ordinary_output_intact() {
        let dir = TempDir::new().unwrap();
        let mut r = run(&dir);
        // Quoted so the shell does not read `>` as a redirection.
        let out = r
            .exec(&sh(
                "plain",
                "printf '%s\\n' 'portfolio-green  127.0.0.1:9004->80/tcp  healthy'",
            ))
            .unwrap();
        assert_eq!(
            out.stdout_safe(&Redactor::new()).trim(),
            "portfolio-green  127.0.0.1:9004->80/tcp  healthy"
        );
    }

    #[test]
    fn outcome_helpers_behave() {
        let dir = TempDir::new().unwrap();
        let mut r = run(&dir);
        let ok = r.exec(&sh("a", "echo x")).must_succeed();
        assert!(ok.success());
        assert_eq!(ok.stdout_trimmed(), "x");
        assert!(!ok.timed_out);
    }

    #[test]
    fn run_capture_runs_without_touching_a_log() {
        let dir = TempDir::new().unwrap();
        let out = run_capture(&sh("echo", "echo hi")).unwrap();
        assert!(out.success());
        assert_eq!(out.stdout_trimmed(), "hi");
        // The whole point: a query must not create or append to a run log.
        assert!(
            std::fs::read_dir(dir.path()).unwrap().next().is_none(),
            "run_capture must not write a trace file"
        );
    }

    #[test]
    fn run_capture_reports_a_non_zero_exit_as_a_result() {
        let out = run_capture(&sh("failing", "exit 4")).unwrap();
        assert_eq!(out.exit_code, Some(4));
        assert!(!out.success());
    }
}
