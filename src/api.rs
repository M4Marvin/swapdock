//! HTTP API surface.
//!
//! Every mutating CLI command maps to one endpoint. Reads are synchronous;
//! mutations record the plan immediately, spawn the work in a thread, and
//! report the run id — the JSONL run log stays the observability query:
//! poll `GET /runs/{id}` for steps, or tail them live from `GET /events`.
//!
//! Routes are served under `/api` and, for compatibility with existing clients,
//! also at the legacy un-prefixed paths: `/api/health` and `/health` are the
//! same handler.
//!
//! Bind loopback by default; put authentication in front (nginx basic-auth,
//! Tailscale ACL) before exposing this anywhere.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream;
use serde::Deserialize;

use crate::deploy::{self, Ctx};
use crate::registry::{App, Loaded, Problem};
use crate::trace::{Run, RunMode, RunStatus, TraceEvent, TraceLog};
use crate::{apply, builder, exec, git, health, validator};

/// How often the SSE handler re-reads the run log for new lines.
const EVENTS_POLL: Duration = Duration::from_millis(300);

/// How long an SSE stream may go without a new event for its run before it
/// closes. A run that stopped writing without a `run_end` would otherwise hold
/// the connection open forever.
const EVENTS_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Upper bound on a `git rev-parse` for `/apps/{name}/latest`. A local ref
/// lookup is instantaneous; the bound exists so a wedged checkout cannot hold
/// an HTTP request open indefinitely.
const LATEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Everything the handlers need that the CLI flags used to carry.
#[derive(Debug, Clone)]
pub struct ServerState {
    pub registry: PathBuf,
    pub trace: PathBuf,
    pub tunnel_config: Option<PathBuf>,
    pub target: PathBuf,
    pub main_config: PathBuf,
    pub pid_file: PathBuf,
    pub nginx_bin: PathBuf,
    pub state_dir: PathBuf,
    pub lock_dir: PathBuf,
    pub drain_secs: u64,
}

type Shared = Arc<ServerState>;

pub fn router(state: ServerState) -> Router {
    let state: Shared = Arc::new(state);
    let api = api_router();
    // Mount the same routes under `/api` (the frontend contract) and at the
    // legacy un-prefixed paths, so existing clients keep working.
    Router::new()
        .nest("/api", api.clone())
        .merge(api)
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(state)
}

/// The routes, without state or layers, so they can be mounted at both paths.
fn api_router() -> Router<Shared> {
    Router::new()
        .route("/health", get(health))
        .route("/apps", get(list_apps).post(init_app))
        .route("/apps/{name}", get(show_app))
        .route("/apps/{name}/status", get(app_status))
        .route("/apps/{name}/latest", get(app_latest))
        .route("/apps/{name}/deploys", post(deploy_app))
        .route("/apps/{name}/rollback", post(rollback_app))
        .route("/apps/{name}/build", post(build_app))
        .route("/apps/{name}/sync", post(sync_app))
        .route("/apps/{name}/verify", get(verify_app))
        .route("/validate", post(validate))
        .route("/render", get(render_config))
        .route("/apply", post(apply_config))
        .route("/events", get(run_events))
        .route("/runs", get(list_runs))
        .route("/runs/{id}", get(show_run))
        .route("/runs/{id}/resume", get(resume_run))
}

/// Starts the server. Returns when the listener fails to bind.
pub async fn serve(state: ServerState, bind: &str) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, router(state)).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// health
// ---------------------------------------------------------------------------

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "ok"}))
}

// ---------------------------------------------------------------------------
// registry reads
// ---------------------------------------------------------------------------

fn load(state: &ServerState) -> anyhow::Result<Loaded> {
    crate::registry::load_dir(&state.registry).map_err(|e| e.into())
}

/// As `validate_registry` does: load problems plus cross-app checks.
fn collect_problems(state: &ServerState, loaded: &Loaded) -> Vec<Problem> {
    let apps = loaded.sorted();
    let mut problems: Vec<Problem> = loaded.problems.clone();
    for app in &apps {
        problems.extend(app.problems());
    }
    let mut routes = validator::TunnelRoutes::new();
    match &state.tunnel_config {
        None => problems.push(Problem::warning(
            "tunnel-not-checked",
            None,
            format!(
                "no tunnel_config given, so {} route cross-checks were skipped",
                apps.len()
            ),
        )),
        Some(path) => match crate::tunnel::read_config(path) {
            Ok(parsed) => {
                problems.extend(parsed.problems);
                routes = parsed.routes;
            }
            Err(e) => problems.push(Problem::warning(
                "tunnel-config-unreadable",
                None,
                format!("{}: {e}; route cross-checks were skipped", path.display()),
            )),
        },
    }
    problems.extend(validator::validate(&apps, &routes));
    validator::sort_problems(&mut problems);
    problems
}

