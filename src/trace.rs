//! The append-only run log.
//!
//! One file, JSON Lines, one object per line. Never rewritten, never rotated
//! in place. That gives three properties the swapdock tool depends on:
//!
//! * **Traceable** — every subprocess lands here with its argv, exit code and
//!   duration, in order, tagged with the run that caused it.
//! * **Resumable** — the last step recorded for a run id is where a crashed
//!   swapdock picks up, so recovery reads the log instead of guessing.
//! * **Observable** — `grep <run_id>` reconstructs a run; `grep '"status":"error"'`
//!   finds every failure across all history.
//!
//! Damage tolerance matters here. A machine that loses power mid-write leaves a
//! partial final line. The reader skips unparseable lines and reports a count
//! rather than refusing to start, because refusing to start is the one
//! behaviour that turns a crash into an outage.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::id::RunId;
use crate::redact::Redactor;
use crate::time::Timestamp;

/// Outcome of one step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    /// Ran and exited zero.
    Ok,
    /// Ran and exited non-zero, or failed to start.
    Error,
    /// Ran past its timeout and was killed.
    Timeout,
    /// Not run, because the tool is in dry-run mode.
    DryRun,
}

impl StepStatus {
    /// True for statuses that mean the step did not succeed.
    pub fn is_failure(self) -> bool {
        matches!(self, StepStatus::Error | StepStatus::Timeout)
    }
}

/// How the run was invoked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunMode {
    /// Commands really execute.
    Live,
    /// Nothing is spawned; only the plan and the trace are written.
    DryRun,
}

/// Terminal state of a whole run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Ok,
    Failed,
    DryRun,
    /// Still in flight: the opening process is alive on this boot.
    Running,
    /// The process died before writing `run_end`.
    Interrupted,
}

/// One line of the log.
///
/// `event` is the tag, so lines are readable and greppable without parsing:
/// `{"event":"step","step":"nginx-test",...}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum TraceEvent {
    RunStart {
        run_id: String,
        ts: String,
        ts_ms: i64,
        version: String,
        mode: RunMode,
        app: Option<String>,
        /// Who started the run: a CLI user, or the API job source. Older logs
        /// have no field and deserialize to `None`.
        #[serde(default)]
        actor: Option<String>,
        /// Process that opened the run. With `boot` this distinguishes a run
        /// that is still in flight from one whose process died, so `runs` can
        /// say `interrupted` instead of leaving it "live" forever.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pid: Option<u32>,
        /// Kernel boot id, so a recycled pid after a reboot is not mistaken for
        /// the original process.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        boot: Option<String>,
        argv: Vec<String>,
    },
    Step {
        run_id: String,
        ts: String,
        ts_ms: i64,
        app: Option<String>,
        seq: u32,
        step: String,
        status: StepStatus,
        argv: Vec<String>,
        #[serde(skip_serializing_if = "Vec::is_empty", default)]
        env: Vec<(String, String)>,
        #[serde(skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        stdout_tail: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        stderr_tail: Option<String>,
    },
    RunEnd {
        run_id: String,
        ts: String,
        ts_ms: i64,
        status: RunStatus,
        steps: u32,
    },
}

/// Where a run had got to.
#[derive(Debug, Clone, PartialEq)]
pub struct ResumePoint {
    pub run_id: RunId,
    /// Sequence number of the last recorded step.
    pub last_seq: u32,
    /// Name of the last recorded step.
    pub last_step: String,
    /// Status of the last recorded step.
    pub last_status: StepStatus,
    /// Steps that did not succeed.
    pub non_ok_steps: u32,
    /// Terminal status, when the run wrote its `run_end` record.
    ///
    /// Distinguishes "finished" from "died mid-way", which is the whole point
    /// of asking where a run stopped.
    pub ended: Option<RunStatus>,
    /// Process that opened the run, and its boot id, for the interrupted check.
    pub pid: Option<u32>,
    pub boot: Option<String>,
}

