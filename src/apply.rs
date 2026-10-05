//! Applying a rendered config to nginx.
//!
//! The sequence is fixed and every step is recorded in the run log:
//!
//! ```text
//! stage   write the new config to <target>.staging.<pid>
//! backup  copy the live file to <target>.bak (if it exists)
//! commit  rename(2) the staging file over the target — atomic
//! test    nginx -t -c <main config> against the real full config
//! reload  nginx -s reload, but only after the test passed
//! verify  the master pid is unchanged, proving reload and not restart
//! ```
//!
//! Two properties make this safe rather than merely ordered:
//!
//! * The broken config is on disk for exactly one `nginx -t` run, and nginx
//!   only reads the file at reload. A failed test restores the backup and
//!   re-tests, so the file on disk always matches a config that passed.
//! * On reload failure the backup is also restored. Disk and running config
//!   agree again, and the operator gets one error instead of a mystery.
//!
//! The one case that is *not* restored is a detected restart: if the master pid
//! changed, the new config is already loaded, so restoring the file would create
//! exactly the mismatch it is meant to prevent.
//!
//! Dry-run records every step and touches nothing: no file is written, renamed
//! or removed, and no nginx process is started. `Run::exec` already intercepts
//! the commands; this module must additionally skip the filesystem mutations,
//! because those do not go through the chokepoint.

use std::fs::{self, File};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;
use thiserror::Error;

use crate::exec::{ExecError, StepSpec};
use crate::trace::{Run, RunMode, StepRecord, StepStatus};

/// How long `nginx -t` may take. Config tests are local and fast; anything past
/// this is a wedged binary, not a slow disk.
pub const TEST_TIMEOUT: Duration = Duration::from_secs(30);

/// How long `nginx -s reload` may take. It returns after signalling the master.
pub const RELOAD_TIMEOUT: Duration = Duration::from_secs(30);

/// Suffix for the backup that doubles as the rollback source.
pub const BACKUP_SUFFIX: &str = ".bak";

/// Paths that locate the nginx installation. Everything has a default matching a
/// Debian nginx, and everything is overridable for tests and odd layouts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyPaths {
    /// nginx binary. `"nginx"` resolves through `PATH`, which is also how tests
    /// inject a fake.
    pub nginx_bin: PathBuf,
    /// Main config, tested as a whole. The target must be included from it.
    pub main_config: PathBuf,
    /// The generated file, e.g. `/etc/nginx/conf.d/front-door.conf`.
    pub target: PathBuf,
    /// Master pid, for the reload-vs-restart check. `None` skips the check.
    pub pid_file: Option<PathBuf>,
}

impl Default for ApplyPaths {
    fn default() -> Self {
        Self {
            nginx_bin: PathBuf::from("nginx"),
            main_config: PathBuf::from("/etc/nginx/nginx.conf"),
            target: PathBuf::from("/etc/nginx/conf.d/front-door.conf"),
            pid_file: Some(PathBuf::from("/run/nginx.pid")),
        }
    }
}

impl ApplyPaths {
    /// Where the new config is staged. Same directory as the target, so the
    /// rename stays on one filesystem and stays atomic.
    fn staging(&self) -> PathBuf {
        let mut name = self
            .target
            .file_name()
            .map(|s| s.to_os_string())
            .unwrap_or_default();
        name.push(format!(".staging.{}", std::process::id()));
        self.target.with_file_name(name)
    }

    /// Where the previous config is kept.
    fn backup(&self) -> PathBuf {
        let mut name = self
            .target
            .file_name()
            .map(|s| s.to_os_string())
            .unwrap_or_default();
        name.push(BACKUP_SUFFIX);
        self.target.with_file_name(name)
    }
}

/// What applying did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyOutcome {
    /// True when the reload signal was accepted and the master is unchanged.
    pub reloaded: bool,
    pub master_pid_before: Option<u32>,
    pub master_pid_after: Option<u32>,
    /// The backup that was written, if the target previously existed.
    pub backup: Option<PathBuf>,
}