async fn list_apps(State(state): State<Shared>) -> impl IntoResponse {
    match load(&state) {
        Ok(loaded) => {
            let apps: Vec<serde_json::Value> = loaded
                .sorted()
                .into_iter()
                .map(|app| {
                    serde_json::json!({
                        "app": app,
                        "problems": app.problems(),
                    })
                })
                .collect();
            (StatusCode::OK, Json(serde_json::Value::Array(apps))).into_response()
        }
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn show_app(State(state): State<Shared>, Path(name): Path<String>) -> impl IntoResponse {
    match load(&state) {
        Ok(loaded) => match loaded.apps.iter().find(|a| a.name == name) {
            Some(app) => (
                StatusCode::OK,
                Json(serde_json::json!({"app": app, "problems": app.problems()})),
            )
                .into_response(),
            None => error_response(StatusCode::NOT_FOUND, &format!("no app named {name:?}")),
        },
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// live status / latest release
// ---------------------------------------------------------------------------

/// Probes the live container's own health endpoint and reports the verdict.
///
/// This answers "is it serving right now?", which is a different question from
/// "did the last swapdock succeed?" — the registry's `live_port` and the process
/// behind it can disagree. Only the path of `health_url` is used; host and port
/// come from the live port, exactly as the deploy gate does.
async fn app_status(State(state): State<Shared>, Path(name): Path<String>) -> impl IntoResponse {
    let app = match load_app(&state, &name) {
        Ok(a) => a,
        Err(e) => return error_response(StatusCode::NOT_FOUND, &e.to_string()),
    };
    let Some(live_port) = app.live_port else {
        return (
            StatusCode::OK,
            Json(serde_json::json!({"up": false, "status": null, "live_port": null})),
        )
            .into_response();
    };
    let health_url = app.health_url.as_deref().unwrap_or("http://127.0.0.1/");
    let path = match health::checked_probe_path(health_url) {
        Ok(p) => p,
        Err(_) => return error_response(StatusCode::CONFLICT, "invalid health path"),
    };
    // The probe is blocking socket I/O; keep it off the async workers.
    let (up, status) = match tokio::task::spawn_blocking(move || {
        health::http_status("127.0.0.1", live_port, &path)
    })
    .await
    {
        Ok(Ok(code)) => ((200..300).contains(&code), Some(code)),
        // A transport failure is a down verdict, not a 5xx from this API: the
        // API is fine, the app is not answering. A panicked probe is the same.
        Ok(Err(_)) | Err(_) => (false, None),
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({"up": up, "status": status, "live_port": live_port})),
    )
        .into_response()
}

/// The release that would be deployed: `origin/<branch>`, then local `HEAD`.
///
/// Read-only: it resolves refs through the exec chokepoint without opening a
/// run, so a query never pollutes the run log. `source` names which ref
/// answered.
async fn app_latest(State(state): State<Shared>, Path(name): Path<String>) -> impl IntoResponse {
    let app = match load_app(&state, &name) {
        Ok(a) => a,
        Err(e) => return error_response(StatusCode::NOT_FOUND, &e.to_string()),
    };
    let Some(repo) = app.repo.clone() else {
        return error_response(
            StatusCode::CONFLICT,
            &format!(
                "{} has no repo recorded; set repo in the registry to resolve a release",
                app.name
            ),
        );
    };
    // `git rev-parse` is blocking; run it on the blocking pool so it cannot
    // stall an async worker.
    let app_for_job = app.clone();
    match tokio::task::spawn_blocking(move || resolve_latest(&app_for_job, &repo)).await {
        Ok(Ok((release, source))) => (
            StatusCode::OK,
            Json(serde_json::json!({"release": release, "source": source})),
        )
            .into_response(),
        // Nothing resolves: a conflict with the requested state, not a fault.
        Ok(Err(LatestError::NotResolved(e))) => error_response(StatusCode::CONFLICT, &e),
        // The machinery failed: this API's fault, so a 5xx.
        Ok(Err(LatestError::Infra(e))) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &e),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("latest query failed: {e}"),
        ),
    }
}

/// Why `/latest` could not name a release.
enum LatestError {
    /// The repo or ref is not there: a 409.
    NotResolved(String),
    /// Spawning or timing out the lookup: a 500.
    Infra(String),
}

/// Runs the two `git rev-parse --short` probes, without a run log.
///
/// The remote ref follows the app's configured branch, not a hardcoded `main`.
fn resolve_latest(app: &App, repo: &std::path::Path) -> Result<(String, String), LatestError> {
    let branch = app.git_branch();
    let remote_ref = format!("origin/{branch}");

    let remote =
        exec::run_capture(&git::rev_parse_short(repo, &remote_ref).timeout(LATEST_TIMEOUT))
            .map_err(|e| LatestError::Infra(e.to_string()))?;
    if remote.success() {
        let sha = remote.stdout_trimmed();
        if !sha.is_empty() {
            return Ok((sha.to_string(), remote_ref));
        }
    }

    let head = exec::run_capture(&git::rev_parse_short(repo, "HEAD").timeout(LATEST_TIMEOUT))
        .map_err(|e| LatestError::Infra(e.to_string()))?;
    if head.success() {
        let sha = head.stdout_trimmed();
        if !sha.is_empty() {
            return Ok((sha.to_string(), "HEAD".to_string()));
        }
    }

    Err(LatestError::NotResolved(format!(
        "could not resolve a release for {}: neither {remote_ref} nor HEAD resolves in {}",
        app.name,
        repo.display()
    )))
}

#[derive(Deserialize)]
struct InitBody {
    name: String,
    #[serde(default = "default_kind")]
    kind: String,
}

fn default_kind() -> String {
    "container".to_string()
}

async fn init_app(State(state): State<Shared>, Json(body): Json<InitBody>) -> impl IntoResponse {
    use std::io::Write;
    // Reuse the CLI's template verbatim: one validated template, not two.
    let text = match crate::cli::init_template(&body.name, &body.kind) {
        Ok(t) => t,
        Err(e) => return error_response(StatusCode::BAD_REQUEST, &e.to_string()),
    };
    let dir = &state.registry;
    if let Err(e) = std::fs::create_dir_all(dir) {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    let path = dir.join(format!("{}.toml", body.name));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut f) => match f.write_all(text.as_bytes()) {
            Ok(()) => (
                StatusCode::CREATED,
                Json(serde_json::json!({"path": path.display().to_string()})),
            )
                .into_response(),
            Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        },
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            error_response(StatusCode::CONFLICT, &format!("{} exists", path.display()))
        }
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// validate / render / apply
// ---------------------------------------------------------------------------

async fn validate(State(state): State<Shared>) -> impl IntoResponse {
    match load(&state) {
        Ok(loaded) => {
            let apps = loaded.sorted();
            let problems = collect_problems(&state, &loaded);
            let (errors, warnings) = validator::counts(&problems);
            let status = if errors > 0 {
                StatusCode::CONFLICT
            } else {
                StatusCode::OK
            };
            (
                status,
                Json(serde_json::json!({
                    "apps": apps.len(),
                    "errors": errors,
                    "warnings": warnings,
                    "problems": problems,
                })),
            )
                .into_response()
        }
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

#[derive(Deserialize)]
struct RenderQuery {
    app: Option<String>,
}

async fn render_config(
    State(state): State<Shared>,
    Query(q): Query<RenderQuery>,
) -> impl IntoResponse {
    match load(&state) {
        Ok(loaded) => {
            let apps = loaded.sorted();
            let selected: Vec<App> = match &q.app {
                None => apps,
                Some(name) => {
                    let found: Vec<App> = apps.into_iter().filter(|a| &a.name == name).collect();
                    if found.is_empty() {
                        return error_response(
                            StatusCode::NOT_FOUND,
                            &format!("no app named {name:?}"),
                        );
                    }
                    found
                }
            };
            (
                StatusCode::OK,
                [(
                    axum::http::header::CONTENT_TYPE,
                    "text/plain; charset=utf-8",
                )],
                crate::render::render(&selected),
            )
                .into_response()
        }
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

#[derive(Deserialize, Default)]
struct ApplyBody {
    #[serde(default)]
    dry_run: bool,
}

async fn apply_config(
    State(state): State<Shared>,
    Json(body): Json<Option<ApplyBody>>,
) -> impl IntoResponse {
    let dry = body.as_ref().map(|b| b.dry_run).unwrap_or(false);
    let loaded = match load(&state) {
        Ok(l) => l,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let problems = collect_problems(&state, &loaded);
    let (errors, _) = validator::counts(&problems);
    if errors > 0 {
        // Keep the structured problem list, but also carry the standard
        // `{"error": ...}` field every other non-2xx response uses.
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": format!("{errors} error(s) in the registry; nothing was applied"),
                "errors": errors,
                "problems": problems,
            })),
        )
            .into_response();
    }
    let apps = loaded.sorted();
    let state2 = state.clone();
    spawn_job(&state, None, dry, move |run| {
        let text = crate::render::render(&apps);
        let paths = crate::apply::ApplyPaths {
            target: state2.target.clone(),
            main_config: state2.main_config.clone(),
            pid_file: Some(state2.pid_file.clone()),
            nginx_bin: state2.nginx_bin.clone(),
        };
        match apply::apply(run, &text, &paths) {
            Ok(outcome) => Ok(serde_json::json!({"reloaded": outcome.reloaded, "backup": outcome.backup.map(|p| p.display().to_string())})),
            Err(e) => Err(e.to_string()),
        }
    })
    .into_response()
}

// ---------------------------------------------------------------------------
// mutations: deploy / rollback / build / sync
// ---------------------------------------------------------------------------

fn ctx_for(state: &ServerState) -> Ctx {
    Ctx {
        registry_dir: state.registry.clone(),
        state_dir: state.state_dir.clone(),
        lock_dir: state.lock_dir.clone(),
        apply_paths: crate::apply::ApplyPaths {
            target: state.target.clone(),
            main_config: state.main_config.clone(),
            pid_file: Some(state.pid_file.clone()),
            nginx_bin: state.nginx_bin.clone(),
        },
        drain_secs: state.drain_secs,
        lock_timeout: crate::lock::ACQUIRE_TIMEOUT,
    }
}

/// Opens the log, starts the run, detaches the work into a thread, and
/// returns the run id to the caller.
fn spawn_job<F>(state: &ServerState, app: Option<&str>, dry_run: bool, work: F) -> impl IntoResponse
where
    F: FnOnce(&mut Run) -> Result<serde_json::Value, String> + Send + 'static,
{
    let argv: Vec<String> = vec!["swapdock".into(), "api".into()];
    let mode = if dry_run {
        RunMode::DryRun
    } else {
        RunMode::Live
    };
    let log = match TraceLog::open(&state.trace) {
        Ok(l) => l,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let mut run = match Run::start(
        log,
        mode,
        app.map(str::to_string),
        &argv,
        crate::redact::Redactor::new(),
    ) {
        Ok(r) => r,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let run_id = run.id().to_string();
    let run_id2 = run_id.clone();
    std::thread::spawn(move || {
        // A panic in the job must not leave a dangling run in the log: catch it,
        // finish the run as failed, then let the panic continue to the thread
        // boundary. `AssertUnwindSafe` is sound here because the run is closed
        // before we resume the unwind.
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&mut run)));
        match caught {
            Ok(result) => {
                let status = match &result {
                    Ok(_) if dry_run => RunStatus::DryRun,
                    Ok(_) => RunStatus::Ok,
                    Err(_) => RunStatus::Failed,
                };
                let _ = run.finish(status);
                if let Err(e) = &result {
                    tracing::warn!(run = %run_id2, "job failed: {e}");
                }
            }
            Err(payload) => {
                let _ = run.finish(RunStatus::Failed);
                tracing::error!(run = %run_id2, "job panicked; run closed as failed");
                std::panic::resume_unwind(payload);
            }
        }
    });
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({"run_id": run_id})),
    )
        .into_response()
}

