//! HTTP API surface.
//!
//! Every mutating CLI command maps to one endpoint. Reads are synchronous;
//! mutations record the plan immediately, spawn the work in a thread, and
//! report the run id — the JSONL run log stays the observability query:
//! poll `GET /runs/{id}` for steps, or tail them live from `GET /events`.
//!
//! Routes are served under `/api`. `GET /health` is the only unauthenticated
//! route; every other request needs a bearer token from `SWAPDOCK_API_TOKEN`
//! (or `?token=` for `EventSource`). The server refuses to start without a
//! token unless `--allow-unauthenticated` was passed for local development.
//!
//! Bind loopback by default and terminate TLS at a reverse proxy. CORS is off
//! unless `SWAPDOCK_ALLOWED_ORIGINS` names the console's origin; the console is
//! expected to be same-origin.

use anyhow::Context;
use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::net::{IpAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream;
use serde::Deserialize;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;

use crate::auth::AuthConfig;
use crate::deploy::{self, DeployContext};
use crate::registry::{App, Loaded, Problem};
use crate::trace::{Run, RunMode, RunStatus, TraceEvent, TraceLog};
use crate::{apply, builder, exec, git, import, transfer, validator};

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

/// Upper bound on each probe behind `/build/apps/{name}/git`. A local ref lookup
/// or status scan is instant; the bound keeps a wedged checkout from holding an
/// HTTP request open.
const GIT_TIMEOUT: Duration = Duration::from_secs(10);

/// Upper bound on listing images for `/build/apps/{name}/images`.
const IMAGE_TIMEOUT: Duration = Duration::from_secs(10);

/// How many mutations may run at once. Every job is a real deploy/build and can
/// take minutes; an unauthenticated or compromised client must not be able to
/// exhaust the host by opening hundreds of them.
#[cfg(not(test))]
const MAX_CONCURRENT_JOBS: usize = 8;
#[cfg(test)]
const MAX_CONCURRENT_JOBS: usize = 64;

static JOB_SLOTS: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_JOBS)));

/// How many EventSource tails may be open at once. Each one holds a task and
/// polls the log; unbounded connections are a cheap resource-exhaustion path.
#[cfg(not(test))]
const MAX_EVENT_STREAMS: usize = 64;
#[cfg(test)]
const MAX_EVENT_STREAMS: usize = 8;

static EVENT_SLOTS: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(MAX_EVENT_STREAMS)));

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
    /// Where a build-server transfer stages its tarball before sending it.
    pub transfer_dir: PathBuf,
    /// Bearer-token auth and browser-origin policy.
    pub auth: AuthConfig,
    /// Per-client request rate limit and request timeout.
    pub rate: Arc<RateLimiter>,
}

/// A small fixed-window request limiter keyed by the forwarded client address.
/// This is deliberately process-local: one operator, one API process.
#[derive(Debug)]
pub struct RateLimiter {
    max: usize,
    window: Duration,
    hits: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl RateLimiter {
    pub fn new(max: usize, window_secs: u64) -> Self {
        Self {
            max,
            window: Duration::from_secs(window_secs),
            hits: Mutex::new(HashMap::new()),
        }
    }

    fn allow(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut hits = self.hits.lock().unwrap_or_else(|e| e.into_inner());
        // Bound the key map: a spoofed forwarded header must not grow it
        // without limit. Unknown keys are refused once the map is full rather
        // than evicting active clients.
        if hits.len() >= 4096 && !hits.contains_key(key) {
            return false;
        }
        let queue = hits.entry(key.to_string()).or_default();
        while queue
            .front()
            .is_some_and(|at| now.duration_since(*at) > self.window)
        {
            queue.pop_front();
        }
        if queue.len() >= self.max {
            return false;
        }
        queue.push_back(now);
        true
    }
}

type Shared = Arc<ServerState>;

pub fn router(state: ServerState) -> Router {
    let state: Shared = Arc::new(state);
    let mut router = Router::new()
        // `/health` stays unprefixed for external monitors. Everything else is
        // under `/api`, so a proxy rule cannot accidentally leave a mutation
        // route unprotected at the root.
        .route("/health", get(health))
        .nest("/api", api_router());

    if state.auth.is_required() {
        router = router.layer(middleware::from_fn_with_state(state.clone(), require_auth));
    }

    if !state.auth.allowed_origins().is_empty() {
        let origins: Vec<HeaderValue> = state
            .auth
            .allowed_origins()
            .iter()
            .filter_map(|origin| HeaderValue::from_str(origin).ok())
            .collect();
        if !origins.is_empty() {
            router = router.layer(
                CorsLayer::new()
                    .allow_origin(AllowOrigin::list(origins))
                    .allow_methods([
                        Method::GET,
                        Method::POST,
                        Method::PUT,
                        Method::DELETE,
                        Method::OPTIONS,
                    ])
                    .allow_headers([
                        header::AUTHORIZATION,
                        header::CONTENT_TYPE,
                        header::HeaderName::from_static("x-csrf-token"),
                    ])
                    .allow_credentials(true),
            );
        }
    }

    // JSON bodies are small; a large body must not occupy memory before a
    // handler can reject it. This does not bound response streaming (SSE).
    router = router.layer(RequestBodyLimitLayer::new(2 * 1024 * 1024));
    router = router.layer(middleware::from_fn_with_state(state.clone(), rate_limit));

    router.with_state(state)
}

/// Applies a process-local per-client rate limit and a request timeout. SSE is
/// exempt from the timeout because it is intentionally long-lived.
async fn rate_limit(
    State(state): State<Shared>,
    request: axum::extract::Request,
    next: Next,
) -> axum::response::Response {
    let key = login_key(request.headers());
    if !state.rate.allow(&key) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, "60")],
            "too many requests; retry later",
        )
            .into_response();
    }
    if request.uri().path() == "/api/events" {
        return next.run(request).await;
    }
    match tokio::time::timeout(Duration::from_secs(30), next.run(request)).await {
        Ok(response) => response,
        Err(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            "request timed out after 30 seconds",
        )
            .into_response(),
    }
}

/// Auth middleware. Health and `POST /api/login` are the only unauthenticated
/// routes; cookie sessions need a CSRF header on mutations.
async fn require_auth(
    State(state): State<Shared>,
    request: axum::extract::Request,
    next: Next,
) -> axum::response::Response {
    let path = request.uri().path();
    if path == "/health" || path == "/api/health" {
        return next.run(request).await;
    }
    if !state.auth.host_origin_ok(request.headers()) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "Host or Origin is not allowed; set SWAPDOCK_ALLOWED_HOSTS/ORIGINS"
            })),
        )
            .into_response();
    }
    if path == "/api/login" {
        return next.run(request).await;
    }
    if state.auth.authorized(request.headers(), request.uri()) {
        // A cookie session needs a CSRF header on mutations. Bearer clients are
        // exempt: a custom Authorization header is not sent by a cross-site
        // form, and tooling cannot obtain a CSRF token from a cookie it does
        // not use.
        let mutation = matches!(
            *request.method(),
            Method::POST | Method::PUT | Method::PATCH | Method::DELETE
        );
        if mutation
            && !state.auth.bearer_valid(request.headers())
            && !state.auth.csrf_valid(request.headers())
        {
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "error": "CSRF token missing or invalid; refresh the console session"
                })),
            )
                .into_response();
        }
        return next.run(request).await;
    }
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
        Json(serde_json::json!({"error": "authentication required"})),
    )
        .into_response()
}

/// The API routes, mounted under `/api`.
fn api_router() -> Router<Shared> {
    Router::new()
        .route("/health", get(health))
        .route("/login", post(login))
        .route("/session", get(session_info))
        .route("/logout", post(logout))
        .route("/apps", get(list_apps).post(init_app))
        .route("/import", post(import_app))
        .route("/apps/{name}", get(show_app).put(update_app))
        .route("/apps/{name}/status", get(app_status))
        .route("/apps/{name}/inspect", get(app_inspect))
        .route("/apps/{name}/doctor", get(app_doctor))
        .route("/apps/{name}/adopt", post(adopt_app))
        .route("/apps/{name}/backup", post(backup_app))
        .route("/apps/{name}/probe", post(probe_app))
        .route("/apps/{name}/prune", post(prune_app))
        .route("/apps/{name}/latest", get(app_latest))
        .route("/apps/{name}/drift", get(app_drift))
        .route("/apps/{name}/deploys", post(deploy_app))
        .route("/apps/{name}/rollback", post(rollback_app))
        .route("/apps/{name}/build", post(build_app))
        .route("/apps/{name}/sync", post(sync_app))
        .route("/apps/{name}/verify", get(verify_app))
        // The build-server surface: reads about source and images, plus the
        // build and transfer mutations. `builds` reuses the `/build` handler.
        .route("/build/apps/{name}/git", get(app_git))
        .route("/build/apps/{name}/images", get(app_images))
        .route("/build/apps/{name}/builds", post(build_app))
        .route("/build/apps/{name}/transfers", post(transfer_app))
        .route("/doctor", get(doctor_all))
        .route("/validate", get(validate).post(validate))
        .route("/render", get(render_config))
        .route("/apply", post(apply_config))
        .route("/events", get(run_events))
        .route("/runs", get(list_runs))
        .route("/runs/{id}", get(show_run))
        .route("/runs/{id}/resume", get(resume_run))
}

