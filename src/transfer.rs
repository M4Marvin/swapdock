//! Shipping a built image to another host.
//!
//! A transfer is not a deploy: it moves an already-built image to a second
//! machine and loads it there. It is useful when the box that builds is not the
//! box that runs the app, and the image is too large to push through a registry.
//!
//! Every command goes through the exec chokepoint, so a transfer is a run with
//! steps and shows up in `/events` exactly like a build or a swapdock:
//!
//! ```text
//! save   docker save <image_ref> -o <stagedir>/<name>-<release>.tar
//! send   tailscale file cp <tar> <target>:   (falls back to scp on failure)
//! load   ssh <target> docker load -i <remote path>
//! verify ssh <target> docker image inspect <image_ref>
//! clean  remove the local tarball, best effort
//! ```
//!
//! The tarball lands in the target's taildrop receive directory when taildrop is
//! used, which is `~`; scp lands it in `/tmp`. The remote load therefore names
//! the path it actually sent to, never a guess.

use std::path::Path;
use std::time::Duration;

use serde_json::json;
use thiserror::Error;

use crate::exec::{ExecError, StepSpec};
use crate::registry::App;
use crate::trace::{Run, StepRecord, StepStatus};

/// How long `docker save` may take. Writing a multi-gigabyte image is slow.
pub const SAVE_TIMEOUT: Duration = Duration::from_secs(600);

/// How long the tarball may take to reach the target.
pub const SEND_TIMEOUT: Duration = Duration::from_secs(600);

/// How long the remote `docker load` and its confirming inspect may take.
pub const LOAD_TIMEOUT: Duration = Duration::from_secs(600);

/// Why a transfer did not finish.
#[derive(Debug, Error)]
pub enum TransferError {
    #[error("{0} has no image_repo recorded; set image_repo before transferring")]
    NoImageRepo(String),

    #[error("invalid release {0:?}: must be a commit SHA (hex, 4-64 chars)")]
    InvalidRelease(String),

    #[error("could not prepare the transfer directory: {0}")]
    Io(String),

    #[error("docker save failed: {0}")]
    SaveFailed(String),

    #[error("could not send the image to {0}: {1}")]
    SendFailed(String, String),

    #[error("remote load on {0} failed: {1}")]
    LoadFailed(String, String),

    #[error("could not run a transfer step: {0}")]
    Exec(#[from] ExecError),

    #[error("could not write to the run log: {0}")]
    Trace(#[from] anyhow::Error),
}

/// What a finished transfer moved, and how.
#[derive(Debug, Clone)]
pub struct TransferOutcome {
    pub image_ref: String,
    pub target: String,
    /// `taildrop` or `scp`, whichever carried the tarball.
    pub method: &'static str,
}

/// Saves `release` of `app` and loads it on `target`.
///
/// Validation happens before the first spawn, so an invalid release or an app
/// with no `image_repo` leaves no run behind.
pub fn transfer(
    run: &mut Run,
    app: &App,
    release: &str,
    target: &str,
    transfer_dir: &Path,
) -> Result<TransferOutcome, TransferError> {
    if !crate::registry::is_valid_release_arg(release) {
        return Err(TransferError::InvalidRelease(release.to_string()));
    }
    let image_ref = app
        .image_ref(release)
        .ok_or_else(|| TransferError::NoImageRepo(app.name.clone()))?;

    std::fs::create_dir_all(transfer_dir)
        .map_err(|e| TransferError::Io(format!("create {}: {e}", transfer_dir.display())))?;

    let tarball_name = format!("{}-{release}.tar", app.name);
    let tarball = transfer_dir.join(&tarball_name);

    // (a) save the image to a local tarball.
    let save = StepSpec::new("transfer-save", "docker")
        .args(["save", "-o"])
        .arg(tarball.display().to_string())
        .arg(&image_ref)
        .timeout(SAVE_TIMEOUT);
    let outcome = run.exec(&save)?;
    if !outcome.success() {
        return Err(TransferError::SaveFailed(
            outcome.stderr_trimmed().to_string(),
        ));
    }

    // (b) send it, preferring taildrop and falling back to scp.
    let (method, remote_path) = send(run, &tarball, &tarball_name, target)?;

    // (c) load it on the far side and confirm the image is present.
    let load = StepSpec::new("transfer-load", "ssh")
        .args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=30",
            target,
            "--",
        ])
        .args(["docker", "load", "-i"])
        .arg(&remote_path)
        .timeout(LOAD_TIMEOUT);
    let outcome = run.exec(&load)?;
    if !outcome.success() {
        return Err(TransferError::LoadFailed(
            target.to_string(),
            outcome.stderr_trimmed().to_string(),
        ));
    }