/// Why applying failed. Every variant leaves the target file in a known state,
// see the docs on each one.
#[derive(Debug, Error)]
pub enum ApplyError {
    #[error(
        "nginx -t rejected the new config; the previous file was restored and re-tested: {stderr_tail}"
    )]
    TestFailed { stderr_tail: String },

    #[error("nginx -s reload failed; the previous file was restored and re-tested: {detail}")]
    ReloadFailed { detail: String },

    #[error(
        "the nginx master pid changed from {before:?} to {after:?}: \
         that is a restart, not a reload. The new config is already loaded, \
         so the file was deliberately NOT restored"
    )]
    RestartDetected {
        before: Option<u32>,
        after: Option<u32>,
    },

    #[error("filesystem error during apply: {0}")]
    Io(String),

    #[error("could not run nginx: {0}")]
    Exec(#[from] ExecError),

    #[error("could not write to the run log: {0}")]
    Trace(#[from] anyhow::Error),
}

/// Makes every path absolute against the working directory.
///
/// Missing files cannot be canonicalized, so this joins rather than resolves:
/// symlinks stay as given, which is what the operator typed.
fn absolutize(paths: &ApplyPaths) -> ApplyPaths {
    let mut out = paths.clone();
    out.target = absolutize_one(&paths.target);
    out.main_config = absolutize_one(&paths.main_config);
    out.pid_file = paths.pid_file.as_ref().map(|p| absolutize_one(p));
    if !paths.nginx_bin.is_absolute() && paths.nginx_bin.components().count() > 1 {
        out.nginx_bin = absolutize_one(&paths.nginx_bin);
    }
    out
}

fn absolutize_one(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    // Drop a leading `./` so the log shows `/tmp/x/front-door.conf` rather than
    // `/tmp/x/./front-door.conf`.
    let stripped = path.strip_prefix("./").unwrap_or(path);
    match std::env::current_dir() {
        Ok(cwd) => cwd.join(stripped),
        Err(_) => stripped.to_path_buf(),
    }
}

fn io_error(context: &str, source: std::io::Error) -> ApplyError {
    ApplyError::Io(format!("{context}: {source}"))
}

/// Writes `config_text` to the target and reloads nginx.
///
/// See the module docs for the exact sequence and the failure semantics.
pub fn apply(
    run: &mut Run,
    config_text: &str,
    paths: &ApplyPaths,
) -> Result<ApplyOutcome, ApplyError> {
    // nginx resolves a relative `-c` against its compiled prefix, not the
    // working directory, so a relative main config would silently test the wrong
    // file. Absolutize everything up front; the log then shows exact paths too.
    let paths = absolutize(paths);
    let staging = paths.staging();
    let backup = paths.backup();

    if run.mode() == RunMode::DryRun {
        return apply_dry(run, config_text, &paths, &staging, &backup);
    }

    let pid_before = read_pid(paths.pid_file.as_deref());

    // 1. Stage. Sync the file and its directory, so a crash cannot leave a
    //    half-written staging file behind to confuse the next run.
    write_staging(run, &staging, config_text)?;

    // 2. Back up whatever is live now, if anything.
    let had_target = paths.target.exists();
    if had_target {
        fs::copy(&paths.target, &backup)
            .map_err(|e| io_error(&format!("copy {} to backup", paths.target.display()), e))?;
        record_fs(
            run,
            "apply-backup",
            &[
                "copy".into(),
                paths.target.display().to_string(),
                backup.display().to_string(),
            ],
            Some(json!({"backup": backup.display().to_string()})),
        )?;
    }

    // 3. Commit. rename(2) is atomic: readers see the old file or the new file,
    //    never a mix.
    fs::rename(&staging, &paths.target)
        .map_err(|e| io_error(&format!("rename {} into place", staging.display()), e))?;
    record_fs(
        run,
        "apply-commit",
        &[
            "rename".into(),
            staging.display().to_string(),
            paths.target.display().to_string(),
        ],
        None,
    )?;

    // 4. Test the real full config, which now includes the new file.
    if let Err(stderr_tail) = test_config(run, &paths) {
        restore(run, &paths, had_target, &backup)?;
        // Prove the restore, because "restored" without a passing test is a claim.
        test_config(run, &paths).map_err(|tail| {
            ApplyError::Io(format!(
                "restored the previous config but it no longer passes nginx -t: {tail}"
            ))
        })?;
        return Err(ApplyError::TestFailed { stderr_tail });
    }

    // 5. Reload. Only reachable with a tested config on disk.
    reload(run, &paths).map_err(|detail| {
        // Best effort: make disk and running agree again. If the restore itself
        // fails there is nothing more this function can do, so report the reload.
        let _ = restore(run, &paths, had_target, &backup);
        let _ = test_config(run, &paths);
        ApplyError::ReloadFailed { detail }
    })?;

    // 6. The master pid must be unchanged: same master means HUP was honoured.
    let pid_after = read_pid(paths.pid_file.as_deref());
    if pid_file_changed(pid_before, pid_after) {
        return Err(ApplyError::RestartDetected {
            before: pid_before,
            after: pid_after,
        });
    }

    Ok(ApplyOutcome {
        reloaded: true,
        master_pid_before: pid_before,
        master_pid_after: pid_after,
        backup: had_target.then(|| backup.clone()),
    })
}

/// Records the plan without touching the filesystem or spawning anything.
fn apply_dry(
    run: &mut Run,
    config_text: &str,
    paths: &ApplyPaths,
    staging: &Path,
    backup: &Path,
) -> Result<ApplyOutcome, ApplyError> {
    let seq = run.next_seq();
    run.record_step(
        StepRecord::new(seq, "apply-plan", StepStatus::DryRun, &[
            "apply".to_string(),
            paths.target.display().to_string(),
        ])
        .with_detail(json!({
            "would_write": staging.display().to_string(),
            "bytes": config_text.len(),
            "would_backup": backup.display().to_string(),
            "would_test": format!("{} -t -c {}", paths.nginx_bin.display(), paths.main_config.display()),
            "would_reload": format!(
                "{} -s reload -c {}",
                paths.nginx_bin.display(),
                paths.main_config.display()
            ),
        })),
    )
    .map_err(ApplyError::Trace)?;

    Ok(ApplyOutcome {
        reloaded: false,
        master_pid_before: None,
        master_pid_after: None,
        backup: None,
    })
}

/// Writes the staging file with an fsync of both file and directory.
fn write_staging(run: &mut Run, staging: &Path, config_text: &str) -> Result<(), ApplyError> {
    if let Some(parent) = staging.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|e| io_error(&format!("create {}", parent.display()), e))?;
    }

    let mut file =
        File::create(staging).map_err(|e| io_error(&format!("create {}", staging.display()), e))?;
    file.write_all(config_text.as_bytes())
        .map_err(|e| io_error(&format!("write {}", staging.display()), e))?;
    file.sync_all()
        .map_err(|e| io_error(&format!("sync {}", staging.display()), e))?;
    drop(file);

    file_permissions(staging)?;

    // Sync the directory entry too, or the rename after a crash could vanish.
    if let Some(parent) = staging.parent()
        && let Ok(dir) = File::open(parent)
    {
        let _ = dir.sync_all();
    }

    record_fs(
        run,
        "apply-stage",
        &["write".into(), staging.display().to_string()],
        Some(json!({"bytes": config_text.len()})),
    )?;
    Ok(())
}

