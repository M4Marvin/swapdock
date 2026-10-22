//! Pre-deploy snapshots.
//!
//! The tool cannot know how an app's state is best copied — SQLite wants
//! `.backup`, some apps want a logical dump, others only need a directory
//! copy. So the command is configuration, not code: an argv from the registry,
//! run through the same chokepoint as every other subprocess.
//!
//! Placeholders keep the command independent of one app:
//!
//! | placeholder | value |
//! |---|---|
//! | `{file}` | path of this snapshot, under the backup directory |
//! | `{app}` | app name |
//! | `{release}` | release being deployed, when known |
//! | `{timestamp}` | UTC timestamp, filename-safe |
//!
//! The command is an argv, never a shell string. That is deliberate: a registry
//! file is editable by the deploy user, but a backup command that reaches
//! `/bin/sh` would turn a typo into arbitrary execution.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;
use thiserror::Error;

use crate::exec::StepSpec;
use crate::registry::App;
use crate::time::Timestamp;
use crate::trace::{Run, RunMode, StepRecord};

/// How long a backup may take. A large database on a slow disk is still a
/// backup problem, not a reason to hold the deploy lock forever.
pub const BACKUP_TIMEOUT: Duration = Duration::from_secs(300);

/// Why a backup did not finish.
#[derive(Debug, Error)]
pub enum BackupError {
    #[error("{0} has no [backup] command configured")]
    NotConfigured(String),

    #[error("{0} has no [backup] restore_command configured")]
    RestoreNotConfigured(String),

    #[error("backup command is empty")]
    EmptyCommand,

    #[error("restore command is empty")]
    EmptyRestore,

    #[error("verify command is empty")]
    EmptyVerify,

    #[error("snapshot {0} does not exist")]
    MissingSnapshot(String),

