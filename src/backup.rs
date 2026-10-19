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

    #[error("backup command is empty")]
    EmptyCommand,

    #[error("could not create backup directory {path}: {source}")]
    Dir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("backup command failed: {stderr}")]
    Failed { stderr: String },

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

    Ok(uses_file.then_some(file))
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

    #[test]
    fn timestamp_is_filename_safe() {
        let stamp = snapshot_timestamp();
        assert!(!stamp.contains(':'));
        assert!(!stamp.contains('.'));
        assert!(stamp.ends_with('Z'));
    }
}