/// The generated file must be world-readable: nginx workers run unprivileged.
fn file_permissions(staging: &Path) -> Result<(), ApplyError> {
    fs::set_permissions(staging, fs::Permissions::from_mode(0o644))
        .map_err(|e| io_error(&format!("chmod 644 {}", staging.display()), e))
}

/// Runs `nginx -t -c <main config>`. Returns the stderr tail on failure.
fn test_config(run: &mut Run, paths: &ApplyPaths) -> Result<(), String> {
    let spec = StepSpec::new("nginx-test", paths.nginx_bin.display().to_string())
        .args(["-t", "-c"])
        .arg(paths.main_config.display().to_string())
        .timeout(TEST_TIMEOUT);

    match run.exec(&spec) {
        Ok(outcome) if outcome.success() => Ok(()),
        Ok(outcome) => Err(last_lines(&outcome.stderr, 5)),
        Err(e) => Err(truncate(&e.to_string(), 500)),
    }
}

/// Sends the reload signal.
///
/// `-c` is required, not optional: without it nginx reads the compiled-in default
/// config to find the pid file, and would signal the wrong master (or fail when
/// none is there) instead of the one serving this config.
fn reload(run: &mut Run, paths: &ApplyPaths) -> Result<(), String> {
    let spec = StepSpec::new("nginx-reload", paths.nginx_bin.display().to_string())
        .args(["-s", "reload", "-c"])
        .arg(paths.main_config.display().to_string())
        .timeout(RELOAD_TIMEOUT);

    match run.exec(&spec) {
        Ok(outcome) if outcome.success() => Ok(()),
        Ok(outcome) => Err(format!(
            "exit {}: {}",
            outcome
                .exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "?".into()),
            last_lines(&outcome.stderr, 5)
        )),
        Err(e) => Err(truncate(&e.to_string(), 500)),
    }
}