    #[error("could not create backup directory {path}: {source}")]
    Dir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("backup command failed: {stderr}")]
    Failed { stderr: String },

    #[error("restore command failed: {stderr}")]
    RestoreFailed { stderr: String },

    #[error("snapshot verification failed: {stderr}")]
    VerifyFailed { stderr: String },

    #[error("offsite copy failed: {stderr}")]
    OffsiteFailed { stderr: String },

    #[error("refusing to run an unsafe backup/restore argv: {0}")]
    UnsafeCommand(String),

    #[error("backup command did not create {path}")]
    MissingFile { path: PathBuf },

    #[error("could not run the backup command: {0}")]
    Exec(#[from] crate::exec::ExecError),

    #[error("could not write to the run log: {0}")]
    Trace(#[from] anyhow::Error),
}

/// Expands the documented placeholders in one argument.
pub fn expand_arg(
    arg: &str,
    app: &str,
    release: Option<&str>,
    timestamp: &str,
    file: &Path,
) -> String {
    arg.replace("{app}", app)
        .replace("{release}", release.unwrap_or(""))
        .replace("{timestamp}", timestamp)
        .replace("{file}", &file.display().to_string())
}

/// A filename-safe UTC timestamp. The RFC 3339 form sorts lexicographically;
/// only the characters that filesystems dislike are replaced.
pub fn snapshot_timestamp() -> String {
    Timestamp::now().to_rfc3339().replace([':', '.'], "-")
}

/// Where this app's snapshots go.
pub fn backup_dir(app: &App) -> PathBuf {
    app.backup
        .as_ref()
        .and_then(|b| b.dir.clone())
        .unwrap_or_else(|| PathBuf::from("/srv/swapdock/backups").join(&app.name))
}

/// Runs one snapshot. Returns the file path when `{file}` was substituted.
pub fn run_backup(
    run: &mut Run,
    app: &App,
    release: Option<&str>,
) -> Result<Option<PathBuf>, BackupError> {
    let backup = app
        .backup
        .as_ref()
        .ok_or_else(|| BackupError::NotConfigured(app.name.clone()))?;
    let Some(program) = backup.command.first() else {
        return Err(BackupError::EmptyCommand);
    };
    if program.trim().is_empty() {
        return Err(BackupError::EmptyCommand);
    }
    if let Some(problem) = crate::registry::backup_argv_problem(program, &backup.command[1..]) {
        return Err(BackupError::UnsafeCommand(problem));
    }

    let timestamp = snapshot_timestamp();
    let dir = backup_dir(app);
    let file = dir.join(format!("{}-{timestamp}", app.name));
    let uses_file = backup.command.iter().any(|arg| arg.contains("{file}"));
    let args: Vec<String> = backup.command[1..]
        .iter()
        .map(|arg| expand_arg(arg, &app.name, release, &timestamp, &file))
        .collect();

    if run.mode() == RunMode::DryRun {
        let seq = run.next_seq();
        run.record_step(
            StepRecord::new(
                seq,
                "backup",
                crate::trace::StepStatus::DryRun,
                std::slice::from_ref(program),
            )
            .with_detail(json!({
                "dir": dir.display().to_string(),
                "file": uses_file.then(|| file.display().to_string()),
                "app": app.name,
                "release": release,
            })),
        )?;
        return Ok(uses_file.then_some(file));
    }

    std::fs::create_dir_all(&dir).map_err(|source| BackupError::Dir {
        path: dir.clone(),
        source,
    })?;

    let spec = StepSpec::new("backup", program)
        .args(args)
        .timeout(BACKUP_TIMEOUT);
    let outcome = run.exec(&spec)?;
    if !outcome.success() {
        return Err(BackupError::Failed {
            stderr: outcome.stderr_trimmed().to_string(),
        });
    }
    if uses_file && !file.exists() {
        return Err(BackupError::MissingFile { path: file });
    }

    if let Some(verify) = &backup.verify_command {
        let Some(verify_program) = verify.first() else {
            return Err(BackupError::EmptyVerify);
        };
        if verify_program.trim().is_empty() {
            return Err(BackupError::EmptyVerify);
        }
        if let Some(problem) = crate::registry::backup_argv_problem(verify_program, &verify[1..]) {
            return Err(BackupError::UnsafeCommand(problem));
        }
        let verify_args: Vec<String> = verify[1..]
            .iter()
            .map(|arg| expand_arg(arg, &app.name, release, &timestamp, &file))
            .collect();
        let outcome = run.exec(
            &StepSpec::new("backup-verify", verify_program)
                .args(verify_args)
                .timeout(BACKUP_TIMEOUT),
        )?;
        if !outcome.success() {
            return Err(BackupError::VerifyFailed {
                stderr: outcome.stderr_trimmed().to_string(),
            });
        }
    }

    if let Some(offsite) = &backup.offsite_command {
        let Some(offsite_program) = offsite.first() else {
            return Err(BackupError::UnsafeCommand(
                "offsite_command is empty".into(),
            ));
        };
        if let Some(problem) = crate::registry::backup_argv_problem(offsite_program, &offsite[1..])
        {
            return Err(BackupError::UnsafeCommand(problem));
        }
        let offsite_args: Vec<String> = offsite[1..]
            .iter()
            .map(|arg| expand_arg(arg, &app.name, release, &timestamp, &file))
            .collect();
        let outcome = run.exec(
            &StepSpec::new("backup-offsite", offsite_program)
                .args(offsite_args)
                .timeout(BACKUP_TIMEOUT),
        )?;
        if !outcome.success() {
            return Err(BackupError::OffsiteFailed {
                stderr: outcome.stderr_trimmed().to_string(),
            });
        }
    }

    if let Some(keep) = backup.keep
        && keep > 0
    {
        match prune_snapshots(&dir, &app.name, keep) {
            Ok(removed) if removed.is_empty() => {}
            Ok(removed) => {
                let seq = run.next_seq();
                run.record_step(
                    StepRecord::new(
                        seq,
                        "backup-prune",
                        crate::trace::StepStatus::Ok,
                        &["prune".to_string(), app.name.clone()],
                    )
                    .with_detail(json!({
                        "keep": keep,
                        "removed": removed.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
                    })),
                )?;
            }
            Err(e) => {
                tracing::warn!(app = %app.name, "backup prune failed: {e}");
                let seq = run.next_seq();
                run.record_step(
                    StepRecord::new(
                        seq,
                        "backup-prune",
                        crate::trace::StepStatus::Error,
                        &["prune".to_string(), app.name.clone()],
                    )
                    .with_detail(json!({"keep": keep, "error": e.to_string()})),
                )?;
            }
        }
    }

    Ok(uses_file.then_some(file))
}

/// Removes all but the newest `keep` snapshots for one app.
///
/// Only files whose name starts `<app>-` in the configured backup directory are
/// candidates; unknown files are never deleted. Names use an ISO timestamp, so
/// lexical order is chronological.
pub fn prune_snapshots(dir: &Path, app: &str, keep: usize) -> std::io::Result<Vec<PathBuf>> {
    if keep == 0 {
        return Ok(Vec::new());
    }
    let prefix = format!("{app}-");
    let mut snapshots: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&prefix))
                && path.is_file()
        })
        .collect();
    snapshots.sort();
    let remove_count = snapshots.len().saturating_sub(keep);
    let mut removed = Vec::with_capacity(remove_count);
    for path in snapshots.into_iter().take(remove_count) {
        std::fs::remove_file(&path)?;
        removed.push(path);
    }
    Ok(removed)
}