    let verify = StepSpec::new("transfer-verify", "ssh")
        .args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=30",
            target,
            "--",
        ])
        .args(["docker", "image", "inspect", &image_ref])
        .timeout(LOAD_TIMEOUT);
    let outcome = run.exec(&verify)?;
    if !outcome.success() {
        return Err(TransferError::LoadFailed(
            target.to_string(),
            format!(
                "{} was loaded but not inspectable: {}",
                image_ref,
                outcome.stderr_trimmed()
            ),
        ));
    }

    // (d) drop the local tarball. Best effort: a leftover file is a nuisance,
    // not a failed transfer, so it is recorded but never fatal.
    cleanup(run, &tarball)?;

    Ok(TransferOutcome {
        image_ref,
        target: target.to_string(),
        method,
    })
}

/// Sends the tarball, taildrop first and scp on any failure.
///
/// Returns the transport that worked and the path the tarball landed on at the
/// far end. A missing `tailscale` binary is a spawn error, which is the same
/// decision point as `tailscale file cp` exiting non-zero: fall back.
fn send(
    run: &mut Run,
    tarball: &Path,
    name: &str,
    target: &str,
) -> Result<(&'static str, String), TransferError> {
    let taildrop = StepSpec::new("transfer-taildrop", "tailscale")
        .args(["file", "cp"])
        .arg(tarball.display().to_string())
        .arg(format!("{target}:"))
        .timeout(SEND_TIMEOUT);
    match run.exec(&taildrop) {
        Ok(o) if o.success() => {
            let remote_path = format!("~/{name}");
            record_send(run, "taildrop", &remote_path)?;
            return Ok(("taildrop", remote_path));
        }
        Ok(_) => {}
        // A log-write failure is real and must not be masked by the fallback.
        Err(e @ ExecError::Trace(_)) => return Err(e.into()),
        // Anything else — a missing binary, a timeout, a non-zero exit — is the
        // reason the fallback exists.
        Err(_) => {}
    }

    let scp = StepSpec::new("transfer-scp", "scp")
        .arg(tarball.display().to_string())
        .arg(format!("{target}:/tmp/"))
        .timeout(SEND_TIMEOUT);
    let outcome = run.exec(&scp)?;
    if !outcome.success() {
        return Err(TransferError::SendFailed(
            target.to_string(),
            format!(
                "taildrop and scp both failed; scp said: {}",
                outcome.stderr_trimmed()
            ),
        ));
    }
    let remote_path = format!("/tmp/{name}");
    record_send(run, "scp", &remote_path)?;
    Ok(("scp", remote_path))
}

/// Records which transport carried the tarball and where it landed.
fn record_send(run: &mut Run, method: &str, remote_path: &str) -> Result<(), TransferError> {
    let seq = run.next_seq();
    run.record_step(
        StepRecord::new(seq, "transfer-send", StepStatus::Ok, &[])
            .with_detail(json!({"method": method, "remote_path": remote_path})),
    )?;
    Ok(())
}

/// Removes the local tarball and records the attempt.
fn cleanup(run: &mut Run, tarball: &Path) -> Result<(), TransferError> {
    let seq = run.next_seq();
    let path = tarball.display().to_string();
    let (status, detail) = match std::fs::remove_file(tarball) {
        Ok(()) => (StepStatus::Ok, json!({"removed": true, "path": path})),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (
            StepStatus::Ok,
            json!({"removed": false, "path": path, "reason": "already gone"}),
        ),
        Err(e) => (
            StepStatus::Error,
            json!({"removed": false, "path": path, "error": e.to_string()}),
        ),
    };
    run.record_step(StepRecord::new(seq, "transfer-cleanup", status, &[]).with_detail(detail))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::redact::Redactor;
    use crate::registry::tests::sample;
    use crate::trace::{RunMode, TraceEvent, TraceLog};

    fn run(dir: &Path) -> Run {
        let log = TraceLog::open(dir.join("t.jsonl")).unwrap();
        Run::start(
            log,
            RunMode::Live,
            Some("portfolio".into()),
            &["swapdock".to_string()],
            Redactor::new(),
        )
        .unwrap()
    }

    #[test]
    fn transfer_refuses_an_invalid_release_before_any_step() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut r = run(dir.path());

        let err = transfer(&mut r, &sample(), "main; evil", "hetzner", dir.path()).unwrap_err();
        assert!(matches!(err, TransferError::InvalidRelease(_)), "{err:?}");

        let read = TraceLog::read(dir.path().join("t.jsonl")).unwrap();
        assert!(
            !read
                .events
                .iter()
                .any(|e| matches!(e, TraceEvent::Step { .. })),
            "validation must run before the first spawn"
        );
    }

    #[test]
    fn transfer_refuses_without_an_image_repo() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut r = run(dir.path());
        let mut app = sample();
        app.image_repo = None;

        let err = transfer(&mut r, &app, "9c1f2ab", "hetzner", dir.path()).unwrap_err();
        assert!(matches!(err, TransferError::NoImageRepo(_)), "{err:?}");

        let read = TraceLog::read(dir.path().join("t.jsonl")).unwrap();
        assert!(
            !read
                .events
                .iter()
                .any(|e| matches!(e, TraceEvent::Step { .. })),
            "a missing image_repo must be refused before any command"
        );
    }
}
