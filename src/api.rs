//! HTTP API surface.
//!
//! Every mutating CLI command maps to one endpoint. Reads are synchronous;
//! mutations record the plan immediately, spawn the work in a thread, and
//! report the run id — the JSONL run log stays the observability query:
//! poll `GET /runs/{id}` for steps.
//!
//! Bind loopback by default; put authentication in front (nginx basic-auth,
//! Tailscale ACL) before exposing this anywhere.

use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::deploy::{self, Ctx};
use crate::registry::{App, Loaded, Problem};
use crate::trace::{Run, RunMode, RunStatus, TraceEvent, TraceLog};
use crate::{apply, builder, validator};

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
    Router::new()
        .route("/health", get(health))
        .route("/apps", get(list_apps).post(init_app))
        .route("/apps/{name}", get(show_app))
        .route("/apps/{name}/deploys", post(deploy_app))
        .route("/apps/{name}/rollback", post(rollback_app))
        .route("/apps/{name}/build", post(build_app))
        .route("/apps/{name}/sync", post(sync_app))
        .route("/apps/{name}/verify", get(verify_app))
        .route("/validate", post(validate))
        .route("/render", get(render_config))
        .route("/apply", post(apply_config))
        .route("/runs", get(list_runs))
        .route("/runs/{id}", get(show_run))
        .route("/runs/{id}/resume", get(resume_run))
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(state)
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
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"errors": errors, "problems": problems})),
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
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };
    let mut run = match Run::start(
        log,
        mode,
        app.map(str::to_string),
        &argv,
        crate::redact::Redactor::new(),
    ) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };
    let run_id = run.id().to_string();
    let run_id2 = run_id.clone();
    std::thread::spawn(move || {
        let result = work(&mut run);
        let status = match &result {
            Ok(_) if dry_run => RunStatus::DryRun,
            Ok(_) => RunStatus::Ok,
            Err(_) => RunStatus::Failed,
        };
        let _ = run.finish(status);
        if let Err(e) = &result {
            tracing::warn!(run = %run_id2, "job failed: {e}");
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
                        run_id, ts, mode, ..
                    } => summaries.push(serde_json::json!({
                        "run_id": run_id,
                        "started": ts,
                        "status": format!("{mode:?}"),
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
                            cur["status"] = serde_json::json!(format!("{status:?}"));
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
}