/// Optional TLS material for the API listener.
#[derive(Debug, Clone)]
pub struct TlsConfig {
    pub cert: PathBuf,
    pub key: PathBuf,
}

/// Serves the API over HTTP, or HTTPS when both TLS paths are supplied.
///
/// TLS here terminates on the API listener itself; the console can still be
/// served by the nginx front door. Supplying a certificate does not obtain or
/// renew it — use certbot/Caddy/Cloudflare for that.
pub async fn serve(state: ServerState, bind: &str, tls: Option<TlsConfig>) -> anyhow::Result<()> {
    let app = router(state);
    if let Some(tls) = tls {
        // axum-server's no-provider build leaves the rustls provider to us.
        // `ring` is used explicitly so a static musl build does not need the
        // aws-lc C toolchain.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let config =
            axum_server::tls_rustls::RustlsConfig::from_pem_file(&tls.cert, &tls.key).await?;
        let addr: std::net::SocketAddr = bind.parse().with_context(|| {
            format!("TLS bind {bind:?} must be an IP:port (use a reverse proxy for a hostname)")
        })?;
        axum_server::bind_rustls(addr, config)
            .serve(app.into_make_service())
            .await?;
        return Ok(());
    }
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// health
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct LoginBody {
    password: String,
}

fn secure_cookies(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("https"))
}

fn login_key(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|value| value.to_str().ok())
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or("unknown")
        .to_string()
}

async fn login(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<LoginBody>,
) -> impl IntoResponse {
    let key = login_key(&headers);
    if !state.auth.login_allowed(&key) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, "300")],
            Json(serde_json::json!({"error": "too many failed logins; retry later"})),
        )
            .into_response();
    }
    let Some((id, csrf)) = state.auth.login(&body.password) else {
        state.auth.login_failed(&key);
        return error_response(
            StatusCode::UNAUTHORIZED,
            "the access token was not accepted, or console sessions are disabled",
        );
    };
    state.auth.login_succeeded(&key);
    let cookie = AuthConfig::session_cookie(&id, secure_cookies(&headers));
    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        Json(serde_json::json!({
            "csrf_token": csrf,
            "expires_in": 12 * 60 * 60,
        })),
    )
        .into_response()
}

async fn session_info(State(state): State<Shared>, headers: HeaderMap) -> impl IntoResponse {
    match state.auth.session_csrf(&headers) {
        Some(csrf) => (
            StatusCode::OK,
            Json(serde_json::json!({"authenticated": true, "mode": "session", "csrf_token": csrf})),
        )
            .into_response(),
        None => (
            StatusCode::OK,
            Json(serde_json::json!({"authenticated": true, "mode": "bearer"})),
        )
            .into_response(),
    }
}

async fn logout(State(state): State<Shared>, headers: HeaderMap) -> impl IntoResponse {
    state.auth.logout(&headers);
    (
        StatusCode::OK,
        [(
            header::SET_COOKIE,
            AuthConfig::clear_cookie(secure_cookies(&headers)),
        )],
        Json(serde_json::json!({"ok": true})),
    )
        .into_response()
}

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

#[derive(Deserialize)]
struct ImportBody {
    compose_dir: String,
    #[serde(default)]
    service: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    domain: Option<String>,
    #[serde(default)]
    front_port: Option<u16>,
    #[serde(default)]
    slot: Option<u8>,
    #[serde(default)]
    container_port: Option<u16>,
    #[serde(default)]
    healthcheck: Option<String>,
    #[serde(default)]
    repo: Option<String>,
    #[serde(default)]
    dry_run: bool,
}

/// `POST /api/import` — the console's Compose importer.
///
/// This is the same engine as `swapdock import`; it writes a registry entry and
/// generated override, never the Compose file, and does not deploy.
async fn import_app(
    State(state): State<Shared>,
    Json(body): Json<ImportBody>,
) -> impl IntoResponse {
    let opts = import::Options {
        compose_dir: PathBuf::from(body.compose_dir),
        service: body.service,
        name: body.name,
        domain: body.domain,
        front_port: body.front_port,
        slot: body.slot,
        container_port: body.container_port,
        healthcheck: body.healthcheck,
        repo: body.repo.map(PathBuf::from),
        state_dir: state.state_dir.clone(),
        dry_run: body.dry_run,
    };
    let registry_dir = state.registry.clone();
    match tokio::task::spawn_blocking(move || import::run(&opts, &registry_dir, None)).await {
        Ok(Ok(report)) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "app": report.app,
                "wrote": report.wrote,
                "dry_run": report.dry_run,
                "registry_path": report.registry_path,
                "override_path": report.override_path,
                "override_text": report.override_text,
                "service": report.service,
                "image": report.image,
                "container_port": report.container_port,
                "healthcheck": report.healthcheck,
                "strategy_reason": report.strategy_reason,
                "notes": report.notes,
                "warnings": report.warnings,
            })),
        )
            .into_response(),
        Ok(Err(e)) => {
            let status = match e {
                import::ImportError::Refused(_) => StatusCode::CONFLICT,
                _ => StatusCode::BAD_REQUEST,
            };
            error_response(status, &e.to_string())
        }
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("import task failed: {e}"),
        ),
    }
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

/// Replaces one app's registry file. Refuses when the resulting estate has
/// errors, so a bad edit cannot brick `validate`. Keeps a `.bak` of the
/// previous file for revert; `save_app` preserves its mode and owner.
async fn update_app(
    State(state): State<Shared>,
    Path(name): Path<String>,
    Json(app): Json<App>,
) -> impl IntoResponse {
    if app.name != name {
        return error_response(
            StatusCode::BAD_REQUEST,
            &format!("path name {name:?} does not match body name {:?}", app.name),
        );
    }
    // Serialize the read-modify-write with deploys and other edits. Without
    // this, an operator save can write back a pre-deploy live_port/old_release
    // after a deploy committed, silently reverting the registry.
    let lock_dir = state.lock_dir.clone();
    let _estate = match tokio::task::spawn_blocking(move || {
        crate::lock::Lock::acquire(
            &lock_dir,
            crate::lock::estate_lock_name(),
            crate::lock::ACQUIRE_TIMEOUT,
        )
    })
    .await
    {
        Ok(Ok(lock)) => lock,
        Ok(Err(e)) => {
            return error_response(StatusCode::SERVICE_UNAVAILABLE, &e.to_string());
        }
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("lock task failed: {e}"),
            );
        }
    };
    let loaded = match load(&state) {
        Ok(l) => l,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    if !loaded.apps.iter().any(|a| a.name == name) {
        return error_response(StatusCode::NOT_FOUND, &format!("no app named {name:?}"));
    }
    let mut apps: Vec<App> = loaded
        .apps
        .iter()
        .map(|a| {
            if a.name == name {
                app.clone()
            } else {
                a.clone()
            }
        })
        .collect();
    apps.sort_by(|a, b| a.name.cmp(&b.name));

    // Same checks `validate` runs: per-app plus cross-app plus tunnel.
    // Load-time (parse) problems carry over; app problems are recomputed
    // from the staged apps inside collect_problems.
    let staged = Loaded {
        apps,
        problems: loaded.problems.clone(),
    };
    let problems = collect_problems(&state, &staged);
    let (errors, _) = validator::counts(&problems);
    if errors > 0 {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": format!("{errors} error(s) in the edited registry; nothing was saved"),
                "errors": errors,
                "problems": problems,
            })),
        )
            .into_response();
    }

    // Rollback copy before the write.
    let target = state.registry.join(format!("{name}.toml"));
    let backup = state.registry.join(format!("{name}.toml.bak"));
    if let Ok(raw) = std::fs::read(&target) {
        let _ = std::fs::write(&backup, raw);
    }
    match crate::registry::save_app(&state.registry, &app) {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "backup": backup.display().to_string()})),
        )
            .into_response(),
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
/// behind it can disagree. `release`/`old_release` are the recorded identity;
/// `up`/`status` are the probe.
async fn app_status(State(state): State<Shared>, Path(name): Path<String>) -> impl IntoResponse {
    let app = match load_app(&state, &name) {
        Ok(a) => a,
        Err(e) => return error_response(StatusCode::NOT_FOUND, &e.to_string()),
    };
    // A pending record means a previous run stopped between the flip and the
    // commit. Report it alongside the recorded state; the record is advisory,
    // but a confident "up" without it would be a lie.
    let pending = crate::journal::read(&state.state_dir, &name).ok().flatten();
    // The probe is blocking socket I/O; keep it off the async workers.
    match tokio::task::spawn_blocking(move || deploy::live_status(&app)).await {
        Ok(Ok(s)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "up": s.up,
                "status": s.code,
                "live_port": s.live_port,
                "release": s.release,
                "old_release": s.old_release,
                "pending": pending,
            })),
        )
            .into_response(),
        // An unusable health path is a registry conflict, not a fault here.
        Ok(Err(e)) => error_response(StatusCode::CONFLICT, &e.to_string()),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("status probe failed: {e}"),
        ),
    }
}