#[derive(Deserialize, Default)]
struct DeployBody {
    release: Option<String>,
    #[serde(default)]
    dry_run: bool,
}

async fn deploy_app(
    State(state): State<Shared>,
    Path(name): Path<String>,
    Json(body): Json<Option<DeployBody>>,
) -> impl IntoResponse {
    let body = body.unwrap_or_default();
    let app = match load_app(&state, &name) {
        Ok(a) => a,
        Err(e) => return error_response(StatusCode::NOT_FOUND, &e.to_string()),
    };
    let release = match deploy::resolve_release(&app, body.release.as_deref()) {
        Ok(r) => r,
        Err(e) => return error_response(StatusCode::CONFLICT, &e.to_string()),
    };
    let ctx = ctx_for(&state);
    let mut deploy_app = app;
    spawn_job(&state, Some(&name), body.dry_run, move |run| {
        deploy::deploy(run, &ctx, &mut deploy_app, &release)
            .map(|()| serde_json::json!({}))
            .map_err(|e| e.to_string())
    })
    .into_response()
}

#[derive(Deserialize, Default)]
struct DryBody {
    #[serde(default)]
    dry_run: bool,
}

async fn rollback_app(
    State(state): State<Shared>,
    Path(name): Path<String>,
    Json(body): Json<Option<DryBody>>,
) -> impl IntoResponse {
    let dry = body.as_ref().map(|b| b.dry_run).unwrap_or(false);
    let mut app = match load_app(&state, &name) {
        Ok(a) => a,
        Err(e) => return error_response(StatusCode::NOT_FOUND, &e.to_string()),
    };
    let ctx = ctx_for(&state);
    spawn_job(&state, Some(&name), dry, move |run| {
        deploy::rollback(run, &ctx, &mut app)
            .map(|old| serde_json::json!({"rolled_back_to": old}))
            .map_err(|e| e.to_string())
    })
    .into_response()
}