impl ResumePoint {
    /// Lifecycle state, resolving an unfinished run to `running` or `interrupted`.
    pub fn state(&self) -> RunStatus {
        run_state(self.pid, self.boot.as_deref(), self.ended)
    }
}

/// Kernel boot id, cached for the life of the process.
///
/// Reading it per run would be wasteful, and on a host without the file the
/// value is simply absent: a run is then judged by its pid alone.
fn current_boot_id() -> Option<String> {
    static BOOT: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    BOOT.get_or_init(|| {
        std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
    .clone()
}

fn pid_is_alive(pid: u32) -> bool {
    pid > 0 && Path::new(&format!("/proc/{pid}")).exists()
}

/// What a reader should say about a run.
///
/// A run with a `run_end` record is finished. Without one, the process that
/// opened it decides: alive on the same boot means still running; anything else
/// means the process died mid-run and nothing will ever close it. That is the
/// difference between "Running for 3 days" and "interrupted", and the reason
/// the pid and boot id are recorded.
pub fn run_state(pid: Option<u32>, boot: Option<&str>, ended: Option<RunStatus>) -> RunStatus {
    if let Some(status) = ended {
        return status;
    }
    let same_boot = match (boot, current_boot_id()) {
        (Some(recorded), Some(current)) => recorded == current,
        // Without a recorded boot id the pid is the only evidence available.
        _ => true,
    };
    match pid {
        Some(pid) if same_boot && pid_is_alive(pid) => RunStatus::Running,
        _ => RunStatus::Interrupted,
    }
}

/// A log file opened for appending.
#[derive(Debug)]
pub struct TraceLog {
    path: PathBuf,
    file: File,
}

/// Names the log in an io error, keeping its kind.
///
/// A bare "Permission denied" leaves the operator guessing which file to fix;
/// the path is the one thing such an error must say.
fn at(path: &Path, e: std::io::Error) -> std::io::Error {
    std::io::Error::new(e.kind(), format!("{}: {e}", path.display()))
}

impl TraceLog {
    /// Opens (or creates) the log for appending.
    ///
    /// New files are created group-readable (`0640`), not world-readable.
    /// Secrets are redacted before anything is written, but the log still
    /// contains argv, paths and container details; it is operator data, not
    /// public data. Existing files keep their mode: opening never rechmods.
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| at(parent, e))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o640)
            .open(&path)
            .map_err(|e| at(&path, e))?;
        // `.mode(0o640)` only applies on creation. A binary-only upgrade can
        // inherit an old 0664 log; tighten it when the current user owns it.
        if let Ok(metadata) = file.metadata() {
            let mut permissions = metadata.permissions();
            if permissions.mode() & 0o077 != 0 {
                permissions.set_mode(0o640);
                let _ = file.set_permissions(permissions);
            }
        }
        Ok(Self { path, file })
    }

    /// Where the log is being written.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends one event as a single line.
    ///
    /// The line is written with one `write_all` on a file opened `O_APPEND`, so
    /// concurrent writers cannot interleave within a line. It is flushed to the
    /// kernel but not `fsync`ed, which means a process crash preserves the line
    /// and a power cut may lose the last few.
    pub fn append(&mut self, event: &TraceEvent) -> std::io::Result<()> {
        let mut line = serde_json::to_string(event)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        line.push('\n');
        self.file.write_all(line.as_bytes())?;
        self.file.flush()
    }

    /// Reads every event back, skipping damaged lines.
    pub fn read(path: impl AsRef<Path>) -> std::io::Result<Read> {
        let path = path.as_ref();
        let file = match File::open(path) {
            Ok(f) => f,
            // A missing log is not an error: it just means no runs yet.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Read::default());
            }
            Err(e) => return Err(at(path, e)),
        };

        let mut events = Vec::new();
        let mut damaged = 0usize;

        for line in BufReader::new(file).lines() {
            let Ok(line) = line else {
                // Undecodable bytes: almost certainly a torn write.
                damaged += 1;
                continue;
            };
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<TraceEvent>(&line) {
                Ok(ev) => events.push(ev),
                Err(_) => damaged += 1,
            }
        }

        Ok(Read { events, damaged })
    }

    /// Finds where a run stopped, for `swapdock resume`.
    pub fn resume_point(
        path: impl AsRef<Path>,
        run_id: &RunId,
    ) -> std::io::Result<Option<ResumePoint>> {
        let read = Self::read(path)?;
        let mut found: Option<ResumePoint> = None;
        let mut non_ok = 0u32;
        let mut ended: Option<RunStatus> = None;
        let mut pid: Option<u32> = None;
        let mut boot: Option<String> = None;

        for ev in &read.events {
            match ev {
                TraceEvent::RunStart {
                    run_id: id,
                    pid: run_pid,
                    boot: run_boot,
                    ..
                } if id == run_id.as_str() => {
                    pid = *run_pid;
                    boot = run_boot.clone();
                }
                TraceEvent::Step {
                    run_id: id,
                    seq,
                    step,
                    status,
                    ..
                } if id == run_id.as_str() => {
                    if status.is_failure() {
                        non_ok += 1;
                    }
                    found = Some(ResumePoint {
                        run_id: run_id.clone(),
                        last_seq: *seq,
                        last_step: step.clone(),
                        last_status: *status,
                        non_ok_steps: non_ok,
                        ended: None,
                        pid,
                        boot: boot.clone(),
                    });
                }
                TraceEvent::RunEnd {
                    run_id: id, status, ..
                } if id == run_id.as_str() => {
                    ended = Some(*status);
                }
                _ => {}
            }
        }

        // A run that wrote its final record did not stop early, whatever the
        // status of its last step was.
        if let (Some(point), Some(status)) = (found.as_mut(), ended) {
            point.ended = Some(status);
        }

        Ok(found)
    }
}