fn inspect_json(app: &App) -> anyhow::Result<serde_json::Value> {
    let live = deploy::live_status(app)?;
    let mut value = serde_json::json!({
        "app": app,
        "live": {
            "release": live.release,
            "old_release": live.old_release,
            "live_port": live.live_port,
            "up": live.up,
            "code": live.code,
        },
    });
    let port = app.live_port.unwrap_or(app.front_port);
    let found = exec::run_capture(&crate::docker::ps_publishing(port))?;
    let names: Vec<String> = found
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    if let Some(container) = names.first() {
        let mut details = serde_json::json!({"name": container});
        if let Ok(out) = exec::run_capture(&crate::docker::inspect_image_ref(container))
            && out.success()
        {
            details["image"] = serde_json::json!(out.stdout_trimmed());
        }
        if let Ok(out) = exec::run_capture(&crate::docker::inspect_health(container))
            && out.success()
        {
            details["health"] = serde_json::json!(out.stdout_trimmed());
        }
        if let Ok(out) = exec::run_capture(&crate::docker::inspect_mounts(container))
            && out.success()
            && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(out.stdout_trimmed())
        {
            details["mounts"] = parsed;
        }
        value["container"] = details;
    }
    Ok(value)
}

async fn app_inspect(State(state): State<Shared>, Path(name): Path<String>) -> impl IntoResponse {
    let app = match load_app(&state, &name) {
        Ok(a) => a,
        Err(e) => return error_response(StatusCode::NOT_FOUND, &e.to_string()),
    };
    match tokio::task::spawn_blocking(move || inspect_json(&app)).await {
        Ok(Ok(value)) => (StatusCode::OK, Json(value)).into_response(),
        Ok(Err(e)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("inspect failed: {e}"),
        ),
    }
}

#[derive(Deserialize, Default)]
struct DoctorQuery {
    #[serde(default)]
    public: bool,
}

async fn app_doctor(
    State(state): State<Shared>,
    Path(name): Path<String>,
    Query(q): Query<DoctorQuery>,
) -> impl IntoResponse {
    let loaded = match load(&state) {
        Ok(l) => l,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let selected: Vec<App> = loaded
        .sorted()
        .into_iter()
        .filter(|a| a.name == name)
        .collect();
    if selected.is_empty() {
        return error_response(StatusCode::NOT_FOUND, &format!("no app named {name:?}"));
    }
    let state_for_job = state.clone();
    match tokio::task::spawn_blocking(move || {
        let log = TraceLog::open(&state_for_job.trace)?;
        let mut run = Run::start(
            log,
            RunMode::Live,
            Some(name.clone()),
            &["swapdock".to_string(), "api".to_string()],
            crate::redact::Redactor::new(),
        )?;
        let mut problems: Vec<Problem> = selected.iter().flat_map(|a| a.problems()).collect();
        problems.extend(crate::doctor::check(&mut run, &selected, None, q.public));
        validator::sort_problems(&mut problems);
        run.finish(RunStatus::Ok)?;
        Ok::<_, anyhow::Error>(problems)
    })
    .await
    {
        Ok(Ok(problems)) => (
            StatusCode::OK,
            Json(serde_json::json!({"problems": problems})),
        )
            .into_response(),
        Ok(Err(e)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("doctor failed: {e}"),
        ),
    }
}

#[derive(Deserialize, Default)]
struct DoctorAllQuery {
    app: Option<String>,
    #[serde(default)]
    public: bool,
}

async fn doctor_all(
    State(state): State<Shared>,
    Query(q): Query<DoctorAllQuery>,
) -> impl IntoResponse {
    let loaded = match load(&state) {
        Ok(l) => l,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let selected: Vec<App> = loaded
        .sorted()
        .into_iter()
        .filter(|a| q.app.as_deref().is_none_or(|name| a.name == name))
        .collect();
    if selected.is_empty() {
        return error_response(StatusCode::NOT_FOUND, "no matching apps");
    }
    let state_for_job = state.clone();
    let public = q.public;
    match tokio::task::spawn_blocking(move || {
        let log = TraceLog::open(&state_for_job.trace)?;
        let mut run = Run::start(
            log,
            RunMode::Live,
            None,
            &["swapdock".to_string(), "api".to_string()],
            crate::redact::Redactor::new(),
        )?;
        let mut problems: Vec<Problem> = selected.iter().flat_map(|a| a.problems()).collect();
        problems.extend(crate::doctor::check(&mut run, &selected, None, public));
        validator::sort_problems(&mut problems);
        run.finish(RunStatus::Ok)?;
        Ok::<_, anyhow::Error>(problems)
    })
    .await
    {
        Ok(Ok(problems)) => (
            StatusCode::OK,
            Json(serde_json::json!({"problems": problems})),
        )
            .into_response(),
        Ok(Err(e)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("doctor failed: {e}"),
        ),
    }
}

#[derive(Deserialize)]
struct AdoptBody {
    release: String,
    healthcheck: Option<String>,
    container_port: Option<u16>,
    #[serde(default)]
    dry_run: bool,
}

async fn adopt_app(
    State(state): State<Shared>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Option<AdoptBody>>,
) -> impl IntoResponse {
    let Some(body) = body else {
        return error_response(StatusCode::BAD_REQUEST, "release is required");
    };
    let app = match load_app(&state, &name) {
        Ok(a) => a,
        Err(e) => return error_response(StatusCode::NOT_FOUND, &e.to_string()),
    };
    let ctx = ctx_for(&state);
    let mut app = app;
    spawn_job(
        &state,
        Some(&name),
        body.dry_run,
        Some(api_actor(&headers)),
        move |run| {
            deploy::adopt(
                run,
                &ctx,
                &mut app,
                &body.release,
                body.healthcheck.as_deref(),
                body.container_port,
            )
            .map(|()| serde_json::json!({}))
            .map_err(|e| e.to_string())
        },
    )
    .into_response()
}

#[derive(Deserialize, Default)]
struct BackupBody {
    #[serde(default)]
    dry_run: bool,
}

async fn backup_app(
    State(state): State<Shared>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Option<BackupBody>>,
) -> impl IntoResponse {
    let dry = body.as_ref().map(|b| b.dry_run).unwrap_or(false);
    let app = match load_app(&state, &name) {
        Ok(a) => a,
        Err(e) => return error_response(StatusCode::NOT_FOUND, &e.to_string()),
    };
    spawn_job(
        &state,
        Some(&name),
        dry,
        Some(api_actor(&headers)),
        move |run| {
            crate::backup::run_backup(run, &app, app.release.as_deref())
                .map(|file| serde_json::json!({"file": file}))
                .map_err(|e| e.to_string())
        },
    )
    .into_response()
}

#[derive(Deserialize, Default)]
struct ProbeBody {
    url: Option<String>,
}

async fn probe_app(
    State(state): State<Shared>,
    Path(name): Path<String>,
    Json(body): Json<Option<ProbeBody>>,
) -> impl IntoResponse {
    let body = body.unwrap_or_default();
    let app = match load_app(&state, &name) {
        Ok(a) => a,
        Err(e) => return error_response(StatusCode::NOT_FOUND, &e.to_string()),
    };
    let Some(url) = body.url.or_else(|| app.public_url.clone()) else {
        return error_response(
            StatusCode::CONFLICT,
            &format!("{name} has no public_url; pass a url"),
        );
    };
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "url must start with http:// or https://",
        );
    }
    if !probe_target_allowed(&app, &url) {
        return error_response(
            StatusCode::BAD_REQUEST,
            &format!(
                "{name} can only probe its own hostnames/public_url; set SWAPDOCK_PROBE_ALLOWLIST for another target"
            ),
        );
    }
    match tokio::task::spawn_blocking(move || {
        // Pin curl to a public address we resolved here. The older code let
        // curl resolve the hostname itself, so an allowlisted name could rebind
        // to loopback/RFC1918 between the allowlist check and the request.
        let (host, port) = url_host_port(&url).ok_or("url has no usable host")?;
        let ip = public_ip_for_host(&host, port)?;
        let pinned = if ip.is_ipv6() {
            format!("[{ip}]")
        } else {
            ip.to_string()
        };
        let spec = exec::StepSpec::new("probe-public", "curl")
            .args([
                "-fsS",
                "-o",
                "/dev/null",
                "-w",
                "%{http_code}",
                "--max-time",
                "15",
                "--proto",
                "=http,https",
                "--resolve",
                &format!("{host}:{port}:{pinned}"),
                &url,
            ])
            .timeout(Duration::from_secs(20));
        let outcome = exec::run_capture(&spec).map_err(|e| e.to_string())?;
        let status = outcome.stdout_trimmed().parse::<u16>().unwrap_or(0);
        Ok::<_, String>(serde_json::json!({"url": url, "status": status}))
    })
    .await
    {
        Ok(Ok(value)) => (StatusCode::OK, Json(value)).into_response(),
        Ok(Err(e)) => error_response(StatusCode::BAD_GATEWAY, &e),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("probe failed: {e}"),
        ),
    }
}

/// Extracts a lower-case host from an http(s) URL without a URL crate.
fn url_host(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    let authority = rest.split('/').next()?;
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let host = authority
        .rsplit_once(':')
        .map(|(host, port)| {
            // Keep the host, drop the port. Bracketed IPv6 is accepted as-is.
            if port.chars().all(|c| c.is_ascii_digit()) {
                host
            } else {
                authority
            }
        })
        .unwrap_or(authority);
    Some(host.trim_matches(['[', ']']).to_ascii_lowercase())
}

