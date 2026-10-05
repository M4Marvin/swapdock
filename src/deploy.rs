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
//! swap:    lock → resolve → pull → green-start → health-wait → render+validate
//!          → apply → probe-front → drain → stop-old → registry-commit
//! replace: lock → resolve → pull → render+validate → apply → stop-old
//!          → start-new → health-wait → probe-front → registry-commit
//! ```
//!
//! `rollback` is not a third strategy: it runs the app's own strategy with the
//! previous release, health gate included. Rolling back through the same gate is
//! what makes it trustworthy.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;
use thiserror::Error;

use crate::apply::{self, ApplyPaths};
use crate::compose;
use crate::docker;
use crate::exec::StepSpec;
use crate::git;
use crate::health::{self, HealthError};
use crate::lock::{self, Lock, LockError};
use crate::registry::{self, App};
use crate::render;
use crate::trace::{Run, RunMode, StepRecord};

/// Seconds to let old workers drain after the flip before stopping them.
/// nginx closes an old keepalive connection after one more request, so ordinary
/// traffic drains in about a round trip; this covers the tail.
pub const DRAIN_SECS: u64 = 15;

/// Environment variable carrying the image reference into compose.
pub const IMAGE_ENV: &str = "IMAGE";

/// Step names of the swap strategy, in order. The run log groups by these, so
/// they are stable identifiers covered by a test.
pub fn swap_steps() -> Vec<&'static str> {
    vec![
        "lock",
        "resolve-release",
        "pull",
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
pub struct Ctx {
    pub registry_dir: PathBuf,
    pub state_dir: PathBuf,
    pub lock_dir: PathBuf,
    pub apply_paths: ApplyPaths,
    pub drain_secs: u64,
    pub lock_timeout: Duration,
}

impl Default for Ctx {
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

    #[error("could not take the swapdock lock: {0}")]
    Lock(#[from] LockError),

    #[error("refused to swapdock into a broken estate: {0}")]
    Estate(String),

    #[error("sync failed: {0}")]
    Sync(String),

    #[error("could not run a swapdock step: {0}")]
    Exec(#[from] crate::exec::ExecError),

    #[error("could not pull {image_ref}: {stderr}")]
    PullFailed { image_ref: String, stderr: String },

    #[error("could not write a file: {0}")]
    Io(String),

    #[error("could not write to the run log: {0}")]
    Trace(#[from] anyhow::Error),
}

/// Decides which commit to swapdock: the flag wins, then the recorded release.
pub fn resolve_release(app: &App, flag: Option<&str>) -> Result<String, DeployError> {
    if let Some(sha) = flag {
        return Ok(sha.to_string());
    }
    app.release.clone().ok_or(DeployError::NoRelease)
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
pub fn deploy(run: &mut Run, ctx: &Ctx, app: &mut App, release: &str) -> Result<(), DeployError> {
    match app.strategy {
        registry::Strategy::Swap => run_swap(run, ctx, app, release),
        registry::Strategy::Replace => run_replace(run, ctx, app, release),
    }
}

/// Rolls back by deploying the previous release through the same strategy.
pub fn rollback(run: &mut Run, ctx: &Ctx, app: &mut App) -> Result<String, DeployError> {
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
pub fn run_swap(run: &mut Run, ctx: &Ctx, app: &mut App, release: &str) -> Result<(), DeployError> {
    let _app_lock = hold(run, ctx, &lock::app_lock_name(&app.name))?;
    refuse_if_broken(run, ctx)?;

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

    let green_name = format!("{}-green-{}", app.compose_svc, short_sha(release));
    start_green(run, ctx, app, release, green_port, &green_name)?;

    health::gate_container(
        run,
        "health-wait",
        green_port,
        &health::probe_path(app_health_url(app)),
    )?;

    apply_new_state(run, ctx, app, green_port)?;

    probe_front(run, app)?;

    drain(run, ctx)?;

    stop_publishing(run, app.live_port)?;

    commit_release(run, ctx, app, release.to_string(), green_port)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// replace
// ---------------------------------------------------------------------------
//
/// Deploys by stopping the old container and starting the new on the same port.
pub fn run_replace(
    run: &mut Run,
    ctx: &Ctx,
    app: &mut App,
    release: &str,
) -> Result<(), DeployError> {
    let _app_lock = hold(run, ctx, &lock::app_lock_name(&app.name))?;
    refuse_if_broken(run, ctx)?;

    let port = app.live_port.ok_or_else(|| {
        DeployError::Estate(format!(
            "{} has no live_port yet; a first swapdock must use swap",
            app.name
        ))
    })?;

    pull_image(run, app, release)?;

    // The port does not change, so the rendered config is identical. Skip the
    // reload rather than bounce every worker for no reason.
    if config_changed(ctx, app, port)? {
        apply_new_state(run, ctx, app, port)?;
    } else {
        record_skip(run, "apply-skipped", "rendered config identical; no reload")?;
    }

    stop_publishing(run, Some(port))?;

    start_live(run, app, release, port)?;

    health::gate_container(
        run,
        "health-wait",
        port,
        &health::probe_path(app_health_url(app)),
    )?;

    probe_front(run, app)?;

    commit_release(run, ctx, app, release.to_string(), port)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// shared steps
// ---------------------------------------------------------------------------

fn app_health_url(app: &App) -> &str {
    app.health_url.as_deref().unwrap_or("http://127.0.0.1/")
}

/// Takes a lock, or records the intent in dry-run mode.
fn hold(run: &mut Run, ctx: &Ctx, name: &str) -> Result<Option<Lock>, DeployError> {
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

/// Refuses to swapdock into an estate that does not validate.
fn refuse_if_broken(run: &mut Run, ctx: &Ctx) -> Result<(), DeployError> {
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
        record_skip(
            run,
            "pull",
            &format!("{image_ref} is local; assuming present"),
        )?;
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
    ctx: &Ctx,
    app: &App,
    release: &str,
    green_port: u16,
    green_name: &str,
) -> Result<(), DeployError> {
    let override_path = green_override_path(&ctx.state_dir, &app.name);
    let text = green_override(green_name, &app.compose_svc);
    write_state_file(run, &override_path, &text)?;

    let compose_file = app.compose_dir.join("docker-compose.yml");
    let project = format!("{}-green", app.compose_project());
    let image = app.image_ref(release).unwrap_or_default();
    let port_var = app.env_name.clone();
    let mut spec = compose::up_detached(
        &compose_file,
        &project,
        &[override_path.as_path()],
        &app.compose_svc,
    );
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
    let compose_file = app.compose_dir.join("docker-compose.yml");
    let project = app.compose_project();
    let image = app.image_ref(release).unwrap_or_default();
    let spec = compose::up_detached(&compose_file, &project, &[], &app.compose_svc)
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
    ctx: &Ctx,
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
fn config_changed(ctx: &Ctx, app: &App, port: u16) -> Result<bool, DeployError> {
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
    for host in &app.hostnames {
        let code =
            health::http_status_with_host("127.0.0.1", app.front_port, "/", host).map_err(|e| {
                HealthError::ProbeFailed {
                    url: format!("http://127.0.0.1:{}/ (Host: {host})", app.front_port),
                    attempts: 1,
                    last: e,
                }
            })?;
        let seq = run.next_seq();
        run.record_step(
            StepRecord::new(seq, "probe-front", crate::trace::StepStatus::Ok, &[]).with_detail(
                json!({
                    "host": host,
                    "front_port": app.front_port,
                    "status": code,
                }),
            ),
        )?;
        if !(200..400).contains(&code) {
            return Err(HealthError::ProbeFailed {
                url: format!("http://127.0.0.1:{}/ (Host: {host})", app.front_port),
                attempts: 1,
                last: format!("HTTP {code}"),
            }
            .into());
        }
    }
    Ok(())
}

/// Lets old workers drain before stopping the old container.
fn drain(run: &mut Run, ctx: &Ctx) -> Result<(), DeployError> {
    let spec = StepSpec::new("drain-wait", "sleep").arg(ctx.drain_secs.to_string());
    run.exec(&spec)?;
    Ok(())
}

/// Stops and removes every container publishing `port`, if any.
fn stop_publishing(run: &mut Run, port: Option<u16>) -> Result<(), DeployError> {
    let Some(port) = port else {
        record_skip(run, "stop-old", "no previous container; first swapdock")?;
        return Ok(());
    };
    let found = run.exec(&docker::ps_publishing(port))?;
    let names: Vec<&str> = found
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
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
        run.exec(&docker::stop(name))?;
        run.exec(&docker::rm(name))?;
    }
    Ok(())
}

/// Advances the registry and writes it back atomically.
fn commit_release(
    run: &mut Run,
    ctx: &Ctx,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::tests::sample;

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
        assert_eq!(resolve_release(&app, Some("abc1234")).unwrap(), "abc1234");
        assert_eq!(resolve_release(&app, None).unwrap(), "9c1f2ab");

        let mut bare = sample();
        bare.release = None;
        assert!(matches!(
            resolve_release(&bare, None).unwrap_err(),
            DeployError::NoRelease
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
    fn short_sha_never_panics_on_short_input() {
        assert_eq!(short_sha("9c1f2ab"), "9c1f2ab");
        assert_eq!(
            short_sha("9c1f2ab0123456789abcdef0123456789abcdef"),
            "9c1f2ab"
        );
        assert_eq!(short_sha("abc"), "abc");
        assert_eq!(short_sha(""), "");
    }
}