#[derive(Deserialize, Default)]
struct BuildBody {
    release: Option<String>,
    build_host: Option<String>,
    #[serde(default)]
    dry_run: bool,
}

async fn build_app(
    State(state): State<Shared>,
    Path(name): Path<String>,
    Json(body): Json<Option<BuildBody>>,
) -> impl IntoResponse {
    let body = body.unwrap_or_default();
    let app = match load_app(&state, &name) {
        Ok(a) => a,
        Err(e) => return error_response(StatusCode::NOT_FOUND, &e.to_string()),
    };
    let release = body.release.clone().or(app.release.clone());
    let Some(release) = release else {
        return error_response(
            StatusCode::CONFLICT,
            "no release: pass release, or sync the source first",
        );
    };
    if !crate::registry::is_valid_release_arg(&release) {
        return error_response(
            StatusCode::CONFLICT,
            &format!("invalid release {release:?}: must be a commit SHA (hex, 4-64 chars)"),
        );
    }
    let env_host = std::env::var("SWAPDOCK_BUILD_HOST").ok();
    let host = builder::resolve_host(body.build_host.as_deref(), env_host.as_deref(), &app);
    spawn_job(&state, Some(&name), body.dry_run, move |run| {
        let built = if builder::is_local(&host) {
            let work_parent = std::env::temp_dir().join("swapdock-build");
            builder::build_local(run, &app, &release, &work_parent)
        } else {
            builder::build_ssh(run, &app, &release, &host)
        }
        .map_err(|e| e.to_string())?;
        Ok(serde_json::json!({"image_ref": built.image_ref, "release": built.release}))
    })
    .into_response()
}

async fn sync_app(
    State(state): State<Shared>,
    Path(name): Path<String>,
    Json(body): Json<Option<DryBody>>,
) -> impl IntoResponse {
    let dry = body.as_ref().map(|b| b.dry_run).unwrap_or(false);
    let app = match load_app(&state, &name) {
        Ok(a) => a,
        Err(e) => return error_response(StatusCode::NOT_FOUND, &e.to_string()),
    };
    spawn_job(&state, Some(&name), dry, move |run| {
        deploy::sync_repo(run, &app)
            .map(|sha| serde_json::json!({"head": sha}))
            .map_err(|e| e.to_string())
    })
    .into_response()
}

fn load_app(state: &ServerState, name: &str) -> anyhow::Result<App> {
    let loaded = load(state)?;
    match loaded.apps.iter().find(|a| a.name == name) {
        Some(app) => Ok(app.clone()),
        None => anyhow::bail!("no app named {name:?}"),
    }
}

#[derive(Deserialize)]
struct VerifyQuery {
    since: String,
    access_log: Option<PathBuf>,
}