/// Result of reading a log back.
#[derive(Debug, Default)]
pub struct Read {
    pub events: Vec<TraceEvent>,
    /// Lines that did not parse, almost always from a torn write.
    pub damaged: usize,
}

/// Collects events for one run and writes them to the log.
#[derive(Debug)]
pub struct Run {
    id: RunId,
    app: Option<String>,
    mode: RunMode,
    redactor: Redactor,
    log: TraceLog,
    seq: u32,
}

impl Run {
    /// Starts a run, writing the `run_start` record immediately.
    ///
    /// Writing the header first means a run that dies during its very first step
    /// is still discoverable by id.
    pub fn start(
        log: TraceLog,
        mode: RunMode,
        app: Option<String>,
        argv: &[String],
        redactor: Redactor,
    ) -> anyhow::Result<Self> {
        let actor = std::env::var("SWAPDOCK_ACTOR")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                std::env::var("SUDO_USER")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
            })
            .or_else(|| {
                std::env::var("USER")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
            });
        Self::start_as(log, mode, app, argv, redactor, actor)
    }

    /// Starts a run with an explicit actor (used by API jobs).
    pub fn start_as(
        log: TraceLog,
        mode: RunMode,
        app: Option<String>,
        argv: &[String],
        redactor: Redactor,
        actor: Option<String>,
    ) -> anyhow::Result<Self> {
        let id = RunId::generate();
        let now = Timestamp::now();

        let mut run = Self {
            id: id.clone(),
            app,
            mode,
            redactor,
            log,
            seq: 0,
        };

        run.log.append(&TraceEvent::RunStart {
            run_id: id.to_string(),
            ts: now.to_rfc3339(),
            ts_ms: now.epoch_millis(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            mode,
            app: run.app.clone(),
            actor,
            pid: Some(std::process::id()),
            boot: current_boot_id(),
            argv: run.redactor.argv(argv),
        })?;

        Ok(run)
    }

    pub fn id(&self) -> &RunId {
        &self.id
    }

    pub fn mode(&self) -> RunMode {
        self.mode
    }

    /// The redactor, so callers can register a secret they have just read.
    pub fn redactor_mut(&mut self) -> &mut Redactor {
        &mut self.redactor
    }

    /// Read-only access, for masking output that is about to be displayed.
    pub fn redactor(&self) -> &Redactor {
        &self.redactor
    }

    /// Takes the next sequence number.
    pub fn next_seq(&mut self) -> u32 {
        self.seq += 1;
        self.seq
    }

    /// Appends a `run_end` record.
    pub fn finish(mut self, status: RunStatus) -> anyhow::Result<()> {
        let now = Timestamp::now();
        self.log.append(&TraceEvent::RunEnd {
            run_id: self.id.to_string(),
            ts: now.to_rfc3339(),
            ts_ms: now.epoch_millis(),
            status,
            steps: self.seq,
        })?;
        Ok(())
    }

    /// Appends a `step` record. Redaction happens here, at the last moment
    /// before the data can reach a file.
    pub fn record_step(&mut self, step: StepRecord<'_>) -> anyhow::Result<()> {
        let now = Timestamp::now();
        self.log.append(&TraceEvent::Step {
            run_id: self.id.to_string(),
            ts: now.to_rfc3339(),
            ts_ms: now.epoch_millis(),
            app: self.app.clone(),
            seq: step.seq,
            step: step.name.to_string(),
            status: step.status,
            argv: self.redactor.argv(step.argv),
            env: self.redactor.env(step.env),
            exit_code: step.exit_code,
            duration_ms: step.duration_ms,
            detail: step.detail.as_ref().map(|d| self.redactor.json(d)),
            error: step.error,
            stdout_tail: step.stdout_tail.map(|s| self.redactor.text(&s)),
            stderr_tail: step.stderr_tail.map(|s| self.redactor.text(&s)),
        })?;
        Ok(())
    }
}