/// Host and port from an http(s) URL, defaulting to 80/443.
fn url_host_port(url: &str) -> Option<(String, u16)> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    let scheme_https = url.starts_with("https://");
    let authority = rest.split('/').next()?;
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let (host, tail) = rest.split_once(']')?;
        let port = tail
            .strip_prefix(':')
            .and_then(|p| p.parse().ok())
            .unwrap_or(if scheme_https { 443 } else { 80 });
        (host.to_string(), port)
    } else if let Some((host, port)) = authority.rsplit_once(':')
        && port.chars().all(|c| c.is_ascii_digit())
    {
        (host.to_string(), port.parse().ok()?)
    } else {
        (authority.to_string(), if scheme_https { 443 } else { 80 })
    };
    Some((host.to_ascii_lowercase(), port))
}

/// Resolve once, reject non-public addresses, and return the address curl must
/// use. This is the DNS-rebinding guard; the caller passes it via `--resolve`.
fn public_ip_for_host(host: &str, port: u16) -> Result<IpAddr, String> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return if is_public_ip(ip) {
            Ok(ip)
        } else {
            Err(format!("refusing non-public probe address {ip}"))
        };
    }
    let addresses: Vec<IpAddr> = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("resolve {host}: {e}"))?
        .map(|addr| addr.ip())
        .collect();
    addresses
        .into_iter()
        .find(|ip| is_public_ip(*ip))
        .ok_or_else(|| format!("{host} resolves only to non-public addresses"))
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !v4.is_private()
                && !v4.is_loopback()
                && !v4.is_link_local()
                && !v4.is_unspecified()
                && !v4.is_multicast()
                && !v4.is_broadcast()
                && !(v4.octets()[0] == 0)
                && !(v4.octets()[0] == 100 && (64..=127).contains(&v4.octets()[1]))
                && !(v4.octets()[0] == 198 && (18..=19).contains(&v4.octets()[1]))
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(v4));
            }
            !v6.is_loopback()
                && !v6.is_unspecified()
                && !v6.is_multicast()
                && !v6.is_unique_local()
                && !v6.is_unicast_link_local()
        }
    }
}

/// The probe is an SSRF surface if it accepts arbitrary URLs. Restrict it to
/// the app's own hostnames, its configured public_url, or an explicit operator
/// allowlist in `SWAPDOCK_PROBE_ALLOWLIST`.
fn probe_target_allowed(app: &App, url: &str) -> bool {
    let Some(host) = url_host(url) else {
        return false;
    };
    if app
        .hostnames
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(&host))
    {
        return true;
    }
    if app
        .public_url
        .as_deref()
        .and_then(url_host)
        .is_some_and(|candidate| candidate == host)
    {
        return true;
    }
    std::env::var("SWAPDOCK_PROBE_ALLOWLIST")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .any(|entry| entry == "*" || entry.eq_ignore_ascii_case(&host))
}

#[derive(Deserialize, Default)]
struct PruneBody {
    #[serde(default = "default_keep")]
    keep: usize,
    #[serde(default)]
    dry_run: bool,
}

fn default_keep() -> usize {
    3
}