/// Restores one snapshot with the app's `[backup] restore_command`.
///
/// The file must exist: restoring a missing path must fail before the restore
/// program touches live state. `{file}`, `{app}`, and `{timestamp}` expand the
/// same way as the dump command.
pub fn run_restore(run: &mut Run, app: &App, file: &Path) -> Result<(), BackupError> {
    let backup = app
        .backup
        .as_ref()
        .ok_or_else(|| BackupError::NotConfigured(app.name.clone()))?;
    let Some(command) = backup.restore_command.as_ref() else {
        return Err(BackupError::RestoreNotConfigured(app.name.clone()));
    };
    let Some(program) = command.first() else {
        return Err(BackupError::EmptyRestore);
    };
    if program.trim().is_empty() {
        return Err(BackupError::EmptyRestore);
    }
    if let Some(problem) = crate::registry::backup_argv_problem(program, &command[1..]) {
        return Err(BackupError::UnsafeCommand(problem));
    }
    if !file.is_file() {
        return Err(BackupError::MissingSnapshot(file.display().to_string()));
    }

    let timestamp = snapshot_timestamp();
    let args: Vec<String> = command[1..]
        .iter()
        .map(|arg| expand_arg(arg, &app.name, app.release.as_deref(), &timestamp, file))
        .collect();

    if run.mode() == RunMode::DryRun {
        let seq = run.next_seq();
        run.record_step(
            StepRecord::new(
                seq,
                "restore",
                crate::trace::StepStatus::DryRun,
                std::slice::from_ref(program),
            )
            .with_detail(json!({
                "file": file.display().to_string(),
                "app": app.name,
            })),
        )?;
        return Ok(());
    }

    let spec = StepSpec::new("restore", program)
        .args(args)
        .timeout(BACKUP_TIMEOUT);
    let outcome = run.exec(&spec)?;
    if !outcome.success() {
        return Err(BackupError::RestoreFailed {
            stderr: outcome.stderr_trimmed().to_string(),
        });
    }
    Ok(())
}