/// Data for one step record. Built by `exec`, consumed by `Run::record_step`.
#[derive(Debug)]
pub struct StepRecord<'a> {
    pub seq: u32,
    pub name: &'a str,
    pub status: StepStatus,
    pub argv: &'a [String],
    pub env: &'a [(String, String)],
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    pub detail: Option<serde_json::Value>,
    pub error: Option<String>,
    pub stdout_tail: Option<String>,
    pub stderr_tail: Option<String>,
}

impl<'a> StepRecord<'a> {
    /// A minimal record; the exec chokepoint fills in the rest.
    pub fn new(seq: u32, name: &'a str, status: StepStatus, argv: &'a [String]) -> Self {
        Self {
            seq,
            name,
            status,
            argv,
            env: &[],
            exit_code: None,
            duration_ms: None,
            detail: None,
            error: None,
            stdout_tail: None,
            stderr_tail: None,
        }
    }

    pub fn with_env(mut self, env: &'a [(String, String)]) -> Self {
        self.env = env;
        self
    }

    pub fn with_exit(mut self, code: i32) -> Self {
        self.exit_code = Some(code);
        self
    }

    pub fn with_duration(mut self, ms: u64) -> Self {
        self.duration_ms = Some(ms);
        self
    }

    pub fn with_detail(mut self, detail: serde_json::Value) -> Self {
        self.detail = Some(detail);
        self
    }

    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        self.error = Some(error.into());
        self
    }

    pub fn with_stdout_tail(mut self, tail: String) -> Self {
        self.stdout_tail = Some(tail);
        self
    }

    pub fn with_stderr_tail(mut self, tail: String) -> Self {
        self.stderr_tail = Some(tail);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn argv_of(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn start_run(dir: &TempDir, mode: RunMode) -> (Run, PathBuf) {
        let path = dir.path().join("nested/swapdock.jsonl");
        let log = TraceLog::open(&path).expect("open log");
        let run = Run::start(
            log,
            mode,
            Some("portfolio".into()),
            &argv_of(&["swapdock", "up", "portfolio"]),
            Redactor::new(),
        )
        .expect("start run");
        (run, path)
    }

    #[test]
    fn a_run_can_record_an_explicit_actor() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("t.jsonl");
        let log = TraceLog::open(&path).unwrap();
        let run = Run::start_as(
            log,
            RunMode::Live,
            None,
            &argv_of(&["swapdock", "api"]),
            Redactor::new(),
            Some("api".into()),
        )
        .unwrap();
        run.finish(RunStatus::Ok).unwrap();
        let read = TraceLog::read(&path).unwrap();
        assert!(read.events.iter().any(|event| matches!(
            event,
            TraceEvent::RunStart { actor, .. } if actor.as_deref() == Some("api")
        )));
    }

    #[test]
    fn io_errors_name_the_log_path() {
        // A bare "Permission denied" is what made the host's log problem take
        // an afternoon to find; the message must name the file.
        let path = Path::new("/srv/swapdock/logs/swapdock.jsonl");
        let err = at(
            path,
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        );
        assert!(
            err.to_string()
                .contains("/srv/swapdock/logs/swapdock.jsonl"),
            "{err}"
        );
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn writes_one_json_object_per_line() {
        let dir = TempDir::new().unwrap();
        let (mut run, path) = start_run(&dir, RunMode::Live);
        let argv = argv_of(&["docker", "ps"]);
        let rec = StepRecord::new(run.next_seq(), "docker-ps", StepStatus::Ok, &argv);
        run.record_step(rec).unwrap();
        run.finish(RunStatus::Ok).unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = raw.lines().collect();
        assert_eq!(lines.len(), 3, "run_start + step + run_end: {raw}");

        for line in &lines {
            serde_json::from_str::<TraceEvent>(line)
                .unwrap_or_else(|e| panic!("line must be valid JSON: {line} ({e})"));
        }
    }

    #[test]
    fn new_log_files_are_group_readable() {
        // The operator runs show/runs unprivileged against a log that root
        // writes. Redaction (not file mode) is what keeps secrets out of it.
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("deploy.jsonl");
        let _ = TraceLog::open(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o640,
            "new log files must be group-readable, got {mode:o}"
        );
    }

    #[test]
    fn opening_tightens_an_old_world_readable_log() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("deploy.jsonl");
        std::fs::write(&path, "").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o664)).unwrap();
        let _ = TraceLog::open(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640, "an inherited 0664 log must be tightened");
    }

    #[test]
    fn opening_never_rechmods_an_existing_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("deploy.jsonl");
        std::fs::write(&path, "").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let _ = TraceLog::open(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "existing modes must be left alone");
    }

    #[test]
    fn creates_missing_parent_directories() {
        let dir = TempDir::new().unwrap();
        let (_, path) = start_run(&dir, RunMode::Live);
        assert!(path.exists(), "must create parent dirs: {path:?}");
    }

    #[test]
    fn reads_back_what_it_wrote() {
        let dir = TempDir::new().unwrap();
        let (mut run, path) = start_run(&dir, RunMode::Live);
        for (seq, name) in [(1u32, "pull"), (2, "health-wait"), (3, "nginx-reload")] {
            let argv = argv_of(&["docker", "compose", "pull"]);
            run.record_step(StepRecord::new(seq, name, StepStatus::Ok, &argv))
                .unwrap();
        }
        run.finish(RunStatus::Ok).unwrap();

        let read = TraceLog::read(&path).unwrap();
        assert_eq!(read.damaged, 0);
        assert_eq!(read.events.len(), 5);

        let names: Vec<&str> = read
            .events
            .iter()
            .filter_map(|e| match e {
                TraceEvent::Step { step, .. } => Some(step.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(names, ["pull", "health-wait", "nginx-reload"]);
    }

    #[test]
    fn appends_rather_than_truncating() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("swapdock.jsonl");

        for _ in 0..3 {
            let log = TraceLog::open(&path).unwrap();
            let run = Run::start(
                log,
                RunMode::Live,
                None,
                &argv_of(&["swapdock"]),
                Redactor::new(),
            )
            .unwrap();
            run.finish(RunStatus::Ok).unwrap();
        }

        let read = TraceLog::read(&path).unwrap();
        assert_eq!(read.events.len(), 6, "3 runs x (start + end)");
    }

    #[test]
    fn skips_a_torn_final_line_and_reports_it() {
        let dir = TempDir::new().unwrap();

        let (mut run, path) = start_run(&dir, RunMode::Live);
        let argv = argv_of(&["docker", "ps"]);
        run.record_step(StepRecord::new(1, "docker-ps", StepStatus::Ok, &argv))
            .unwrap();
        run.finish(RunStatus::Ok).unwrap();

        // Simulate power loss halfway through a write.
        let mut raw = std::fs::read_to_string(&path).unwrap();
        raw.push_str("{\"event\":\"step\",\"run_id\":\"abc\",\"ts\":\"2026");
        std::fs::write(&path, raw).unwrap();

        let read = TraceLog::read(&path).unwrap();
        assert_eq!(read.damaged, 1, "must count the torn line");
        assert_eq!(read.events.len(), 3, "must still return the good lines");
    }

    #[test]
    fn a_missing_log_reads_as_empty() {
        let dir = TempDir::new().unwrap();
        let read = TraceLog::read(dir.path().join("nope.jsonl")).unwrap();
        assert!(read.events.is_empty());
        assert_eq!(read.damaged, 0);
    }

    #[test]
    fn resume_point_reports_the_last_step_of_that_run() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("swapdock.jsonl");

        let mut first = Run::start(
            TraceLog::open(&path).unwrap(),
            RunMode::Live,
            Some("portfolio".into()),
            &argv_of(&["swapdock"]),
            Redactor::new(),
        )
        .unwrap();
        let argv = argv_of(&["docker", "compose", "pull"]);
        first
            .record_step(StepRecord::new(1, "pull", StepStatus::Ok, &argv))
            .unwrap();
        first
            .record_step(StepRecord::new(2, "green-start", StepStatus::Ok, &argv))
            .unwrap();
        // No finish(): this run died mid-way, which is the case resume exists for.
        let first_id = first.id().clone();

        // A second, unrelated run must not confuse the lookup.
        let second = Run::start(
            TraceLog::open(&path).unwrap(),
            RunMode::Live,
            Some("charts".into()),
            &argv_of(&["swapdock"]),
            Redactor::new(),
        )
        .unwrap();
        let second_id = second.id().clone();
        second.finish(RunStatus::Ok).unwrap();

        let point = TraceLog::resume_point(&path, &first_id)
            .unwrap()
            .expect("must find the point");
        assert_eq!(point.last_seq, 2);
        assert_eq!(point.last_step, "green-start");
        assert_eq!(point.last_status, StepStatus::Ok);
        assert_eq!(point.non_ok_steps, 0);
        assert_eq!(point.ended, None, "this run never wrote a final record");

        let other = TraceLog::resume_point(&path, &second_id).unwrap();
        assert!(other.is_none(), "a run with no steps has no point");
    }

    #[test]
    fn a_completed_run_reports_its_terminal_status() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("swapdock.jsonl");

        let mut run = Run::start(
            TraceLog::open(&path).unwrap(),
            RunMode::Live,
            None,
            &argv_of(&["swapdock"]),
            Redactor::new(),
        )
        .unwrap();
        let id = run.id().clone();
        // A non-zero exit is a fact about the step, not necessarily about the run.
        run.record_step(StepRecord::new(
            1,
            "probe",
            StepStatus::Error,
            &argv_of(&["false"]),
        ))
        .unwrap();
        run.finish(RunStatus::Ok).unwrap();

        let point = TraceLog::resume_point(&path, &id).unwrap().unwrap();
        assert_eq!(point.ended, Some(RunStatus::Ok), "the run finished");
        assert_eq!(point.non_ok_steps, 1, "one step did not succeed");
    }

    #[test]
    fn no_secret_reaches_the_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("swapdock.jsonl");
        let mut redactor = Redactor::new();
        redactor.register("ghp_abcdefghijklmnop1234567890");

        let mut run = Run::start(
            TraceLog::open(&path).unwrap(),
            RunMode::Live,
            None,
            &argv_of(&["swapdock", "login"]),
            redactor,
        )
        .unwrap();

        let argv = argv_of(&["docker", "login", "-u", "marv", "--password-stdin"]);
        let env = vec![
            (
                "GITHUB_TOKEN".to_string(),
                "ghp_abcdefghijklmnop1234567890".to_string(),
            ),
            ("PORTFOLIO_PORT".to_string(), "9004".to_string()),
        ];
        let rec = StepRecord::new(1, "registry-login", StepStatus::Ok, &argv)
            .with_env(&env)
            .with_stdout_tail("using ghp_abcdefghijklmnop1234567890 to authenticate".into())
            .with_stderr_tail("push access to ghp_abcdefghijklmnop1234567890 denied".into());
        run.record_step(rec).unwrap();
        run.finish(RunStatus::Ok).unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(
            !raw.contains("ghp_abcdefghijklmnop1234567890"),
            "the token leaked into the log:\n{raw}"
        );
        assert!(
            raw.contains("REDACTED"),
            "expected a redaction marker:\n{raw}"
        );
        assert!(raw.contains("9004"), "safe values must survive:\n{raw}");
    }

    #[test]
    fn step_status_reports_failure() {
        assert!(StepStatus::Error.is_failure());
        assert!(StepStatus::Timeout.is_failure());
        assert!(!StepStatus::Ok.is_failure());
        assert!(!StepStatus::DryRun.is_failure());
    }

    #[test]
    fn timestamps_in_the_log_sort_as_times() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("swapdock.jsonl");
        let (mut run, _) = start_run(&dir, RunMode::Live);
        let argv = argv_of(&["true"]);
        for seq in 1..=3 {
            run.record_step(StepRecord::new(seq, "step", StepStatus::Ok, &argv))
                .unwrap();
        }
        run.finish(RunStatus::Ok).unwrap();

        let read = TraceLog::read(&path).unwrap();
        let stamps: Vec<&str> = read
            .events
            .iter()
            .map(|e| match e {
                TraceEvent::RunStart { ts, .. }
                | TraceEvent::Step { ts, .. }
                | TraceEvent::RunEnd { ts, .. } => ts.as_str(),
            })
            .collect();
        let mut sorted = stamps.clone();
        sorted.sort_unstable();
        assert_eq!(stamps, sorted, "log lines must be in timestamp order");
        assert!(stamps.iter().all(|s| s.len() == 24 && s.ends_with('Z')));
    }
}

#[cfg(test)]
mod interrupted_tests {
    use super::*;

    #[test]
    fn a_live_pid_on_the_same_boot_is_running() {
        assert_eq!(
            run_state(Some(std::process::id()), current_boot_id().as_deref(), None),
            RunStatus::Running
        );
    }

    #[test]
    fn a_dead_pid_is_interrupted() {
        // PID 0 can never own a process, so this is deterministic.
        assert_eq!(run_state(Some(0), None, None), RunStatus::Interrupted);
    }

    #[test]
    fn a_different_boot_id_is_interrupted_even_if_the_pid_exists() {
        assert_eq!(
            run_state(Some(std::process::id()), Some("not-this-boot"), None),
            RunStatus::Interrupted
        );
    }

    #[test]
    fn a_terminal_record_wins_over_pid_evidence() {
        assert_eq!(run_state(Some(0), None, Some(RunStatus::Ok)), RunStatus::Ok);
    }
}