async fn prune_app(
    State(state): State<Shared>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Option<PruneBody>>,
) -> impl IntoResponse {
    let body = body.unwrap_or_default();
    let app = match load_app(&state, &name) {
        Ok(a) => a,
        Err(e) => return error_response(StatusCode::NOT_FOUND, &e.to_string()),
    };
    let Some(repo) = app.image_repo.clone() else {
        return error_response(StatusCode::CONFLICT, "app has no image_repo");
    };
    spawn_job(
        &state,
        Some(&name),
        body.dry_run,
        Some(api_actor(&headers)),
        move |run| {
            if run.mode() == RunMode::DryRun {
                return Ok(serde_json::json!({"would_prune": repo, "keep": body.keep}));
            }
            let listed = run
                .exec(&crate::docker::image_tags(&repo))
                .map_err(|e| e.to_string())?;
            let tags: Vec<String> = listed
                .stdout
                .lines()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .collect();
            let keep = crate::prune::Keep {
                current: app.release.clone(),
                old: app.old_release.clone(),
            };
            let mut removed = 0usize;
            let mut failed = 0usize;
            for tag in crate::prune::select_removals(&tags, &keep, body.keep) {
                let image = format!("{repo}:{tag}");
                match run.exec(&crate::docker::rmi(&image)) {
                    Ok(o) if o.success() => removed += 1,
                    _ => failed += 1,
                }
            }
            Ok(serde_json::json!({"removed": removed, "failed": failed}))
        },
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
    let Some(repo) = app.build_repo.clone().or(app.repo.clone()) else {
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

/// `GET /apps/{name}/drift` — how far the live deployment is from the newest
/// source version, and whether that version is already built.
///
/// This is the read model behind the app page's "current vs latest" card. It
/// deliberately reports the *fetched* `origin/<branch>`, not a network fetch:
/// a read must not mutate the checkout, and a stale checkout must be visible as
/// "last fetched" rather than silently claimed as current.
async fn app_drift(State(state): State<Shared>, Path(name): Path<String>) -> impl IntoResponse {
    let app = match load_app(&state, &name) {
        Ok(a) => a,
        Err(e) => return error_response(StatusCode::NOT_FOUND, &e.to_string()),
    };
    let repo = app.build_repo.clone().or(app.repo.clone());
    match tokio::task::spawn_blocking(move || compute_drift(&app, repo.as_deref())).await {
        Ok(report) => (StatusCode::OK, Json(report)).into_response(),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("drift query failed: {e}"),
        ),
    }
}

/// The pure state machine behind the drift card. Kept separate so it can be
/// tested without a git checkout or Docker.
fn drift_state(
    live: Option<&str>,
    latest: Option<&str>,
    built: bool,
    has_source: bool,
    live_image_present: bool,
) -> &'static str {
    if live.is_none() {
        return "not_deployed";
    }
    if !live_image_present {
        return "live_unbuilt";
    }
    if !has_source {
        return "no_source";
    }
    let Some(latest) = latest else {
        return "unknown";
    };
    let Some(live) = live else {
        return "not_deployed";
    };
    if live == latest || live.starts_with(latest) || latest.starts_with(live) {
        return "up_to_date";
    }
    if built {
        "behind_built"
    } else {
        "behind_unbuilt"
    }
}

fn same_commit(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && (a == b || a.starts_with(b) || b.starts_with(a))
}

/// Reads `<subject>\0<committed-at>` for one revision. Missing values become
/// null; a git failure never fails the whole drift response.
fn git_summary(repo: &std::path::Path, rev: &str) -> (Option<String>, Option<String>) {
    let spec = git::log_one(repo, rev).timeout(GIT_TIMEOUT);
    match exec::run_capture(&spec) {
        Ok(out) if out.success() => {
            let text = out.stdout_trimmed();
            match text.split_once('\0') {
                Some((subject, committed_at)) => (
                    (!subject.trim().is_empty()).then(|| subject.trim().to_string()),
                    (!committed_at.trim().is_empty()).then(|| committed_at.trim().to_string()),
                ),
                None => (
                    (!text.trim().is_empty()).then(|| text.trim().to_string()),
                    None,
                ),
            }
        }
        _ => (None, None),
    }
}

fn compute_drift(app: &App, repo: Option<&std::path::Path>) -> serde_json::Value {
    let live_release = app.release.clone();
    let live_up = deploy::live_status(app).map(|s| s.up).unwrap_or(false);

    let images = app
        .image_repo
        .as_deref()
        .and_then(|repo| list_images(repo).ok())
        .unwrap_or_default();

    let live_image_present = match (app.kind, app.root.as_deref(), live_release.as_deref()) {
        (crate::registry::Kind::Static, Some(root), Some(live)) => {
            crate::static_site::ensure_release(std::path::Path::new(root), live).is_ok()
        }
        _ => live_release
            .as_deref()
            .is_some_and(|live| image_for(&images, live).is_some()),
    };

    let latest = repo.and_then(|repo| {
        resolve_latest(app, repo).ok().map(|(release, source)| {
            let (subject, committed_at) = git_summary(repo, &source);
            (release, source, subject, committed_at)
        })
    });

    let commits_behind = match (repo, live_release.as_deref(), latest.as_ref()) {
        (Some(repo), Some(live), Some((_, source, _, _))) if !same_commit(live, source) => {
            let range = format!("{live}..{source}");
            exec::run_capture(&git::rev_list_count(repo, &range).timeout(GIT_TIMEOUT))
                .ok()
                .filter(|o| o.success())
                .and_then(|o| o.stdout_trimmed().parse::<u64>().ok())
        }
        _ => Some(0),
    };

    let built = latest.as_ref().and_then(|(release, _, _, _)| {
        if app.kind == crate::registry::Kind::Static {
            app.root.as_deref().and_then(|root| {
                crate::static_site::ensure_release(std::path::Path::new(root), release)
                    .ok()
                    .map(|path| {
                        serde_json::json!({
                            "release": release,
                            "tag": release,
                            "path": path,
                            "size": null,
                            "created": null,
                        })
                    })
            })
        } else {
            image_for(&images, release).cloned()
        }
    });

    let state = drift_state(
        live_release.as_deref(),
        latest.as_ref().map(|(release, _, _, _)| release.as_str()),
        built.is_some(),
        repo.is_some(),
        live_image_present,
    );

    let message = match state {
        "not_deployed" => "Not deployed yet".to_string(),
        "no_source" => "No source checkout configured".to_string(),
        "unknown" => "Could not resolve the latest source version".to_string(),
        "up_to_date" => "Up to date with the fetched source".to_string(),
        "behind_built" => match commits_behind {
            Some(n) => format!(
                "{n} commit{} behind; the latest image is ready",
                if n == 1 { "" } else { "s" }
            ),
            None => "Newer source is available and already built".to_string(),
        },
        "behind_unbuilt" => match commits_behind {
            Some(n) => format!(
                "{n} commit{} behind; build the latest version to deploy",
                if n == 1 { "" } else { "s" }
            ),
            None => "Newer source is available but not built yet".to_string(),
        },
        _ => "Unknown version state".to_string(),
    };

    serde_json::json!({
        "live": {
            "release": live_release,
            "live_port": app.live_port,
            "up": live_up,
            "image_present": live_image_present,
        },
        "latest": latest.map(|(release, source, subject, committed_at)| serde_json::json!({
            "release": release,
            "source": source,
            "subject": subject,
            "committed_at": committed_at,
        })),
        "built": built,
        "commits_behind": commits_behind,
        "state": state,
        "message": message,
        "source": repo.map(|p| p.display().to_string()),
    })
}

/// Finds the image record for a release, using the same prefix rule as the web
/// client: a short tag and a long commit are the same release.
fn image_for<'a>(images: &'a [serde_json::Value], release: &str) -> Option<&'a serde_json::Value> {
    images.iter().find(|image| {
        image
            .get("tag")
            .and_then(|v| v.as_str())
            .is_some_and(|tag| same_commit(tag, release))
    })
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
    headers: HeaderMap,
    Json(body): Json<Option<ApplyBody>>,
) -> impl IntoResponse {
    let dry = body.as_ref().map(|b| b.dry_run).unwrap_or(false);
    let state2 = state.clone();
    spawn_job(&state, None, dry, Some(api_actor(&headers)), move |run| {
        let _estate = crate::lock::Lock::acquire(
            &state2.lock_dir,
            crate::lock::estate_lock_name(),
            crate::lock::ACQUIRE_TIMEOUT,
        )
        .map_err(|e| e.to_string())?;
        // Load after the lock. A pre-lock snapshot can be stale if a deploy
        // committed while we waited; applying it would write old live_ports.
        let loaded = load(&state2).map_err(|e| e.to_string())?;
        let problems = collect_problems(&state2, &loaded);
        let (errors, _) = validator::counts(&problems);
        if errors > 0 {
            return Err(format!(
                "{errors} error(s) in the registry; nothing was applied: {}",
                problems
                    .iter()
                    .filter(|p| p.severity == crate::registry::Severity::Error)
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }
        let apps = loaded.sorted();
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

fn ctx_for(state: &ServerState) -> DeployContext {
    DeployContext {
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
fn api_actor(headers: &HeaderMap) -> String {
    let key = login_key(headers);
    if key == "unknown" {
        "api".to_string()
    } else {
        format!("api:{key}")
    }
}

fn spawn_job<F>(
    state: &ServerState,
    app: Option<&str>,
    dry_run: bool,
    actor: Option<String>,
    work: F,
) -> impl IntoResponse
where
    F: FnOnce(&mut Run) -> Result<serde_json::Value, String> + Send + 'static,
{
    let permit = match JOB_SLOTS.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return error_response(
                StatusCode::TOO_MANY_REQUESTS,
                "too many concurrent operations; wait for one to finish",
            );
        }
    };
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
    let app_name = app.map(str::to_string);
    let mut run = match Run::start_as(
        log,
        mode,
        app_name.clone(),
        &argv,
        crate::redact::Redactor::new(),
        Some(actor.unwrap_or_else(|| "api".to_string())),
    ) {
        Ok(r) => r,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let run_id = run.id().to_string();
    let run_id2 = run_id.clone();
    std::thread::spawn(move || {
        let _permit = permit;
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
                let failure = result.as_ref().err().cloned();
                let _ = run.finish(status);
                if let Some(error) = failure {
                    tracing::warn!(run = %run_id2, "job failed: {error}");
                    if !dry_run
                        && let Err(e) =
                            crate::notify::send_failure(app_name.as_deref(), &run_id2, &error)
                    {
                        tracing::warn!(run = %run_id2, "failure webhook failed: {e}");
                    }
                }
            }
            Err(payload) => {
                let _ = run.finish(RunStatus::Failed);
                tracing::error!(run = %run_id2, "job panicked; run closed as failed");
                if !dry_run
                    && let Err(e) =
                        crate::notify::send_failure(app_name.as_deref(), &run_id2, "job panicked")
                {
                    tracing::warn!(run = %run_id2, "failure webhook failed: {e}");
                }
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
    /// Deploy even when the app is already on that release.
    #[serde(default)]
    force: bool,
}

async fn deploy_app(
    State(state): State<Shared>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Option<DeployBody>>,
) -> impl IntoResponse {
    let body = body.unwrap_or_default();
    let app = match load_app(&state, &name) {
        Ok(a) => a,
        Err(e) => return error_response(StatusCode::NOT_FOUND, &e.to_string()),
    };
    let release = match deploy::resolve_release(&app, body.release.as_deref(), body.force) {
        Ok(r) => r,
        Err(e) => return error_response(StatusCode::CONFLICT, &e.to_string()),
    };
    let ctx = ctx_for(&state);
    let mut deploy_app = app;
    spawn_job(
        &state,
        Some(&name),
        body.dry_run,
        Some(api_actor(&headers)),
        move |run| {
            deploy::deploy(run, &ctx, &mut deploy_app, &release)
                .map(|()| serde_json::json!({}))
                .map_err(|e| e.to_string())
        },
    )
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
    headers: HeaderMap,
    Json(body): Json<Option<DryBody>>,
) -> impl IntoResponse {
    let dry = body.as_ref().map(|b| b.dry_run).unwrap_or(false);
    let mut app = match load_app(&state, &name) {
        Ok(a) => a,
        Err(e) => return error_response(StatusCode::NOT_FOUND, &e.to_string()),
    };
    let ctx = ctx_for(&state);
    spawn_job(
        &state,
        Some(&name),
        dry,
        Some(api_actor(&headers)),
        move |run| {
            deploy::rollback(run, &ctx, &mut app)
                .map(|old| serde_json::json!({"rolled_back_to": old}))
                .map_err(|e| e.to_string())
        },
    )
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
    headers: HeaderMap,
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
            &format!("invalid release {release:?}: must be a commit SHA (hex, 7-64 chars)"),
        );
    }
    let env_host = std::env::var("SWAPDOCK_BUILD_HOST").ok();
    let host = builder::resolve_host(body.build_host.as_deref(), env_host.as_deref(), &app);
    spawn_job(
        &state,
        Some(&name),
        body.dry_run,
        Some(api_actor(&headers)),
        move |run| {
            let built = if builder::is_local(&host) {
                let work_parent = std::env::temp_dir().join("swapdock-build");
                builder::build_local(run, &app, &release, &work_parent)
            } else {
                builder::build_ssh(run, &app, &release, &host)
            }
            .map_err(|e| e.to_string())?;
            Ok(serde_json::json!({"image_ref": built.image_ref, "release": built.release}))
        },
    )
    .into_response()
}

// ---------------------------------------------------------------------------
// build-server: source and image reads, transfers
// ---------------------------------------------------------------------------

/// `GET /build/apps/{name}/git` — the source state of an app's checkout.
///
/// Read-only: it resolves refs and scans status through the exec chokepoint
/// without opening a run, so asking never pollutes the run log. Each probe is
/// independent; one failing ref yields a null and an error string rather than
/// failing the whole request.
async fn app_git(State(state): State<Shared>, Path(name): Path<String>) -> impl IntoResponse {
    let app = match load_app(&state, &name) {
        Ok(a) => a,
        Err(e) => return error_response(StatusCode::NOT_FOUND, &e.to_string()),
    };
    let Some(repo) = app.build_repo.clone().or(app.repo.clone()) else {
        return error_response(StatusCode::CONFLICT, "no repo recorded");
    };
    let branch = app.git_branch().to_string();
    match tokio::task::spawn_blocking(move || probe_git(&repo, &branch)).await {
        Ok(report) => (StatusCode::OK, Json(report)).into_response(),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("git probe failed: {e}"),
        ),
    }
}

/// Runs the three git probes without a run log, tolerating individual failures.
fn probe_git(repo: &std::path::Path, branch: &str) -> serde_json::Value {
    let mut errors: Vec<String> = Vec::new();

    let remote_ref = format!("origin/{branch}");
    let remote_sha =
        match exec::run_capture(&git::rev_parse_short(repo, &remote_ref).timeout(GIT_TIMEOUT)) {
            Ok(o) if o.success() && !o.stdout_trimmed().is_empty() => {
                Some(o.stdout_trimmed().to_string())
            }
            Ok(o) => {
                errors.push(format!(
                    "{remote_ref}: {}",
                    probe_reason(o.stderr_trimmed())
                ));
                None
            }
            Err(e) => {
                errors.push(format!("{remote_ref}: {e}"));
                None
            }
        };

    let head_sha = match exec::run_capture(&git::rev_parse_short(repo, "HEAD").timeout(GIT_TIMEOUT))
    {
        Ok(o) if o.success() && !o.stdout_trimmed().is_empty() => {
            Some(o.stdout_trimmed().to_string())
        }
        Ok(o) => {
            errors.push(format!("HEAD: {}", probe_reason(o.stderr_trimmed())));
            None
        }
        Err(e) => {
            errors.push(format!("HEAD: {e}"));
            None
        }
    };

    let dirty = match exec::run_capture(&git::status_porcelain(repo).timeout(GIT_TIMEOUT)) {
        Ok(o) if o.success() => Some(!o.stdout_trimmed().is_empty()),
        Ok(o) => {
            errors.push(format!("status: {}", probe_reason(o.stderr_trimmed())));
            None
        }
        Err(e) => {
            errors.push(format!("status: {e}"));
            None
        }
    };

    serde_json::json!({
        "repo": repo.display().to_string(),
        "branch": branch,
        "remote_sha": remote_sha,
        "head_sha": head_sha,
        "dirty": dirty,
        "error": if errors.is_empty() { None } else { Some(errors.join("; ")) },
    })
}

/// A non-empty probe error, or a generic line when git said nothing.
fn probe_reason(stderr: &str) -> &str {
    if stderr.is_empty() {
        "did not resolve"
    } else {
        stderr
    }
}

/// `GET /build/apps/{name}/images` — the local tags for an app's image repo.
///
/// Read-only, through the chokepoint with no run. An empty list is a valid
/// answer: the app has simply never been built on this host.
async fn app_images(State(state): State<Shared>, Path(name): Path<String>) -> impl IntoResponse {
    let app = match load_app(&state, &name) {
        Ok(a) => a,
        Err(e) => return error_response(StatusCode::NOT_FOUND, &e.to_string()),
    };
    let Some(image_repo) = app.image_repo.clone() else {
        return error_response(
            StatusCode::CONFLICT,
            &format!("{} has no image_repo recorded", app.name),
        );
    };
    match tokio::task::spawn_blocking(move || list_images(&image_repo)).await {
        Ok(Ok(images)) => {
            (StatusCode::OK, Json(serde_json::json!({"images": images}))).into_response()
        }
        Ok(Err(e)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &e),
        Err(e) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("image list failed: {e}"),
        ),
    }
}

/// Lists `docker images <repo>` as `{ref, tag, size, created}` records.
fn list_images(repo: &str) -> Result<Vec<serde_json::Value>, String> {
    let spec = exec::StepSpec::new("docker-images", "docker")
        .args([
            "images",
            repo,
            "--format",
            "{{.Repository}}:{{.Tag}}|{{.Size}}|{{.CreatedAt}}",
        ])
        .timeout(IMAGE_TIMEOUT);
    let outcome = exec::run_capture(&spec).map_err(|e| e.to_string())?;
    if !outcome.success() {
        return Err(format!(
            "docker images failed: {}",
            outcome.stderr_trimmed()
        ));
    }

    let mut images = Vec::new();
    for line in outcome.stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // The CreatedAt value is `YYYY-MM-DD HH:MM:SS +0000 UTC`, which has no
        // `|`, so three fields are enough.
        let mut fields = line.splitn(3, '|');
        let reference = fields.next().unwrap_or("").trim();
        if reference.is_empty() {
            continue;
        }
        let size = fields.next().map(str::trim).filter(|s| !s.is_empty());
        let created = fields.next().map(str::trim).filter(|s| !s.is_empty());
        let tag = reference
            .rsplit_once(':')
            .map(|(_, t)| t)
            .unwrap_or(reference);
        images.push(serde_json::json!({
            "ref": reference,
            "tag": tag,
            "size": size,
            "created": created,
        }));
    }
    Ok(images)
}

#[derive(Deserialize, Default)]
struct TransferBody {
    release: Option<String>,
    target: Option<String>,
}

/// `POST /build/apps/{name}/transfers` — save an image and load it on `target`.
///
/// Validation is complete before `spawn_job` runs, so a bad release or a missing
/// target answers 409 and opens no run.
async fn transfer_app(
    State(state): State<Shared>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Option<TransferBody>>,
) -> impl IntoResponse {
    let body = body.unwrap_or_default();
    let app = match load_app(&state, &name) {
        Ok(a) => a,
        Err(e) => return error_response(StatusCode::NOT_FOUND, &e.to_string()),
    };
    let Some(release) = body.release.clone() else {
        return error_response(StatusCode::CONFLICT, "no release: pass release");
    };
    if !crate::registry::is_valid_release_arg(&release) {
        return error_response(
            StatusCode::CONFLICT,
            &format!("invalid release {release:?}: must be a commit SHA (hex, 7-64 chars)"),
        );
    }
    let Some(target) = body.target.clone().filter(|t| !t.trim().is_empty()) else {
        return error_response(
            StatusCode::CONFLICT,
            "no target: pass an ssh or taildrop destination",
        );
    };
    if app.image_repo.is_none() {
        return error_response(
            StatusCode::CONFLICT,
            &format!(
                "{} has no image_repo recorded; set image_repo before transferring",
                app.name
            ),
        );
    }
    let dir = state.transfer_dir.clone();
    spawn_job(
        &state,
        Some(&name),
        false,
        Some(api_actor(&headers)),
        move |run| {
            transfer::transfer(run, &app, &release, &target, &dir)
                .map(|outcome| {
                    serde_json::json!({
                        "image_ref": outcome.image_ref,
                        "target": outcome.target,
                        "method": outcome.method,
                    })
                })
                .map_err(|e| e.to_string())
        },
    )
    .into_response()
}

async fn sync_app(
    State(state): State<Shared>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Option<DryBody>>,
) -> impl IntoResponse {
    let dry = body.as_ref().map(|b| b.dry_run).unwrap_or(false);
    let app = match load_app(&state, &name) {
        Ok(a) => a,
        Err(e) => return error_response(StatusCode::NOT_FOUND, &e.to_string()),
    };
    spawn_job(
        &state,
        Some(&name),
        dry,
        Some(api_actor(&headers)),
        move |run| {
            deploy::sync_repo(run, &app)
                .map(|sha| serde_json::json!({"head": sha}))
                .map_err(|e| e.to_string())
        },
    )
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
            let read = match read_trace(state.trace.clone()).await {
                Ok(r) => r,
                Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e),
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
    // The API must not be an arbitrary-file reader. Only the nginx log tree is
    // allowed, only a regular file, and only up to a bounded size. `/dev/zero`
    // and a multi-gigabyte file are both rejected before any read.
    const MAX_ACCESS_LOG_BYTES: u64 = 64 * 1024 * 1024;
    let canonical = match std::fs::canonicalize(&log_path) {
        Ok(path) => path,
        Err(e) => {
            return error_response(
                StatusCode::CONFLICT,
                &format!("access log {} is not readable: {e}", log_path.display()),
            );
        }
    };
    // Path::starts_with is component-aware; a string prefix would also accept
    // `/var/log/nginx-old/...`. Canonicalize first, then compare components.
    if !canonical.starts_with(std::path::Path::new("/var/log/nginx")) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "access_log must be under /var/log/nginx",
        );
    }
    match std::fs::metadata(&canonical) {
        Ok(meta) if !meta.is_file() => {
            return error_response(StatusCode::BAD_REQUEST, "access_log must be a regular file");
        }
        Ok(meta) if meta.len() > MAX_ACCESS_LOG_BYTES => {
            return error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "access_log exceeds the 64 MiB limit",
            );
        }
        Ok(_) => {}
        Err(e) => {
            return error_response(
                StatusCode::CONFLICT,
                &format!("access log {} is not readable: {e}", canonical.display()),
            );
        }
    }
    let log = canonical.clone();
    let lines = match tokio::task::spawn_blocking(move || crate::verify::read_lines(&log)).await {
        Ok(Ok(lines)) => lines,
        Ok(Err(e)) => return error_response(StatusCode::CONFLICT, &e.to_string()),
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("log read task failed: {e}"),
            );
        }
    };
    let report = crate::verify::verify(&app.hostnames, since_ms, lines.into_iter());
    let body = serde_json::to_value(report).expect("verify report is always serializable");
    (StatusCode::OK, Json(body)).into_response()
}