async fn verify_app(
    State(state): State<Shared>,
    Path(name): Path<String>,
    Query(q): Query<VerifyQuery>,
) -> impl IntoResponse {
    let app = match load_app(&state, &name) {
        Ok(a) => a,
        Err(e) => return error_response(StatusCode::NOT_FOUND, &e.to_string()),
    };
    let since_ms = match crate::id::RunId::parse(&q.since) {
        Some(id) => {
            let read = match TraceLog::read(&state.trace) {
                Ok(r) => r,
                Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
            };
            match read.events.iter().find_map(|e| match e {
                TraceEvent::RunStart { run_id, ts_ms, .. } if run_id == id.as_str() => Some(*ts_ms),
                _ => None,
            }) {
                Some(ms) => ms,
                None => {
                    return error_response(
                        StatusCode::NOT_FOUND,
                        &format!("no run {id} in the log"),
                    );
                }
            }
        }
        None => match crate::time::parse_rfc3339(&q.since) {
            Some(ms) => ms,
            None => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    &format!(
                        "{:?} is neither a run id nor an RFC 3339 timestamp",
                        q.since
                    ),
                );
            }
        },
    };
    let log_path = q
        .access_log
        .clone()
        .unwrap_or_else(|| PathBuf::from(crate::verify::DEFAULT_ACCESS_LOG));
    match crate::verify::read_lines(&log_path) {
        Ok(lines) => {
            let report = crate::verify::verify(&app.hostnames, since_ms, lines.into_iter());
            (StatusCode::OK, Json(serde_json::to_value(report).unwrap())).into_response()
        }
        Err(e) => error_response(StatusCode::CONFLICT, &e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// runs
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct RunsQuery {
    limit: Option<usize>,
}

async fn list_runs(State(state): State<Shared>, Query(q): Query<RunsQuery>) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(20);
    match TraceLog::read(&state.trace) {
        Ok(read) => {
            let mut summaries: Vec<serde_json::Value> = Vec::new();
            for ev in &read.events {
                match ev {
                    TraceEvent::RunStart {
                        run_id,
                        ts,
                        mode,
                        app,
                        ..
                    } => summaries.push(serde_json::json!({
                        "run_id": run_id,
                        "started": ts,
                        "app": app,
                        // Serialize rather than `format!("{mode:?}")`: the
                        // frontend contract is the serde spelling ("live").
                        "status": serde_json::to_value(mode).unwrap_or(serde_json::Value::Null),
                        "steps": 0,
                        "non_ok": 0,
                    })),
                    TraceEvent::Step { run_id, status, .. } => {
                        if let Some(cur) = summaries
                            .iter_mut()
                            .rev()
                            .find(|s| s["run_id"].as_str() == Some(run_id.as_str()))
                        {
                            cur["steps"] =
                                serde_json::json!(cur["steps"].as_u64().unwrap_or(0) + 1);
                            if status.is_failure() {
                                cur["non_ok"] =
                                    serde_json::json!(cur["non_ok"].as_u64().unwrap_or(0) + 1);
                            }
                        }
                    }
                    TraceEvent::RunEnd { run_id, status, .. } => {
                        if let Some(cur) = summaries
                            .iter_mut()
                            .rev()
                            .find(|s| s["run_id"].as_str() == Some(run_id.as_str()))
                        {
                            cur["status"] =
                                serde_json::to_value(status).unwrap_or(serde_json::Value::Null);
                        }
                    }
                }
            }
            summaries.reverse();
            summaries.truncate(limit);
            (StatusCode::OK, Json(serde_json::Value::Array(summaries))).into_response()
        }
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn show_run(State(state): State<Shared>, Path(id): Path<String>) -> impl IntoResponse {
    let read = match TraceLog::read(&state.trace) {
        Ok(r) => r,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let events: Vec<&TraceEvent> = read
        .events
        .iter()
        .filter(|e| match e {
            TraceEvent::RunStart { run_id, .. }
            | TraceEvent::Step { run_id, .. }
            | TraceEvent::RunEnd { run_id, .. } => run_id == &id,
        })
        .collect();
    if events.is_empty() {
        return error_response(StatusCode::NOT_FOUND, &format!("no run {id:?} in the log"));
    }
    match serde_json::to_value(events) {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn resume_run(State(state): State<Shared>, Path(id): Path<String>) -> impl IntoResponse {
    let Some(id) = crate::id::RunId::parse(&id) else {
        return error_response(StatusCode::BAD_REQUEST, "not a valid run id");
    };
    match TraceLog::resume_point(&state.trace, &id) {
        Ok(Some(p)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "run_id": p.run_id.as_str(),
                "last_seq": p.last_seq,
                "last_step": p.last_step,
                "last_status": format!("{:?}", p.last_status),
                "non_ok_steps": p.non_ok_steps,
                "ended": p.ended.map(|s| format!("{s:?}")),
            })),
        )
            .into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, &format!("no steps for run {id}")),
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// events (SSE)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct EventsQuery {
    run: Option<String>,
}

/// `GET /events?run=<id>` — streams one run's trace events as they appear.
///
/// The append-only JSONL log is the source of truth, so this is a tail, not a
/// second bus: each poll reads the bytes appended since the last one, parses
/// complete lines, and forwards the ones tagged with `run`. When a `run_end`
/// arrives the stream sends an explicit `done` event and closes, so a client
/// never has to poll `/runs/{id}` to learn that the run finished. A run that
/// stops writing without an end closes after [`EVENTS_IDLE_TIMEOUT`].
async fn run_events(
    State(state): State<Shared>,
    Query(q): Query<EventsQuery>,
) -> axum::response::Response {
    let Some(raw) = q.run.filter(|s| !s.is_empty()) else {
        return error_response(StatusCode::BAD_REQUEST, "missing run query parameter");
    };
    let Some(run_id) = crate::id::RunId::parse(&raw) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            &format!("{raw:?} is not a valid run id"),
        );
    };
    // Confirm the run exists before opening a stream. An unknown id would
    // otherwise wait forever for events that will never arrive.
    let path = state.trace.clone();
    let id = run_id.clone();
    match tokio::task::spawn_blocking(move || run_exists(&path, id.as_str())).await {
        Ok(true) => {}
        Ok(false) => {
            return error_response(
                StatusCode::NOT_FOUND,
                &format!("no run {run_id} in the log"),
            );
        }
        Err(e) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
        }
    }
    let stream = stream::unfold(
        EventsState::new(state.trace.clone(), run_id.as_str().to_string()),
        |mut st| async move {
            loop {
                if let Some(event) = st.pending.pop_front() {
                    return Some((Ok::<Event, Infallible>(event), st));
                }
                if st.done {
                    return None;
                }
                if st.last_event.elapsed() >= EVENTS_IDLE_TIMEOUT {
                    // Nothing new for this run in a minute; close rather than
                    // hold a connection open for a run that stopped writing.
                    st.done = true;
                    continue;
                }
                if let Err(e) = st.poll() {
                    // A read failure is terminal for this stream; report it as
                    // an SSE event rather than a bare disconnect.
                    st.pending
                        .push_back(Event::default().event("error").data(e.to_string()));
                    st.done = true;
                    continue;
                }
                if st.pending.is_empty() && !st.done {
                    tokio::time::sleep(EVENTS_POLL).await;
                }
            }
        },
    );
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// True when `run_id` has a `run_start` in the log.
///
/// Streams the file line by line rather than loading every event, so a large
/// log does not have to be materialised just to answer this.
fn run_exists(path: &std::path::Path, run_id: &str) -> bool {
    use std::io::BufRead;
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    for line in std::io::BufReader::new(file).lines() {
        let Ok(line) = line else { continue };
        if !line.contains(run_id) {
            continue;
        }
        if let Ok(TraceEvent::RunStart { run_id: id, .. }) =
            serde_json::from_str::<TraceEvent>(&line)
            && id == run_id
        {
            return true;
        }
    }
    false
}

/// Cursor over the run log for one SSE client.
struct EventsState {
    path: PathBuf,
    run_id: String,
    /// Byte offset of the last complete line already forwarded.
    offset: u64,
    /// Events parsed but not yet yielded to the stream.
    pending: VecDeque<Event>,
    done: bool,
    /// When an event for this run was last seen; drives the idle timeout.
    last_event: Instant,
}

impl EventsState {
    fn new(path: PathBuf, run_id: String) -> Self {
        Self {
            path,
            run_id,
            offset: 0,
            pending: VecDeque::new(),
            done: false,
            last_event: Instant::now(),
        }
    }

    /// Reads newly appended lines and queues the ones for this run.
    fn poll(&mut self) -> std::io::Result<()> {
        let events = read_appended(&self.path, &mut self.offset, &self.run_id)?;
        for event in events {
            let ended = matches!(event, TraceEvent::RunEnd { .. });
            self.last_event = Instant::now();
            let json = serde_json::to_string(&event)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            self.pending.push_back(Event::default().data(json));
            if ended {
                self.pending
                    .push_back(Event::default().event("done").data("{}"));
                self.done = true;
            }
        }
        Ok(())
    }
}

/// True when the event is tagged with `run_id`.
fn event_belongs_to(event: &TraceEvent, run_id: &str) -> bool {
    match event {
        TraceEvent::RunStart { run_id: id, .. }
        | TraceEvent::Step { run_id: id, .. }
        | TraceEvent::RunEnd { run_id: id, .. } => id == run_id,
    }
}

/// Reads complete lines appended to `path` since `offset`, advancing it.
///
/// Streams the file line by line and keeps only the events tagged with
/// `run_id`, so even the first poll over a large log does not materialise the
/// whole file. A missing file is not an error — the run may not have written
/// its header yet. A partial final line is left unconsumed so the next poll
/// re-reads it whole. A file smaller than `offset` (log rotation) resets the
/// cursor.
fn read_appended(
    path: &std::path::Path,
    offset: &mut u64,
    run_id: &str,
) -> std::io::Result<Vec<TraceEvent>> {
    use std::io::{BufRead, BufReader, Seek, SeekFrom};

    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    if file.metadata()?.len() < *offset {
        *offset = 0;
    }
    file.seek(SeekFrom::Start(*offset))?;
    let mut reader = BufReader::new(file);

    let mut events = Vec::new();
    let mut consumed = 0u64;
    loop {
        let mut line = Vec::new();
        let n = reader.read_until(b'\n', &mut line)?;
        if n == 0 {
            break;
        }
        if !line.ends_with(b"\n") {
            break; // torn or still being written; retry next poll
        }
        consumed += n as u64;
        let text = String::from_utf8_lossy(&line);
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<TraceEvent>(text)
            && event_belongs_to(&event, run_id)
        {
            events.push(event);
        }
    }
    *offset += consumed;
    Ok(events)
}

fn error_response(status: StatusCode, message: &str) -> axum::response::Response {
    (status, Json(serde_json::json!({"error": message}))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    fn test_state() -> (ServerState, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let apps = dir.path().join("apps");
        std::fs::create_dir_all(&apps).unwrap();
        let minimal = r#"
            name = "blog"
            kind = "container"
            strategy = "swap"
            hostnames = ["blog.example.com"]
            front_port = 8001
            slot = 1
            image_repo = "apps-blog"
            registry = "local"
            health_url = "http://127.0.0.1:3000/up"
            compose_dir = "/tmp/apps"
            compose_svc = "blog"
            env_name = "BLOG_PORT"
        "#;
        std::fs::write(apps.join("blog.toml"), minimal).unwrap();
        let trace = dir.path().join("trace.jsonl");
        std::fs::write(&trace, "").unwrap();
        let state = ServerState {
            registry: apps,
            trace,
            tunnel_config: None,
            target: dir.path().join("front-door.conf"),
            main_config: dir.path().join("nginx.conf"),
            pid_file: dir.path().join("nginx.pid"),
            nginx_bin: "nginx".into(),
            state_dir: dir.path().join("green"),
            lock_dir: dir.path().join("lock"),
            drain_secs: 0,
        };
        (state, dir)
    }

    #[tokio::test]
    async fn health_reports_ok() {
        let (state, _dir) = test_state();
        let app = router(state);
        let res = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn health_is_also_served_under_api() {
        let (state, _dir) = test_state();
        let (status, json) = get_json(state, "/api/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn apps_lists_the_registry() {
        let (state, _dir) = test_state();
        let app = router(state);
        let res = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/apps")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json[0]["app"]["name"], "blog");
    }

    #[tokio::test]
    async fn runs_is_empty_for_a_fresh_log() {
        let (state, _dir) = test_state();
        let app = router(state);
        let res = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/runs")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    // ---- helpers ----

    /// Drives one request through the router and decodes a JSON body.
    async fn get_json(state: ServerState, uri: &str) -> (StatusCode, serde_json::Value) {
        let res = router(state)
            .oneshot(
                axum::http::Request::builder()
                    .uri(uri)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    /// Serves one canned HTTP response on an ephemeral loopback port.
    fn serve_once(response: &'static str) -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(response.as_bytes());
        });
        port
    }

    /// Rewrites the test app's `live_port` in the registry file.
    fn set_live_port(state: &ServerState, port: u16) {
        let path = state.registry.join("blog.toml");
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{text}\nlive_port = {port}\n")).unwrap();
    }

    /// Rewrites the test app's `repo` in the registry file.
    fn point_app_at_repo(state: &ServerState, repo: &std::path::Path) {
        let path = state.registry.join("blog.toml");
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            &path,
            format!("{text}\nrepo = {:?}\n", repo.display().to_string()),
        )
        .unwrap();
    }

    /// Rewrites the test app's `health_url` in the registry file.
    fn set_health_url(state: &ServerState, url: &str) {
        let path = state.registry.join("blog.toml");
        let text = std::fs::read_to_string(&path).unwrap();
        let mut out = String::new();
        for line in text.lines() {
            if line.trim_start().starts_with("health_url") {
                out.push_str(&format!("health_url = {url:?}\n"));
            } else {
                out.push_str(line);
                out.push('\n');
            }
        }
        std::fs::write(&path, out).unwrap();
    }

    /// Sets (or replaces) the test app's `branch`.
    fn set_branch(state: &ServerState, branch: &str) {
        let path = state.registry.join("blog.toml");
        let text = std::fs::read_to_string(&path).unwrap();
        let mut out = String::new();
        for line in text.lines() {
            if line.trim_start().starts_with("branch") {
                continue;
            }
            out.push_str(line);
            out.push('\n');
        }
        out.push_str(&format!("branch = {branch:?}\n"));
        std::fs::write(&path, out).unwrap();
    }

    // ---- SSE ----

    #[tokio::test]
    async fn events_streams_a_finished_run() {
        let (state, _dir) = test_state();
        let run_id = {
            let log = TraceLog::open(&state.trace).unwrap();
            let mut run = Run::start(
                log,
                RunMode::Live,
                Some("blog".into()),
                &["swapdock".to_string()],
                crate::redact::Redactor::new(),
            )
            .unwrap();
            let argv = vec!["docker".to_string(), "ps".to_string()];
            let seq = run.next_seq();
            run.record_step(crate::trace::StepRecord::new(
                seq,
                "pull",
                crate::trace::StepStatus::Ok,
                &argv,
            ))
            .unwrap();
            let id = run.id().to_string();
            run.finish(RunStatus::Ok).unwrap();
            id
        };

        let res = router(state)
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!("/events?run={run_id}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8(bytes.to_vec()).unwrap();

        assert!(
            text.contains("\"event\":\"run_end\""),
            "run_end must be streamed:\n{text}"
        );
        assert!(
            text.contains("\"step\":\"pull\""),
            "steps must be streamed:\n{text}"
        );
        assert!(
            text.contains("event: done"),
            "the stream must announce completion:\n{text}"
        );
    }

    #[tokio::test]
    async fn events_requires_a_run_id() {
        let (state, _dir) = test_state();
        let (status, json) = get_json(state, "/events").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"], "missing run query parameter");
    }

    #[tokio::test]
    async fn events_rejects_a_malformed_run_id() {
        let (state, _dir) = test_state();
        let (status, json) = get_json(state, "/events?run=not-a-run-id").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            json["error"].as_str().unwrap().contains("valid run id"),
            "{json}"
        );
    }

    #[tokio::test]
    async fn events_unknown_run_is_a_404_not_a_stream() {
        let (state, _dir) = test_state();
        // 27 lowercase hex characters, but no run with this id exists.
        let id = "0123456789abcdef0123456789a";
        assert_eq!(id.len(), 27);
        let (status, json) = get_json(state, &format!("/events?run={id}")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(json["error"].as_str().unwrap().contains("no run"), "{json}");
    }

    // ---- status ----

    #[tokio::test]
    async fn status_without_a_live_port_is_down() {
        let (state, _dir) = test_state();
        let (status, json) = get_json(state, "/apps/blog/status").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["up"], false);
        assert!(json["status"].is_null());
        assert!(json["live_port"].is_null());
    }

    #[tokio::test]
    async fn status_probes_the_live_port() {
        let (state, _dir) = test_state();
        let port = serve_once("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        set_live_port(&state, port);

        let (status, json) = get_json(state, "/apps/blog/status").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["up"], true);
        assert_eq!(json["status"], 200);
        assert_eq!(json["live_port"], port);
    }

    #[tokio::test]
    async fn status_treats_a_redirect_as_down() {
        let (state, _dir) = test_state();
        // A health endpoint must answer directly; a 302 is not "up".
        let port = serve_once("HTTP/1.1 302 Found\r\nLocation: /\r\n\r\n");
        set_live_port(&state, port);

        let (status, json) = get_json(state, "/apps/blog/status").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["up"], false);
        assert_eq!(json["status"], 302);
    }

    #[tokio::test]
    async fn status_reports_a_dead_port_down() {
        let (state, _dir) = test_state();
        // Bind and drop so the port is almost certainly free.
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        set_live_port(&state, port);

        let (status, json) = get_json(state, "/apps/blog/status").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["up"], false);
        assert!(json["status"].is_null());
        assert_eq!(json["live_port"], port);
    }

    #[tokio::test]
    async fn status_rejects_a_header_injection_health_url() {
        let (state, _dir) = test_state();
        let port = serve_once("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        set_live_port(&state, port);
        // A CRLF in the path would inject a header into the raw probe request.
        set_health_url(&state, "http://127.0.0.1/\r\nX-Evil: 1");

        let (status, json) = get_json(state, "/apps/blog/status").await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(json["error"], "invalid health path");
    }

    // ---- latest ----

    #[tokio::test]
    async fn latest_without_a_repo_is_a_conflict() {
        let (state, _dir) = test_state();
        let (status, json) = get_json(state, "/apps/blog/latest").await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(
            json["error"].as_str().unwrap().contains("no repo"),
            "{json}"
        );
    }

    #[tokio::test]
    async fn latest_falls_back_to_local_head() {
        let (state, dir) = test_state();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();

        // Build a one-commit repo through the chokepoint, so the test itself
        // honours the single-spawn rule.
        let setup_log = TraceLog::open(dir.path().join("setup.jsonl")).unwrap();
        let mut setup = Run::start(
            setup_log,
            RunMode::Live,
            None,
            &["git".to_string()],
            crate::redact::Redactor::new(),
        )
        .unwrap();
        let init = crate::exec::StepSpec::new("git-init", "git")
            .args(["init", "-q"])
            .cwd(&repo);
        let Ok(init) = setup.exec(&init) else {
            eprintln!("SKIP latest_falls_back_to_local_head: git unavailable");
            return;
        };
        assert!(init.success(), "git init failed: {}", init.stderr_trimmed());
        let commit = crate::exec::StepSpec::new("git-commit", "git")
            .args([
                "-c",
                "user.email=test@example.com",
                "-c",
                "user.name=test",
                "commit",
                "--allow-empty",
                "-q",
                "-m",
                "seed",
            ])
            .cwd(&repo);
        let commit = setup.exec(&commit).unwrap();
        assert!(
            commit.success(),
            "git commit failed: {}",
            commit.stderr_trimmed()
        );
        setup.finish(RunStatus::Ok).unwrap();

        point_app_at_repo(&state, &repo);
        let (status, json) = get_json(state, "/apps/blog/latest").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["source"], "HEAD");
        assert!(
            json["release"].as_str().is_some_and(|s| !s.is_empty()),
            "{json}"
        );
    }

    #[tokio::test]
    async fn latest_uses_the_configured_branch() {
        let (state, dir) = test_state();
        // Not a repo: the failure message names the refs that were tried, which
        // is where the branch choice shows up.
        let repo = dir.path().join("not-a-repo");
        std::fs::create_dir_all(&repo).unwrap();
        point_app_at_repo(&state, &repo);
        set_branch(&state, "develop");

        let (status, json) = get_json(state, "/apps/blog/latest").await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(
            json["error"].as_str().unwrap().contains("origin/develop"),
            "{json}"
        );
    }

    #[tokio::test]
    async fn latest_does_not_write_a_run() {
        let (state, dir) = test_state();
        // A directory that is not a git repo: the lookup 409s, but the point is
        // that no run is written either way.
        let repo = dir.path().join("not-a-repo");
        std::fs::create_dir_all(&repo).unwrap();
        point_app_at_repo(&state, &repo);

        let (status, _) = get_json(state.clone(), "/apps/blog/latest").await;
        assert_eq!(status, StatusCode::CONFLICT);

        let read = TraceLog::read(&state.trace).unwrap();
        assert!(
            read.events.is_empty(),
            "a read-only query must not appear in the run log: {:?}",
            read.events
        );
    }

    // ---- run summaries ----

    #[tokio::test]
    async fn run_summaries_carry_app_and_snake_case_status() {
        let (state, _dir) = test_state();
        {
            let log = TraceLog::open(&state.trace).unwrap();
            let run = Run::start(
                log,
                RunMode::Live,
                Some("blog".into()),
                &["swapdock".to_string()],
                crate::redact::Redactor::new(),
            )
            .unwrap();
            run.finish(RunStatus::Ok).unwrap();
        }

        let (status, json) = get_json(state, "/runs").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json[0]["app"], "blog");
        assert_eq!(json[0]["status"], "ok", "serde snake_case, not \"Ok\"");
    }
}