/// Runs the configured pre-deploy backup, when the registry asked for one.
pub fn before_deploy(
    run: &mut Run,
    app: &App,
    release: Option<&str>,
) -> Result<Option<PathBuf>, BackupError> {
    match &app.backup {
        Some(backup) if backup.before_deploy => run_backup(run, app, release),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::tests::sample;

    #[test]
    fn placeholders_expand_independently_of_shell_syntax() {
        let file = Path::new("/backups/app-20260101T000000Z");
        assert_eq!(
            expand_arg(
                "backup {app} {release} {timestamp} {file}",
                "chat",
                Some("abc1234"),
                "20260101T000000Z",
                file,
            ),
            "backup chat abc1234 20260101T000000Z /backups/app-20260101T000000Z"
        );
        // An absent release expands to an empty string, not the word "None".
        assert_eq!(expand_arg("{release}", "chat", None, "t", file), "");
    }

    #[test]
    fn backup_dir_defaults_under_the_state_root() {
        let mut app = sample();
        app.name = "chat".into();
        assert_eq!(
            backup_dir(&app),
            PathBuf::from("/srv/swapdock/backups/chat")
        );
    }

    #[test]
    fn a_configured_dir_wins() {
        let mut app = sample();
        app.backup = Some(crate::registry::Backup {
            command: vec!["true".into()],
            dir: Some(PathBuf::from("/var/backups/chat")),
            before_deploy: true,
            restore_command: None,
            verify_command: None,
            keep: None,
            offsite_command: None,
        });
        assert_eq!(backup_dir(&app), PathBuf::from("/var/backups/chat"));
    }

    #[test]
    fn an_unconfigured_backup_is_not_a_no_op_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = crate::trace::TraceLog::open(dir.path().join("swapdock.jsonl")).unwrap();
        let mut run = Run::start(
            log,
            RunMode::Live,
            None,
            &["swapdock".to_string()],
            crate::redact::Redactor::new(),
        )
        .unwrap();
        let mut app = sample();
        app.backup = None;
        let err = run_backup(&mut run, &app, None).unwrap_err();
        assert!(matches!(err, BackupError::NotConfigured(_)));
    }

    fn run_in(dir: &tempfile::TempDir, mode: RunMode) -> Run {
        let log = crate::trace::TraceLog::open(dir.path().join("swapdock.jsonl")).unwrap();
        Run::start(
            log,
            mode,
            Some("chat".into()),
            &["swapdock".to_string()],
            crate::redact::Redactor::new(),
        )
        .unwrap()
    }

    #[test]
    fn an_unsafe_backup_argv_is_refused_at_execution_time() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut run = run_in(&dir, RunMode::DryRun);
        let mut app = sample();
        app.backup = Some(crate::registry::Backup {
            command: vec![
                "tar".into(),
                "--checkpoint=1".into(),
                "--checkpoint-action=exec=sh -c id".into(),
                "-cf".into(),
                "{file}".into(),
                "/tmp/data".into(),
            ],
            dir: None,
            before_deploy: false,
            restore_command: None,
            verify_command: None,
            keep: None,
            offsite_command: None,
        });
        let err = run_backup(&mut run, &app, None).unwrap_err();
        assert!(matches!(err, BackupError::UnsafeCommand(_)), "{err}");
        let read = crate::trace::TraceLog::read(dir.path().join("swapdock.jsonl")).unwrap();
        assert!(
            !read
                .events
                .iter()
                .any(|e| matches!(e, crate::trace::TraceEvent::Step { .. })),
            "the unsafe argv must be refused before any spawn"
        );
    }

    #[test]
    fn restore_requires_a_configured_command_and_an_existing_snapshot() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut run = run_in(&dir, RunMode::DryRun);
        let mut app = sample();
        app.backup = Some(crate::registry::Backup {
            command: vec!["true".into()],
            dir: None,
            before_deploy: false,
            restore_command: None,
            verify_command: None,
            keep: None,
            offsite_command: None,
        });
        let err = run_restore(&mut run, &app, Path::new("/missing/snapshot")).unwrap_err();
        assert!(matches!(err, BackupError::RestoreNotConfigured(_)), "{err}");

        app.backup.as_mut().unwrap().restore_command = Some(vec!["gzip".into(), "-h".into()]);
        let err = run_restore(&mut run, &app, Path::new("/missing/snapshot")).unwrap_err();
        assert!(matches!(err, BackupError::MissingSnapshot(_)), "{err}");
    }

    #[test]
    fn restore_dry_run_records_the_command_and_live_restore_runs_it() {
        let dir = tempfile::TempDir::new().unwrap();
        let snapshot = dir.path().join("chat.dump");
        std::fs::write(&snapshot, b"snapshot").unwrap();

        let mut app = sample();
        app.backup = Some(crate::registry::Backup {
            command: vec!["true".into()],
            dir: None,
            before_deploy: false,
            restore_command: Some(vec!["gzip".into(), "-h".into()]),
            verify_command: None,
            keep: None,
            offsite_command: None,
        });

        {
            let mut run = run_in(&dir, RunMode::DryRun);
            run_restore(&mut run, &app, &snapshot).unwrap();
            run.finish(crate::trace::RunStatus::DryRun).unwrap();
        }
        let dry = crate::trace::TraceLog::read(dir.path().join("swapdock.jsonl")).unwrap();
        assert!(
            dry.events.iter().any(|event| matches!(
                event,
                crate::trace::TraceEvent::Step { step, status, .. }
                    if step == "restore" && *status == crate::trace::StepStatus::DryRun
            )),
            "{:?}",
            dry.events
        );

        std::fs::remove_file(dir.path().join("swapdock.jsonl")).unwrap();
        let mut run = run_in(&dir, RunMode::Live);
        run_restore(&mut run, &app, &snapshot).unwrap();
        run.finish(crate::trace::RunStatus::Ok).unwrap();
        let live = crate::trace::TraceLog::read(dir.path().join("swapdock.jsonl")).unwrap();
        assert!(
            live.events.iter().any(|event| matches!(
                event,
                crate::trace::TraceEvent::Step { step, status, .. }
                    if step == "restore" && *status == crate::trace::StepStatus::Ok
            )),
            "{:?}",
            live.events
        );
    }

    #[test]
    fn a_failed_verify_command_fails_the_backup() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut run = run_in(&dir, RunMode::Live);
        let mut app = sample();
        app.backup = Some(crate::registry::Backup {
            command: vec!["gzip".into(), "-h".into()],
            dir: Some(dir.path().join("backups")),
            before_deploy: false,
            restore_command: None,
            verify_command: Some(vec!["gzip".into(), "-Z".into()]),
            keep: None,
            offsite_command: None,
        });
        let err = run_backup(&mut run, &app, None).unwrap_err();
        assert!(matches!(err, BackupError::VerifyFailed { .. }), "{err}");
    }

    #[test]
    fn a_failed_offsite_command_fails_the_backup() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut run = run_in(&dir, RunMode::Live);
        let mut app = sample();
        app.backup = Some(crate::registry::Backup {
            command: vec!["gzip".into(), "-h".into()],
            dir: Some(dir.path().join("backups")),
            before_deploy: false,
            restore_command: None,
            verify_command: None,
            keep: None,
            offsite_command: Some(vec!["gzip".into(), "-Z".into()]),
        });
        let err = run_backup(&mut run, &app, None).unwrap_err();
        assert!(matches!(err, BackupError::OffsiteFailed { .. }), "{err}");
    }

    #[test]
    fn pruning_keeps_the_newest_and_ignores_other_files() {
        let dir = tempfile::TempDir::new().unwrap();
        for name in ["chat-20260101", "chat-20260102", "chat-20260103", "other"] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        let removed = prune_snapshots(dir.path(), "chat", 2).unwrap();
        assert_eq!(removed.len(), 1, "{removed:?}");
        assert!(removed[0].ends_with("chat-20260101"));
        assert!(dir.path().join("chat-20260102").exists());
        assert!(dir.path().join("chat-20260103").exists());
        assert!(dir.path().join("other").exists());
    }

    #[test]
    fn timestamp_is_filename_safe() {
        let stamp = snapshot_timestamp();
        assert!(!stamp.contains(':'));
        assert!(!stamp.contains('.'));
        assert!(stamp.ends_with('Z'));
    }
}