// ---------------------------------------------------------------------------
// runs
// ---------------------------------------------------------------------------

/// Reads the trace log off the async runtime. `TraceLog::read` is a blocking
/// file read; large logs must not stall unrelated requests on a worker thread.
async fn read_trace(path: PathBuf) -> Result<crate::trace::Read, String> {
    tokio::task::spawn_blocking(move || TraceLog::read(&path).map_err(|e| e.to_string()))
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Deserialize)]
struct RunsQuery {
    limit: Option<usize>,
}

async fn list_runs(State(state): State<Shared>, Query(q): Query<RunsQuery>) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(20);
    match read_trace(state.trace.clone()).await {
        Ok(read) => {
            let mut summaries: Vec<serde_json::Value> = Vec::new();
            for ev in &read.events {
                match ev {
                    TraceEvent::RunStart {
                        run_id,
                        ts,
                        mode,
                        app,
                        actor,
                        pid,
                        boot,
                        ..
                    } => summaries.push(serde_json::json!({
                        "run_id": run_id,
                        "started": ts,
                        "app": app,
                        "actor": actor,
                        // Serialize rather than `format!("{mode:?}")`: the
                        // frontend contract is the serde spelling ("live").
                        "status": serde_json::to_value(mode).unwrap_or(serde_json::Value::Null),
                        // `state` is the run's lifecycle, not its mode: a run
                        // whose process died must not read as "live" forever.
                        "state": serde_json::to_value(crate::trace::run_state(
                            *pid,
                            boot.as_deref(),
                            None,
                        ))
                        .unwrap_or(serde_json::Value::Null),
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
                            cur["state"] =
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
    let read = match read_trace(state.trace.clone()).await {
        Ok(r) => r,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };
    let events: Vec<&TraceEvent> = read
        .events
        .iter()
        .filter(|e| match e {
            TraceEvent::RunStart { run_id, .. }
            | TraceEvent::Step { run_id, .. }
            | TraceEvent::RunEnd { run_id, .. } => run_id.as_str() == id.as_str(),
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
    let path = state.trace.clone();
    let resume_id = id.clone();
    let resume = tokio::task::spawn_blocking(move || {
        TraceLog::resume_point(&path, &resume_id).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|result| result);
    match resume {
        Ok(Some(p)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "run_id": p.run_id.as_str(),
                "last_seq": p.last_seq,
                "last_step": p.last_step,
                "last_status": format!("{:?}", p.last_status),
                "non_ok_steps": p.non_ok_steps,
                "state": format!("{:?}", p.state()),
                "ended": p.ended.map(|s| format!("{s:?}")),
            })),
        )
            .into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, &format!("no steps for run {id}")),
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &e),
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
    let permit = match EVENT_SLOTS.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return error_response(
                StatusCode::TOO_MANY_REQUESTS,
                "too many open event streams; close one and retry",
            );
        }
    };
    let stream = stream::unfold(
        EventsState::new(state.trace.clone(), run_id.as_str().to_string(), permit),
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
                if let Err(e) = st.poll().await {
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
    /// Held until the stream is dropped; caps concurrent SSE tails.
    _permit: tokio::sync::OwnedSemaphorePermit,
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
    fn new(path: PathBuf, run_id: String, permit: tokio::sync::OwnedSemaphorePermit) -> Self {
        Self {
            _permit: permit,
            path,
            run_id,
            offset: 0,
            pending: VecDeque::new(),
            done: false,
            last_event: Instant::now(),
        }
    }

    /// Reads newly appended lines and queues the ones for this run.
    ///
    /// The file I/O runs on the blocking pool so a large log or a slow disk
    /// cannot stall unrelated HTTP/SSE tasks on an async worker.
    async fn poll(&mut self) -> std::io::Result<()> {
        let path = self.path.clone();
        let run_id = self.run_id.clone();
        let mut offset = self.offset;
        let (events, new_offset) = tokio::task::spawn_blocking(move || {
            read_appended(&path, &mut offset, &run_id).map(|events| (events, offset))
        })
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))??;
        self.offset = new_offset;
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
            transfer_dir: dir.path().join("transfer"),
            auth: AuthConfig::unauthenticated_for_tests(),
            rate: Arc::new(RateLimiter::new(240, 60)),
        };
        (state, dir)
    }

    #[tokio::test]
    async fn inspect_returns_registry_and_live_sections() {
        let (state, _dir) = test_state();
        let app = router(state);
        let res = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/apps/blog/inspect")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["app"]["name"], "blog");
        assert!(value["live"].is_object());
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
    async fn tls_refuses_missing_material_before_binding() {
        let (state, _dir) = test_state();
        let err = serve(
            state,
            "127.0.0.1:0",
            Some(TlsConfig {
                cert: std::path::PathBuf::from("/missing/cert.pem"),
                key: std::path::PathBuf::from("/missing/key.pem"),
            }),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("cert.pem"), "{err}");
    }

    #[tokio::test]
    async fn health_is_also_served_under_api() {
        let (state, _dir) = test_state();
        let (status, json) = get_json(state, "/api/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn auth_is_required_except_for_health() {
        let (mut state, _dir) = test_state();
        state.auth = AuthConfig::for_tests("0123456789abcdef");
        let app = router(state);

        let unauthenticated = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/apps")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

        let health = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(health.status(), StatusCode::OK);

        let bearer = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/apps")
                    .header("authorization", "Bearer 0123456789abcdef")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(bearer.status(), StatusCode::OK);

        let query = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/apps?token=0123456789abcdef")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // `?token=` is SSE-only; a normal API call must use the header.
        assert_eq!(query.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn session_login_sets_cookie_and_csrf_protects_mutations() {
        let (mut state, _dir) = test_state();
        state.auth = AuthConfig::for_tests("0123456789abcdef");
        let app = router(state);

        let login = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/login")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(r#"{"password":"0123456789abcdef"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(login.status(), StatusCode::OK);
        let cookie = login
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string();
        let bytes = axum::body::to_bytes(login.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let csrf = json["csrf_token"].as_str().unwrap().to_string();

        let session = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/session")
                    .header(header::COOKIE, &cookie)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(session.status(), StatusCode::OK);

        let body = r#"{"name":"session-created","kind":"container"}"#;
        let without_csrf = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/apps")
                    .header("content-type", "application/json")
                    .header(header::COOKIE, &cookie)
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(without_csrf.status(), StatusCode::FORBIDDEN);

        let garbage_bearer = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/apps")
                    .header("content-type", "application/json")
                    .header(header::COOKIE, &cookie)
                    .header(header::AUTHORIZATION, "Bearer not-the-token")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            garbage_bearer.status(),
            StatusCode::FORBIDDEN,
            "a syntactically valid but wrong bearer must not skip CSRF"
        );

        let with_csrf = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/apps")
                    .header("content-type", "application/json")
                    .header(header::COOKIE, &cookie)
                    .header("x-csrf-token", &csrf)
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(with_csrf.status(), StatusCode::FORBIDDEN);
        assert_ne!(with_csrf.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn drift_reports_not_deployed_for_a_fresh_app() {
        let (state, _dir) = test_state();
        let (status, json) = get_json(state, "/apps/blog/drift").await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["state"], "not_deployed");
        assert!(json["live"]["release"].is_null(), "{json}");
    }

    #[tokio::test]
    async fn apps_lists_the_registry() {
        let (state, _dir) = test_state();
        let app = router(state);
        let res = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/apps")
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

    #[test]
    fn probe_address_classes_are_public_or_refused() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "::1",
            "fd00::1",
        ] {
            assert!(!super::is_public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "2606:4700:4700::1111"] {
            assert!(super::is_public_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn probe_url_ports_default_and_parse() {
        assert_eq!(
            super::url_host_port("https://example.com/x"),
            Some(("example.com".into(), 443))
        );
        assert_eq!(
            super::url_host_port("http://example.com:8080/x"),
            Some(("example.com".into(), 8080))
        );
        assert_eq!(
            super::url_host_port("http://[::1]:9000/"),
            Some(("::1".into(), 9000))
        );
        assert_eq!(super::url_host_port("ftp://example.com"), None);
    }

    #[tokio::test]
    async fn import_reports_a_bad_compose_dir_as_a_client_error() {
        let (state, _dir) = test_state();
        let (status, _json) = post_json(
            state,
            "/import",
            serde_json::json!({"compose_dir": "/definitely/not/a/compose/dir"}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn an_orphaned_run_is_reported_as_interrupted() {
        let (state, _dir) = test_state();
        std::fs::write(
            &state.trace,
            "{\"event\":\"run_start\",\"run_id\":\"01J0ORPHAN\",\"ts\":\"2026-10-22T00:00:00Z\",\"ts_ms\":1,\"version\":\"0.2.0\",\"mode\":\"live\",\"app\":null,\"actor\":\"api\",\"pid\":0,\"argv\":[\"swapdock\"]}\n",
        )
        .unwrap();
        let (status, json) = get_json(state, "/runs").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json[0]["state"], "interrupted", "{json}");
    }

    #[tokio::test]
    async fn runs_is_empty_for_a_fresh_log() {
        let (state, _dir) = test_state();
        let app = router(state);
        let res = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/runs")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn status_reports_the_recorded_release_even_with_nothing_live() {
        // The blog app has no recorded release and no live port: the verdict is
        // "down", and asking must not dial anything or open a run.
        let (state, _dir) = test_state();
        let (status, json) = get_json(state, "/apps/blog/status").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["up"], false);
        assert_eq!(json["live_port"], serde_json::Value::Null);
        assert_eq!(json["release"], serde_json::Value::Null);
        assert_eq!(json["old_release"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn deploy_body_takes_force() {
        let body: DeployBody = serde_json::from_str(r#"{"force": true}"#).unwrap();
        assert!(body.force);
        // An absent field keeps the safe default: an explicit redeploy must be
        // asked for, never assumed.
        let body: DeployBody = serde_json::from_str("{}").unwrap();
        assert!(!body.force);
        assert_eq!(body.release, None);
    }

    // ---- helpers ----

    /// Drives one request through the router and decodes a JSON body.
    /// Tests that predate the `/api` mount keep their short paths; the helper
    /// adds the prefix unless the caller already supplied it.
    fn api_uri(uri: &str) -> String {
        if uri.starts_with("/api") || uri == "/health" {
            uri.to_string()
        } else {
            format!("/api{uri}")
        }
    }

    async fn get_json(state: ServerState, uri: &str) -> (StatusCode, serde_json::Value) {
        let res = router(state)
            .oneshot(
                axum::http::Request::builder()
                    .uri(api_uri(uri))
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

    /// Drives one JSON PUT through the router and decodes the response body.
    async fn put_json(
        state: ServerState,
        uri: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let res = router(state)
            .oneshot(
                axum::http::Request::builder()
                    .method("PUT")
                    .uri(api_uri(uri))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
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
    async fn post_json(
        state: ServerState,
        uri: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let res = router(state)
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(api_uri(uri))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
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
                    .uri(format!("/api/events?run={run_id}"))
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
        let error = json["error"].as_str().unwrap_or_default();
        assert!(error.contains("invalid health path"), "{error}");
        // The refusal explains itself, but must not smuggle the raw CRLF back
        // into the response.
        assert!(!error.contains('\r') && !error.contains('\n'), "{error:?}");
    }

    // ---- latest ----

    #[tokio::test]
    async fn update_app_saves_a_clean_edit_and_keeps_a_backup() {
        let (state, _dir) = test_state();
        let (status, before) = get_json(state.clone(), "/apps/blog").await;
        assert_eq!(status, StatusCode::OK);
        let mut edited = before["app"].clone();
        edited["hostnames"] = serde_json::json!(["blog.example.com", "www.blog.example.com"]);

        let (status, out) = put_json(state.clone(), "/apps/blog", edited).await;
        assert_eq!(status, StatusCode::OK, "{out}");
        assert_eq!(out["ok"], true);

        let (status, after) = get_json(state.clone(), "/apps/blog").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            after["app"]["hostnames"],
            serde_json::json!(["blog.example.com", "www.blog.example.com"])
        );
        assert!(state.registry.join("blog.toml.bak").exists());
    }

    #[tokio::test]
    async fn update_app_refuses_an_edit_with_errors() {
        let (state, _dir) = test_state();
        let (status, before) = get_json(state.clone(), "/apps/blog").await;
        assert_eq!(status, StatusCode::OK);
        let mut edited = before["app"].clone();
        edited["front_port"] = serde_json::json!(9002);

        let (status, out) = put_json(state, "/apps/blog", edited).await;
        assert_eq!(status, StatusCode::CONFLICT, "{out}");
        assert!(out["errors"].as_u64().unwrap_or(0) > 0);
    }

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

    // ---- build-server ----

    /// Builds a one-commit repo through the chokepoint, so the test itself
    /// honours the single-spawn rule. `None` when `git` is unavailable.
    fn make_repo(dir: &std::path::Path) -> Option<std::path::PathBuf> {
        let repo = dir.join("repo");
        std::fs::create_dir_all(&repo).ok()?;
        let setup_log = TraceLog::open(dir.join("setup.jsonl")).unwrap();
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
        setup.exec(&init).ok()?;
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
        let commit = setup.exec(&commit).ok()?;
        if !commit.success() {
            return None;
        }
        setup.finish(RunStatus::Ok).unwrap();
        Some(repo)
    }

    #[tokio::test]
    async fn git_reports_head_sha_for_a_temp_repo() {
        let (state, dir) = test_state();
        let Some(repo) = make_repo(dir.path()) else {
            eprintln!("SKIP git_reports_head_sha_for_a_temp_repo: git unavailable");
            return;
        };
        point_app_at_repo(&state, &repo);

        let (status, json) = get_json(state, "/api/build/apps/blog/git").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["branch"], "main");
        assert!(
            json["head_sha"].as_str().is_some_and(|s| !s.is_empty()),
            "a committed repo must report a HEAD sha: {json}"
        );
        assert_eq!(json["dirty"], false, "{json}");
        // There is no origin here, so the remote probe is the tolerated failure.
        assert!(json["remote_sha"].is_null(), "{json}");
        assert!(json["error"].as_str().is_some(), "{json}");
    }

    #[tokio::test]
    async fn git_without_a_repo_is_a_conflict() {
        let (state, _dir) = test_state();
        let (status, json) = get_json(state, "/api/build/apps/blog/git").await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(
            json["error"].as_str().unwrap().contains("no repo"),
            "{json}"
        );
    }

    #[tokio::test]
    async fn git_probe_does_not_write_a_run() {
        let (state, dir) = test_state();
        // Not a repo: every probe fails, but the request still answers 200 with
        // nulls, and no run may be written either way.
        let repo = dir.path().join("not-a-repo");
        std::fs::create_dir_all(&repo).unwrap();
        point_app_at_repo(&state, &repo);

        let (status, json) = get_json(state.clone(), "/api/build/apps/blog/git").await;
        assert_eq!(status, StatusCode::OK);
        assert!(json["head_sha"].is_null(), "{json}");

        let read = TraceLog::read(&state.trace).unwrap();
        assert!(
            read.events.is_empty(),
            "a read-only query must not appear in the run log: {:?}",
            read.events
        );
    }

    #[tokio::test]
    async fn images_lists_or_skips_when_docker_is_missing() {
        let (state, _dir) = test_state();
        let (status, json) = get_json(state, "/api/build/apps/blog/images").await;
        match status {
            StatusCode::OK => assert!(json["images"].is_array(), "{json}"),
            StatusCode::INTERNAL_SERVER_ERROR => {
                eprintln!("SKIP images_lists_or_skips_when_docker_is_missing: docker unavailable")
            }
            other => panic!("unexpected status {other}: {json}"),
        }
    }

    #[tokio::test]
    async fn transfers_reject_an_invalid_release_without_spawning() {
        let (state, _dir) = test_state();
        let (status, json) = post_json(
            state.clone(),
            "/api/build/apps/blog/transfers",
            serde_json::json!({"release": "main; evil", "target": "hetzner"}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(
            json["error"].as_str().unwrap().contains("invalid release"),
            "{json}"
        );

        let read = TraceLog::read(&state.trace).unwrap();
        assert!(
            read.events.is_empty(),
            "validation must run before a run is opened: {:?}",
            read.events
        );
    }

    #[tokio::test]
    async fn build_routes_are_served_under_api_only() {
        let (state, _dir) = test_state();
        let (status, _) = get_json(state.clone(), "/api/build/apps/blog/git").await;
        assert_eq!(status, StatusCode::CONFLICT);
        let res = router(state)
            .oneshot(
                axum::http::Request::builder()
                    .uri("/build/apps/blog/git")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            res.status(),
            StatusCode::NOT_FOUND,
            "the un-prefixed compatibility route must be gone"
        );
    }

    #[tokio::test]
    async fn builds_route_reuses_the_build_handler() {
        let (state, _dir) = test_state();
        // No release recorded on the test app: the shared handler 409s, which
        // proves the route reached it rather than 404ing.
        let (status, _) =
            post_json(state, "/api/build/apps/blog/builds", serde_json::json!({})).await;
        assert_eq!(status, StatusCode::CONFLICT);
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

    #[test]
    fn drift_state_covers_every_card_state() {
        assert_eq!(
            drift_state(None, Some("abc1234"), false, true, true),
            "not_deployed"
        );
        assert_eq!(
            drift_state(Some("abc1234"), Some("def5678"), true, false, true),
            "no_source"
        );
        assert_eq!(
            drift_state(Some("abc1234"), None, false, true, true),
            "unknown"
        );
        assert_eq!(
            drift_state(Some("abc1234"), Some("abc1234"), false, true, true),
            "up_to_date"
        );
        assert_eq!(
            drift_state(Some("abc1234"), Some("abcdef0"), true, true, true),
            "behind_built"
        );
        assert_eq!(
            drift_state(Some("abc1234"), Some("abcdef0"), false, true, true),
            "behind_unbuilt"
        );
        assert_eq!(
            drift_state(Some("abc1234"), Some("abcdef0"), false, true, false),
            "live_unbuilt"
        );
    }

    #[test]
    fn drift_image_lookup_accepts_short_and_long_commits() {
        let images = vec![serde_json::json!({
            "ref": "app:9c1f2ab",
            "tag": "9c1f2ab",
            "size": "1MB",
            "created": "2026-10-21 00:00:00 +0000 UTC"
        })];
        assert!(image_for(&images, "9c1f2ab").is_some());
        assert!(image_for(&images, "9c1f2ab1234567890").is_some());
        assert!(image_for(&images, "deadbee").is_none());
    }

    #[test]
    fn probe_host_parsing_rejects_userinfo_and_keeps_the_host() {
        assert_eq!(
            url_host("https://example.com/up").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            url_host("http://example.com:8080/up").as_deref(),
            Some("example.com")
        );
        assert_eq!(url_host("http://[::1]:8080/up").as_deref(), Some("::1"));
        assert!(url_host("http://user:pass@example.com/up").is_none());
        assert!(url_host("ftp://example.com/up").is_none());
    }

    #[test]
    fn probe_is_limited_to_the_apps_own_hosts() {
        let app = crate::registry::tests::sample();
        assert!(probe_target_allowed(&app, "https://example.com/health"));
        assert!(probe_target_allowed(&app, "http://example.com:8080/health"));
        assert!(!probe_target_allowed(
            &app,
            "http://169.254.169.254/latest/meta-data"
        ));
        assert!(!probe_target_allowed(&app, "http://127.0.0.1:8088/health"));
    }
}

#[cfg(test)]
mod rate_limit_tests {
    use super::RateLimiter;

    #[test]
    fn the_api_actor_carries_the_forwarded_client_ip() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            axum::http::HeaderValue::from_static("203.0.113.9, 10.0.0.1"),
        );
        assert_eq!(super::api_actor(&headers), "api:203.0.113.9");
        assert_eq!(super::api_actor(&axum::http::HeaderMap::new()), "api");
    }

    #[test]
    fn a_client_is_refused_after_the_fixed_window_budget() {
        let limiter = RateLimiter::new(2, 60);
        assert!(limiter.allow("127.0.0.1"));
        assert!(limiter.allow("127.0.0.1"));
        assert!(!limiter.allow("127.0.0.1"));
        assert!(limiter.allow("127.0.0.2"));
    }
}