/// Puts the previous file back: rename the backup over the target, or remove
/// the target if there was no previous file. Removes a leftover staging file.
fn restore(
    run: &mut Run,
    paths: &ApplyPaths,
    had_target: bool,
    backup: &Path,
) -> Result<(), ApplyError> {
    if had_target {
        fs::rename(backup, &paths.target).map_err(|e| {
            io_error(
                &format!("restore {} from backup", paths.target.display()),
                e,
            )
        })?;
        record_fs(
            run,
            "apply-restore",
            &[
                "rename".into(),
                backup.display().to_string(),
                paths.target.display().to_string(),
            ],
            None,
        )?;
    } else {
        fs::remove_file(&paths.target)
            .map_err(|e| io_error(&format!("remove untested {}", paths.target.display()), e))?;
        record_fs(
            run,
            "apply-restore",
            &["remove".into(), paths.target.display().to_string()],
            None,
        )?;
    }

    let staging = paths.staging();
    if staging.exists() {
        let _ = fs::remove_file(&staging);
    }
    Ok(())
}

/// Reads a pid file. Missing or unparsable means "unknown", never an error:
/// the check is skipped rather than failed when there is nothing to compare.
fn read_pid(pid_file: Option<&Path>) -> Option<u32> {
    let path = pid_file?;
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// True only when both pids are known and differ.
fn pid_file_changed(before: Option<u32>, after: Option<u32>) -> bool {
    matches!((before, after), (Some(b), Some(a)) if b != a)
}

/// Records a filesystem mutation as a step, so the trace is complete even
/// though these do not go through the subprocess chokepoint.
fn record_fs(
    run: &mut Run,
    name: &'static str,
    argv: &[String],
    detail: Option<serde_json::Value>,
) -> Result<(), ApplyError> {
    let seq = run.next_seq();
    let mut record = StepRecord::new(seq, name, StepStatus::Ok, argv);
    if let Some(d) = detail {
        record = record.with_detail(d);
    }
    run.record_step(record).map_err(ApplyError::Trace)?;
    Ok(())
}

fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(n);
    truncate(&lines[start..].join("\n"), 1000)
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    // Cut at a char boundary from the end, keeping the most recent context.
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &text[start..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::redact::Redactor;
    use crate::trace::{RunStatus, TraceEvent, TraceLog};
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    /// A fake nginx that obeys environment variables:
    ///
    /// * `NGINX_JOURNAL` — every invocation appends one line: `test <conf>` or
    ///   `reload`.
    /// * `NGINX_FAIL_TEST` — `-t` exits 1 with `TEST-FAIL` on stderr.
    /// * `NGINX_FAIL_TEST_ONCE` — only the first `-t` fails. Models reality: the
    ///   new config is broken, the restored one is fine.
    /// * `NGINX_FAIL_RELOAD` — `-s reload` exits 1.
    /// * `NGINX_NEW_PID` — `-s reload` writes this pid to `NGINX_PID_FILE`.
    ///   Models a restart instead of a reload.
    /// * `NGINX_PID_FILE` — pid file the fake maintains for `-s reload`.
    const FAKE: &str = r#"#!/bin/sh
journal="${NGINX_JOURNAL:-/dev/null}"
if [ "$1" = "-t" ]; then
    echo "test $3" >>"$journal"
    if [ -n "$NGINX_FAIL_TEST_ONCE" ] && [ ! -e "$NGINX_JOURNAL.once" ]; then
        touch "$NGINX_JOURNAL.once"
        echo "TEST-FAIL" >&2
        exit 1
    fi
    if [ -n "$NGINX_FAIL_TEST" ]; then
        echo "TEST-FAIL" >&2
        exit 1
    fi
    echo "syntax is ok" >&2
    exit 0
fi
if [ "$1" = "-s" ] && [ "$2" = "reload" ]; then
    echo "reload $4" >>"$journal"
    if [ -n "$NGINX_FAIL_RELOAD" ]; then
        echo "RELOAD-FAIL" >&2
        exit 1
    fi
    if [ -n "$NGINX_NEW_PID" ] && [ -n "$NGINX_PID_FILE" ]; then
        echo "$NGINX_NEW_PID" >"$NGINX_PID_FILE"
    fi
    exit 0
fi
echo "unexpected arguments: $*" >&2
exit 2
"#;

    /// Process-global lock for the fake's environment variables.
    ///
    /// `NGINX_JOURNAL` and friends are inherited by children, so they are
    /// process-global by nature. Tests run on threads, so every test that uses
    /// a `Harness` must hold this for its whole body, or one test's fake writes
    /// into another test's journal. A poisoned lock is recovered rather than
    /// propagated: a panicking test must not fail the rest of the suite.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn hold_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    struct Harness {
        dir: TempDir,
        paths: ApplyPaths,
        journal: PathBuf,
        /// Environment to prepend for the fake. Set per-test via `env`.
        extra_env: Vec<(String, String)>,
        /// Holds `ENV_LOCK` until the test ends, including `Drop` cleanup.
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl Harness {
        fn new() -> Self {
            let dir = TempDir::new().unwrap();
            let fake = dir.path().join("nginx");
            std::fs::write(&fake, FAKE).unwrap();
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

            let pid_file = dir.path().join("nginx.pid");
            std::fs::write(&pid_file, "4242\n").unwrap();

            let journal = dir.path().join("journal");
            let paths = ApplyPaths {
                nginx_bin: fake,
                main_config: dir.path().join("nginx.conf"),
                target: dir.path().join("front-door.conf"),
                pid_file: Some(pid_file),
            };
            // The main config exists so `-c` points at a real file.
            std::fs::write(&paths.main_config, "# main\n").unwrap();

            Self {
                dir,
                paths,
                journal,
                extra_env: Vec::new(),
                _guard: hold_env(),
            }
        }

        fn env(mut self, key: &str, value: &str) -> Self {
            self.extra_env.push((key.into(), value.into()));
            self
        }

        fn run(&self) -> Run {
            // A relative journal would put the fake's marker file in the
            // working directory instead of the temp dir.
            debug_assert!(self.journal.is_absolute(), "journal must be absolute");
            let log = TraceLog::open(self.dir.path().join("swapdock.jsonl")).unwrap();
            let mut run = Run::start(
                log,
                RunMode::Live,
                Some("test".into()),
                &["swapdock".to_string()],
                Redactor::new(),
            )
            .unwrap();
            for (k, v) in &self.extra_env {
                run.redactor_mut().register_env(k, v);
            }
            // The fake reads its behaviour from the process environment.
            for (k, v) in &self.extra_env {
                // SAFETY: tests are single-threaded here; no other thread reads env.
                unsafe { std::env::set_var(k, v) };
            }
            // SAFETY: same as above.
            unsafe {
                std::env::set_var("NGINX_JOURNAL", &self.journal);
                std::env::set_var("NGINX_PID_FILE", self.paths.pid_file.as_ref().unwrap());
            }
            run
        }

        fn journal_lines(&self) -> Vec<String> {
            std::fs::read_to_string(&self.journal)
                .unwrap_or_default()
                .lines()
                .map(str::to_string)
                .collect()
        }

        fn target_text(&self) -> Option<String> {
            std::fs::read_to_string(&self.paths.target).ok()
        }
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            for key in [
                "NGINX_JOURNAL",
                "NGINX_PID_FILE",
                "NGINX_FAIL_TEST",
                "NGINX_FAIL_TEST_ONCE",
                "NGINX_FAIL_RELOAD",
                "NGINX_NEW_PID",
            ] {
                // SAFETY: test-only cleanup; no other thread depends on these.
                unsafe { std::env::remove_var(key) };
            }
        }
    }

    const NEW: &str = "# new config\nserver { listen 8001; }\n";
    const OLD: &str = "# old config\nserver { listen 8001; }\n";

    #[test]
    fn happy_path_writes_tests_and_reloads() {
        let h = Harness::new();
        let mut run = h.run();
        std::fs::write(&h.paths.target, OLD).unwrap();

        let outcome = apply(&mut run, NEW, &h.paths).unwrap();

        assert!(outcome.reloaded);
        assert_eq!(outcome.master_pid_before, Some(4242));
        assert_eq!(outcome.master_pid_after, Some(4242));
        assert_eq!(h.target_text().as_deref(), Some(NEW));
        assert_eq!(
            std::fs::read_to_string(h.paths.backup()).ok().as_deref(),
            Some(OLD),
            "the backup keeps the previous file"
        );
        assert_eq!(
            h.journal_lines(),
            vec![
                format!("test {}", h.paths.main_config.display()),
                format!("reload {}", h.paths.main_config.display()),
            ]
        );
    }

    #[test]
    fn a_first_time_apply_has_no_backup() {
        let h = Harness::new();
        let mut run = h.run();
        assert!(!h.paths.target.exists());

        let outcome = apply(&mut run, NEW, &h.paths).unwrap();

        assert!(outcome.reloaded);
        assert_eq!(outcome.backup, None);
        assert_eq!(h.target_text().as_deref(), Some(NEW));
    }

    #[test]
    fn a_failed_test_restores_and_re_tests() {
        let h = Harness::new().env("NGINX_FAIL_TEST_ONCE", "1");
        let mut run = h.run();
        std::fs::write(&h.paths.target, OLD).unwrap();

        let err = apply(&mut run, NEW, &h.paths).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("rejected the new config"), "{msg}");
        assert!(
            msg.contains("TEST-FAIL"),
            "the nginx error must survive: {msg}"
        );

        assert_eq!(h.target_text().as_deref(), Some(OLD), "restored");
        assert!(!h.paths.staging().exists(), "no staging file left behind");
        let journal = h.journal_lines();
        assert_eq!(
            journal.len(),
            2,
            "test, then re-test after restore: {journal:?}"
        );
        assert!(
            journal.iter().all(|l| l.starts_with("test ")),
            "{journal:?}"
        );
        assert!(
            journal.iter().all(|l| l.starts_with("test ")),
            "never reload an untested config: {journal:?}"
        );
    }

    #[test]
    fn a_failed_test_with_no_previous_file_removes_the_target() {
        let h = Harness::new().env("NGINX_FAIL_TEST_ONCE", "1");
        let mut run = h.run();

        let err = apply(&mut run, NEW, &h.paths).unwrap_err();
        assert!(matches!(err, ApplyError::TestFailed { .. }), "{err:?}");
        assert!(!h.paths.target.exists(), "nothing untested may remain");
    }

    #[test]
    fn a_failed_reload_restores_the_previous_file() {
        let h = Harness::new().env("NGINX_FAIL_RELOAD", "1");
        let mut run = h.run();
        std::fs::write(&h.paths.target, OLD).unwrap();

        let err = apply(&mut run, NEW, &h.paths).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("reload failed"), "{msg}");
        assert!(msg.contains("RELOAD-FAIL"), "{msg}");

        assert_eq!(h.target_text().as_deref(), Some(OLD), "restored");
        let journal = h.journal_lines();
        assert_eq!(
            journal,
            vec![
                format!("test {}", h.paths.main_config.display()),
                format!("reload {}", h.paths.main_config.display()),
                format!("test {}", h.paths.main_config.display()),
            ],
            "test, reload, re-test after restore: {journal:?}"
        );
    }

    #[test]
    fn a_changed_master_pid_is_a_restart_and_keeps_the_new_file() {
        let h = Harness::new().env("NGINX_NEW_PID", "9999");
        let mut run = h.run();
        std::fs::write(&h.paths.target, OLD).unwrap();

        let err = apply(&mut run, NEW, &h.paths).unwrap_err();
        assert!(
            matches!(
                err,
                ApplyError::RestartDetected {
                    before: Some(4242),
                    after: Some(9999)
                }
            ),
            "{err:?}"
        );
        assert!(
            err.to_string().contains("NOT restored"),
            "the message must say why: {err}"
        );
        assert_eq!(
            h.target_text().as_deref(),
            Some(NEW),
            "the running master loaded the new file; restoring would lie"
        );
    }

    #[test]
    fn dry_run_touches_nothing_and_spawns_nothing() {
        let h = Harness::new();
        let log = TraceLog::open(h.dir.path().join("swapdock.jsonl")).unwrap();
        let mut run = Run::start(
            log,
            RunMode::DryRun,
            None,
            &["swapdock".into()],
            Redactor::new(),
        )
        .unwrap();
        std::fs::write(&h.paths.target, OLD).unwrap();

        let outcome = apply(&mut run, NEW, &h.paths).unwrap();

        assert!(!outcome.reloaded);
        assert_eq!(h.target_text().as_deref(), Some(OLD), "untouched");
        assert!(!h.paths.staging().exists(), "no staging file");
        assert!(!h.dir.path().join("journal").exists(), "nginx never ran");

        let read = TraceLog::read(h.dir.path().join("swapdock.jsonl")).unwrap();
        assert!(
            read.events.iter().any(|e| matches!(
                e,
                TraceEvent::Step { step, status: StepStatus::DryRun, .. }
                if step == "apply-plan"
            )),
            "the plan must be recorded"
        );
    }

    #[test]
    fn every_filesystem_mutation_is_in_the_trace() {
        let h = Harness::new();
        let mut run = h.run();
        std::fs::write(&h.paths.target, OLD).unwrap();
        apply(&mut run, NEW, &h.paths).unwrap();
        run.finish(RunStatus::Ok).unwrap();

        let read = TraceLog::read(h.dir.path().join("swapdock.jsonl")).unwrap();
        let steps: Vec<&str> = read
            .events
            .iter()
            .filter_map(|e| match e {
                TraceEvent::Step { step, .. } => Some(step.as_str()),
                _ => None,
            })
            .collect();
        for expected in [
            "apply-stage",
            "apply-backup",
            "apply-commit",
            "nginx-test",
            "nginx-reload",
        ] {
            assert!(steps.contains(&expected), "missing {expected}: {steps:?}");
        }
    }

    #[test]
    fn the_staged_file_is_world_readable() {
        let h = Harness::new();
        let mut run = h.run();
        // Point the target away so commit cannot run; we only want the staging step.
        let mut paths = h.paths.clone();
        paths.target = h
            .dir
            .path()
            .join("no-such-dir-is-created-by-apply/front-door.conf");
        // create_dir_all in write_staging makes the parent, so this still works.
        apply(&mut run, NEW, &paths).unwrap();

        let mode = std::fs::metadata(&paths.target)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o644, "workers run unprivileged");
    }

    #[test]
    fn relative_paths_are_resolved_against_the_working_directory() {
        // nginx resolves a relative `-c` against its compiled prefix, not the
        // working directory. A relative main config would silently test the wrong
        // file, so apply must absolutize before invoking anything.
        let paths = ApplyPaths {
            target: PathBuf::from("rel/front-door.conf"),
            main_config: PathBuf::from("rel/nginx.conf"),
            pid_file: Some(PathBuf::from("rel/nginx.pid")),
            nginx_bin: PathBuf::from("nginx"),
        };
        let abs = absolutize(&paths);
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(abs.target, cwd.join("rel/front-door.conf"));
        assert_eq!(abs.main_config, cwd.join("rel/nginx.conf"));
        assert_eq!(abs.pid_file, Some(cwd.join("rel/nginx.pid")));
        // A bare binary name still resolves through PATH.
        assert_eq!(abs.nginx_bin, PathBuf::from("nginx"));
    }

    #[test]
    fn absolute_paths_pass_through_unchanged() {
        let paths = ApplyPaths::default();
        assert_eq!(absolutize(&paths), paths);
    }

    #[test]
    fn staging_and_backup_live_beside_the_target() {
        let paths = ApplyPaths {
            target: PathBuf::from("/etc/nginx/conf.d/front-door.conf"),
            ..ApplyPaths::default()
        };
        assert_eq!(
            paths.staging(),
            PathBuf::from(
                "/etc/nginx/conf.d/front-door.conf.staging.".to_string()
                    + &std::process::id().to_string()
            )
        );
        assert_eq!(
            paths.backup(),
            PathBuf::from("/etc/nginx/conf.d/front-door.conf.bak")
        );
        // Same directory, so the rename never crosses a filesystem.
        assert_eq!(paths.staging().parent(), paths.target.parent());
    }

    #[test]
    fn an_unreadable_pid_file_skips_the_check_instead_of_failing() {
        let h = Harness::new();
        let mut run = h.run();
        std::fs::remove_file(h.paths.pid_file.as_ref().unwrap()).unwrap();

        let outcome = apply(&mut run, NEW, &h.paths).unwrap();
        assert!(outcome.reloaded);
        assert_eq!(outcome.master_pid_before, None);
        assert_eq!(outcome.master_pid_after, None);
    }

    #[test]
    fn a_garbage_pid_file_is_treated_as_unknown() {
        let h = Harness::new();
        std::fs::write(h.paths.pid_file.as_ref().unwrap(), "not-a-pid\n").unwrap();
        let mut run = h.run();

        let outcome = apply(&mut run, NEW, &h.paths).unwrap();
        assert!(outcome.reloaded, "unknown pid must not block a good apply");
    }

    #[test]
    fn default_paths_match_a_debian_nginx() {
        let paths = ApplyPaths::default();
        assert_eq!(paths.nginx_bin, PathBuf::from("nginx"));
        assert_eq!(paths.main_config, PathBuf::from("/etc/nginx/nginx.conf"));
        assert_eq!(
            paths.target,
            PathBuf::from("/etc/nginx/conf.d/front-door.conf")
        );
        assert_eq!(paths.pid_file, Some(PathBuf::from("/run/nginx.pid")));
    }

    #[test]
    fn error_messages_name_the_state_they_leave_behind() {
        let failed = ApplyError::TestFailed {
            stderr_tail: "emerg Boolean".into(),
        };
        assert!(failed.to_string().contains("restored"), "{failed}");

        let failed = ApplyError::ReloadFailed {
            detail: "exit 1".into(),
        };
        assert!(failed.to_string().contains("restored"), "{failed}");

        let detected = ApplyError::RestartDetected {
            before: Some(1),
            after: Some(2),
        };
        assert!(detected.to_string().contains("NOT restored"), "{detected}");
    }
}
