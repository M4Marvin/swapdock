//! `swapdock import`: onboarding an existing Compose service.
//!
//! The job of this module is narrow, and deliberately so: read what Compose will
//! actually run, derive a registry entry from it, and write that entry plus a
//! generated override. It does **not** start, stop or deploy anything. The first
//! `swapdock up` is still a separate, explicit decision.
//!
//! Why an override instead of editing the user's file:
//!
//! * the compose file stays the operator's source of truth, working by hand;
//! * the generated file pins `${IMAGE}` and the loopback port, and can install a
//!   healthcheck, without touching the checked-in YAML;
//! * removing the import means deleting two files, not reverting a patch.
//!
//! The merged model comes from `docker compose config --format json`, reached
//! through the exec chokepoint. This is a read-only probe, so it uses
//! [`exec::run_capture`] and deliberately writes no run record: `--dry-run`
//! must write nothing at all, and a non-root import into a writable
//! `--registry`/`--state-dir` must not need `/var/log`.
//!
//! ## What is detected
//!
//! | Field | Source |
//! |---|---|
//! | service | `--service`, or the only service in the file |
//! | image | `image`, with the release tag/digest stripped for `image_repo` |
//! | container port | first `ports[].target`, then `expose[0]`, then 80 |
//! | healthcheck | existing `healthcheck.test`, or `--healthcheck` |
//! | health path | the first `http(s)://...` in the healthcheck command |
//! | state | named volumes force `replace` + `writes_state` |
//! | dependencies | `depends_on`/`links`/`volumes_from` force `replace` |
//!
//! A service with no image (build-only) is refused: swapdock deploys tagged
//! images, and inventing a tag the daemon does not have would fail later and
//! less clearly.
//!
//! Ports and slots are allocated from the existing registry, never reused, and
//! front ports are kept out of [`crate::ports::RESERVED_BACK`]. Explicit values
//! are validated against both rules before anything is written.
//!
//! The staged estate is validated as a whole (existing apps + the import), so a
//! duplicate hostname or slot is refused here rather than at the first deploy.

use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::compose;
use crate::deploy;
use crate::exec::{self, StepSpec};
use crate::ports;
use crate::registry::{self, App, ImageRegistry, Kind, Loaded, Problem, Strategy};
use crate::validator::{self, TunnelRoutes};

/// Default directory whose sibling `overrides/` holds generated overrides.
///
/// Matches `DeployPaths::state_dir`; `--state-dir` overrides it. The override
/// itself lands in `state_dir.parent()/overrides/<app>.yml`, the same layout
/// `swapdock adopt` uses.
pub const DEFAULT_STATE_DIR: &str = "/srv/swapdock/green";

/// First front port considered when the registry does not pin one.
const FIRST_FRONT_PORT: u16 = 8001;

/// Everything the CLI passes to [`run`].
#[derive(Debug, Clone)]
pub struct Options {
    /// Directory holding the compose file.
    pub compose_dir: PathBuf,
    /// Service to import. Inferred when the file has exactly one.
    pub service: Option<String>,
    /// App name (slug). Defaults to the slugified service name.
    pub name: Option<String>,
    /// Hostname to route to the app. May be omitted and added later.
    pub domain: Option<String>,
    /// Front port. Defaults to the first free port from 8001.
    pub front_port: Option<u16>,
    /// Slot owning the back-end port pair. Defaults to the first free slot.
    pub slot: Option<u8>,
    /// Container-side port. Detected from `ports`/`expose`, else 80.
    pub container_port: Option<u16>,
    /// Healthcheck command to install when the service has none.
    pub healthcheck: Option<String>,
    /// Local checkout of the app source, kept for `swapdock sync`/`build`.
    pub repo: Option<PathBuf>,
    /// The green state directory; the override goes in its sibling `overrides/`.
    pub state_dir: PathBuf,
    /// Plan only: no registry file, no override, no directory creation.
    pub dry_run: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            compose_dir: PathBuf::from("."),
            service: None,
            name: None,
            domain: None,
            front_port: None,
            slot: None,
            container_port: None,
            healthcheck: None,
            repo: None,
            state_dir: PathBuf::from(DEFAULT_STATE_DIR),
            dry_run: false,
        }
    }
}

/// Where the effective healthcheck came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthcheckSource {
    /// The compose service already declares one; the override leaves it alone.
    Compose,
    /// The service has none, so `--healthcheck` is installed by the override.
    Flag,
    /// Neither exists. The import succeeds, but the deploy gate will refuse.
    Missing,
}

impl HealthcheckSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Compose => "compose healthcheck",
            Self::Flag => "--healthcheck (installed by the override)",
            Self::Missing => "none",
        }
    }
}

/// A named volume mount that makes a swap unsafe.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NamedVolume {
    source: String,
    target: String,
}

/// What was read out of the merged compose model.
#[derive(Debug, Clone)]
struct Detection {
    image: String,
    image_repo: String,
    container_port: u16,
    container_port_source: &'static str,
    strategy: Strategy,
    writes_state: bool,
    healthcheck: Option<String>,
    install_healthcheck: Option<String>,
    healthcheck_source: HealthcheckSource,
    health_path: String,
    named_volumes: Vec<NamedVolume>,
    dependencies: Vec<String>,
    warnings: Vec<Problem>,
    notes: Vec<String>,
}

/// Paths and names shared by the probe and the plan.
#[derive(Debug, Clone)]
struct Setup {
    compose_file: PathBuf,
    compose_dir: PathBuf,
    project: String,
    registry_dir: PathBuf,
}

/// A validated, not-yet-written import.
#[derive(Debug, Clone)]
struct Plan {
    app: App,
    compose_file: PathBuf,
    project: String,
    registry_dir: PathBuf,
    override_path: PathBuf,
    override_text: String,
    service: String,
    image: String,
    container_port: u16,
    container_port_source: &'static str,
    healthcheck: Option<String>,
    healthcheck_source: HealthcheckSource,
    strategy_reason: String,
    named_volumes: Vec<String>,
    dependencies: Vec<String>,
    notes: Vec<String>,
    warnings: Vec<Problem>,
}

impl Plan {
    fn registry_path(&self) -> PathBuf {
        self.registry_dir.join(format!("{}.toml", self.app.name))
    }

    fn report(&self, dry_run: bool, wrote: bool) -> Report {
        Report {
            app: self.app.clone(),
            compose_file: self.compose_file.clone(),
            registry_dir: self.registry_dir.clone(),
            registry_path: self.registry_path(),
            override_path: self.override_path.clone(),
            override_text: self.override_text.clone(),
            wrote,
            dry_run,
            service: self.service.clone(),
            image: self.image.clone(),
            container_port: self.container_port,
            container_port_source: self.container_port_source,
            healthcheck: self.healthcheck.clone(),
            healthcheck_source: self.healthcheck_source,
            strategy_reason: self.strategy_reason.clone(),
            named_volumes: self.named_volumes.clone(),
            dependencies: self.dependencies.clone(),
            notes: self.notes.clone(),
            warnings: self.warnings.clone(),
        }
    }
}

/// What `import` did (or would do), for the CLI to print.
#[derive(Debug, Clone)]
pub struct Report {
    pub app: App,
    pub compose_file: PathBuf,
    pub registry_dir: PathBuf,
    pub registry_path: PathBuf,
    pub override_path: PathBuf,
    pub override_text: String,
    /// True only when both files were written.
    pub wrote: bool,
    pub dry_run: bool,
    pub service: String,
    pub image: String,
    pub container_port: u16,
    pub container_port_source: &'static str,
    pub healthcheck: Option<String>,
    pub healthcheck_source: HealthcheckSource,
    pub strategy_reason: String,
    pub named_volumes: Vec<String>,
    pub dependencies: Vec<String>,
    /// Human-facing facts that are not problems (image tag handling, etc.).
    pub notes: Vec<String>,
    pub warnings: Vec<Problem>,
}

/// Why an import stopped before writing a registry file.
#[non_exhaustive]
#[derive(Debug)]
pub enum ImportError {
    /// A clear, single-line explanation with a next step.
    Message(String),
    /// The staged estate (existing apps + the import) failed validation.
    Refused(Vec<Problem>),
}

impl ImportError {
    fn message(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Message(message) => f.write_str(message),
            Self::Refused(problems) => {
                write!(
                    f,
                    "refusing to import: {} error(s) in the staged registry; nothing was written:",
                    problems.len()
                )?;
                for problem in problems {
                    write!(f, "\n  {problem}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for ImportError {}

impl From<String> for ImportError {
    fn from(message: String) -> Self {
        Self::Message(message)
    }
}

// ---------------------------------------------------------------------------
// pipeline
// ---------------------------------------------------------------------------

/// Probes compose, builds a plan, validates it, then writes the files.
///
/// `registry_dir` is where `<name>.toml` goes (the global `--registry`);
/// `tunnel_config` is the optional global `--tunnel-config`, cross-checked when
/// supplied. The compose probe is the only subprocess.
pub fn run(
    opts: &Options,
    registry_dir: &Path,
    tunnel_config: Option<&Path>,
) -> Result<Report, ImportError> {
    let compose_dir = compose_dir_abs(&opts.compose_dir)?;
    let compose_file = compose::resolve_compose_file(&compose_dir)
        .map_err(|e| ImportError::message(e.to_string()))?;
    let project = compose_project(&compose_dir);
    if !is_valid_project_name(&project) {
        return Err(ImportError::message(format!(
            "compose project name {project:?} (derived from the compose directory basename) is \
             invalid: compose names use lowercase letters, digits, '-' and '_' and start with a \
             letter or digit. Rename the directory or move the compose file."
        )));
    }

    let services = probe_services(&compose_file, &project)?;
    let service = choose_service(&services, opts.service.as_deref())?;
    let config = probe_config(&compose_file, &project, &service)?;

    let registry_dir = absolute_path(registry_dir)?;
    let loaded = registry::load_dir(&registry_dir).map_err(|e| {
        ImportError::message(format!(
            "could not read the registry directory {}: {e}",
            registry_dir.display()
        ))
    })?;

    let mut tunnel = TunnelRoutes::new();
    let mut extra_problems = Vec::new();
    if let Some(path) = tunnel_config {
        match crate::tunnel::read_config(path) {
            Ok(parsed) => {
                tunnel = parsed.routes;
                extra_problems.extend(parsed.problems);
            }
            Err(e) => extra_problems.push(Problem::warning(
                "tunnel-config-unreadable",
                None,
                format!(
                    "{}: {e}; tunnel route cross-checks were skipped",
                    path.display()
                ),
            )),
        }
    }

    let setup = Setup {
        compose_file,
        compose_dir,
        project,
        registry_dir,
    };
    let plan = build_plan(
        &config,
        &service,
        opts,
        &loaded,
        &setup,
        &tunnel,
        &extra_problems,
    )?;
    execute(&plan, opts.dry_run)
}

/// Reads the merged model for one service and derives an [`App`].
///
/// Pure apart from reading `loaded` and the options: no filesystem writes, no
/// subprocess. That is what lets the dry-run path be tested without Docker.
fn build_plan(
    config: &Value,
    service: &str,
    opts: &Options,
    loaded: &Loaded,
    setup: &Setup,
    tunnel: &TunnelRoutes,
    extra_problems: &[Problem],
) -> Result<Plan, ImportError> {
    let name = resolve_name(service, opts.name.as_deref())?;
    let registry_path = setup.registry_dir.join(format!("{name}.toml"));
    if registry_path.exists() {
        return Err(ImportError::message(format!(
            "{} already exists; refusing to overwrite an existing app. Choose --name or remove \
             the file first.",
            registry_path.display()
        )));
    }
    if loaded.apps.iter().any(|app| app.name == name) {
        return Err(ImportError::message(format!(
            "the registry already declares an app named {name:?}; choose --name"
        )));
    }

    let detection = detect(config, service, &name, opts)?;
    let (front_port, slot) = allocate(&loaded.apps, opts.front_port, opts.slot)?;
    let (back_port, _) = ports::pair(slot).expect("allocate returns a slot below MAX_SLOTS");

    let state_dir = absolute_path(&opts.state_dir)?;
    let override_path = deploy::adopt_override_path(&state_dir, &name);
    let env_name = env_name(&name);
    let override_text = deploy::adopt_override(
        service,
        &env_name,
        detection.container_port,
        back_port,
        &detection.image_repo,
        detection.install_healthcheck.as_deref(),
    );

    let repo = match &opts.repo {
        Some(path) => Some(absolute_path(path)?),
        None => None,
    };

    let app = App {
        name: name.clone(),
        kind: Kind::Container,
        strategy: detection.strategy,
        hostnames: opts.domain.iter().cloned().collect(),
        listen: Vec::new(),
        front_port,
        slot,
        live_port: None,
        old_port: None,
        writes_state: detection.writes_state,
        image_repo: Some(detection.image_repo.clone()),
        registry: Some(ImageRegistry::Local),
        release: None,
        old_release: None,
        build_host: Some("local".into()),
        root: None,
        spa: false,
        health_url: Some(format!("http://127.0.0.1{}", detection.health_path)),
        compose_dir: setup.compose_dir.clone(),
        compose_svc: service.to_string(),
        env_name,
        git_remote: None,
        branch: Some("main".into()),
        repo,
        build_repo: None,
        compose_override: Some(override_path.clone()),
        public_url: None,
        backup: None,
    };

    // Validate the estate as it will be after the import, including load-time
    // problems and cross-app checks. An import must not be the thing that makes
    // `swapdock validate` fail for the first time.
    let mut problems = loaded.problems.clone();
    for existing in &loaded.apps {
        problems.extend(existing.problems());
    }
    problems.extend(app.problems());
    let mut staged = loaded.apps.clone();
    staged.push(app.clone());
    problems.extend(validator::validate(&staged, tunnel));
    problems.extend(detection.warnings.iter().cloned());
    problems.extend(extra_problems.iter().cloned());
    validator::sort_problems(&mut problems);

    let errors: Vec<Problem> = problems.iter().filter(|p| p.is_error()).cloned().collect();
    if !errors.is_empty() {
        return Err(ImportError::Refused(errors));
    }
    let warnings = problems.into_iter().filter(|p| !p.is_error()).collect();

    let strategy_reason = match (detection.writes_state, detection.dependencies.as_slice()) {
        (true, _) => format!(
            "replace (named volumes: {})",
            detection
                .named_volumes
                .iter()
                .map(|v| v.source.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        (false, []) => "swap (no named volumes or dependencies)".to_string(),
        (false, deps) => format!("replace (dependencies: {})", deps.join(", ")),
    };

    Ok(Plan {
        app,
        compose_file: setup.compose_file.clone(),
        project: setup.project.clone(),
        registry_dir: setup.registry_dir.clone(),
        override_path,
        override_text,
        service: service.to_string(),
        image: detection.image,
        container_port: detection.container_port,
        container_port_source: detection.container_port_source,
        healthcheck: detection.healthcheck,
        healthcheck_source: detection.healthcheck_source,
        strategy_reason,
        named_volumes: detection
            .named_volumes
            .iter()
            .map(|v| v.source.clone())
            .collect(),
        dependencies: detection.dependencies,
        notes: detection.notes,
        warnings,
    })
}

/// Writes the override, proves Compose still accepts the merged model, then
/// writes the registry file. A dry run returns before touching the filesystem.
fn execute(plan: &Plan, dry_run: bool) -> Result<Report, ImportError> {
    if dry_run {
        return Ok(plan.report(true, false));
    }

    // The override precedes the registry file on purpose: a registry entry must
    // never point at an override that failed to validate or does not exist.
    std::fs::create_dir_all(&plan.registry_dir).map_err(|e| {
        ImportError::message(format!(
            "could not create the registry directory {}: {e}",
            plan.registry_dir.display()
        ))
    })?;
    if let Some(parent) = plan.override_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            ImportError::message(format!(
                "could not create the override directory {}: {e}",
                parent.display()
            ))
        })?;
    }

    let previous = std::fs::read(&plan.override_path).ok();
    std::fs::write(&plan.override_path, &plan.override_text).map_err(|e| {
        ImportError::message(format!(
            "could not write {}: {e}",
            plan.override_path.display()
        ))
    })?;

    let checked = match exec::run_capture(&compose::config_quiet(
        &plan.compose_file,
        &plan.project,
        &[plan.override_path.as_path()],
    )) {
        Ok(outcome) => outcome,
        Err(e) => {
            restore_override(&plan.override_path, previous.as_deref());
            return Err(ImportError::message(format!(
                "could not validate the generated override with `docker compose config`: {e}"
            )));
        }
    };
    if !checked.success() {
        restore_override(&plan.override_path, previous.as_deref());
        return Err(ImportError::message(format!(
            "the generated compose override was refused by `docker compose config`; no registry \
             file was written: {}",
            checked.stderr_trimmed()
        )));
    }

    let registry_path = plan.registry_path();
    if let Err(e) = registry::save_app(&plan.registry_dir, &plan.app) {
        restore_override(&plan.override_path, previous.as_deref());
        return Err(ImportError::message(format!(
            "could not write {}; the generated override was removed: {e}",
            registry_path.display()
        )));
    }

    Ok(plan.report(false, true))
}

/// Removes the override written by this attempt, or puts back what was there.
fn restore_override(path: &Path, previous: Option<&[u8]>) {
    match previous {
        Some(bytes) => {
            let _ = std::fs::write(path, bytes);
        }
        None => {
            let _ = std::fs::remove_file(path);
        }
    }
}

// ---------------------------------------------------------------------------
// compose probing
// ---------------------------------------------------------------------------

/// The list of service names, without running a container.
///
/// A separate small call (`--services`) is cheaper and safer than parsing the
/// whole merged model: the chokepoint keeps only an output tail, and a large
/// project can exceed it.
fn probe_services(compose_file: &Path, project: &str) -> Result<Vec<String>, ImportError> {
    let spec = StepSpec::new("compose-config-services", "docker")
        .args(["compose", "-p", project, "-f"])
        .arg(compose_file.display().to_string())
        .args(["config", "--services"]);

    let outcome = exec::run_capture(&spec).map_err(|e| {
        ImportError::message(format!(
            "could not run `docker compose config --services`: {e}; import needs Docker Compose v2 \
             and a readable compose file"
        ))
    })?;
    if !outcome.success() {
        return Err(ImportError::message(format!(
            "`docker compose config --services` failed for {}: {}",
            compose_file.display(),
            outcome.stderr_trimmed()
        )));
    }

    let mut services: Vec<String> = outcome
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    services.sort();
    services.dedup();
    Ok(services)
}

/// The merged JSON model for one service.
fn probe_config(compose_file: &Path, project: &str, service: &str) -> Result<Value, ImportError> {
    let spec = compose::config_json(compose_file, project, &[], service);
    let outcome = exec::run_capture(&spec).map_err(|e| {
        ImportError::message(format!(
            "could not run `docker compose config` for service {service:?}: {e}"
        ))
    })?;
    if !outcome.success() {
        return Err(ImportError::message(format!(
            "`docker compose config` failed for {} (service {service:?}): {}",
            compose_file.display(),
            outcome.stderr_trimmed()
        )));
    }
    serde_json::from_str(outcome.stdout_trimmed()).map_err(|e| {
        ImportError::message(format!(
            "`docker compose config --format json` did not return JSON for service {service:?}: {e}"
        ))
    })
}

/// Picks the requested service, or the only one defined.
fn choose_service(services: &[String], requested: Option<&str>) -> Result<String, ImportError> {
    let mut listed = services.to_vec();
    listed.sort();
    listed.dedup();

    match requested {
        Some(service) => {
            if listed.iter().any(|known| known == service) {
                Ok(service.to_string())
            } else {
                Err(ImportError::message(format!(
                    "no service named {service:?} in the compose file; available: {}",
                    list_or_none(&listed)
                )))
            }
        }
        None => match listed.as_slice() {
            [] => Err(ImportError::message("the compose file defines no services")),
            [only] => Ok(only.clone()),
            _ => Err(ImportError::message(format!(
                "the compose file has {} services; pass --service <NAME> to choose one: {}",
                listed.len(),
                listed.join(", ")
            ))),
        },
    }
}

// ---------------------------------------------------------------------------
// detection
// ---------------------------------------------------------------------------

/// Reads one merged service into a [`Detection`], with warnings for anything
/// the operator must know before the first deploy.
fn detect(
    config: &Value,
    service: &str,
    name: &str,
    opts: &Options,
) -> Result<Detection, ImportError> {
    let service_value = config
        .get("services")
        .and_then(|services| services.get(service))
        .ok_or_else(|| {
            ImportError::message(format!(
                "service {service:?} is missing from the merged compose model"
            ))
        })?;

    let image = service_value
        .get("image")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|image| !image.is_empty())
        .ok_or_else(|| {
            ImportError::message(format!(
                "service {service:?} has no image; build-only services cannot be imported yet. \
                 Give the service an image, or build and tag one first."
            ))
        })?;
    let image = image.to_string();
    let image_repo = image_repo_from(&image);
    if image_repo.is_empty() {
        return Err(ImportError::message(format!(
            "service {service:?} has image {image:?}, which has no repository name"
        )));
    }

    let mut warnings = Vec::new();
    let notes = vec![format!(
        "image_repo is {image_repo:?}; the release tag is appended at deploy time \
         ({image_repo}:<sha>), so build or tag that image before the first deploy"
    )];

    let (container_port, container_port_source) = container_port(service_value, service, opts)?;
    if let Some(explicit) = opts.container_port {
        let (published, exposed) = detected_container_ports(service_value);
        if !published.contains(&explicit) && !exposed.contains(&explicit) {
            let mut known = published;
            known.extend(exposed);
            known.sort_unstable();
            known.dedup();
            warnings.push(Problem::warning(
                "container-port-override",
                Some(name),
                format!(
                    "--container-port {explicit} is not among the ports or expose values compose \
                     reports ({}); confirm the app listens on {explicit}",
                    list_or_none(&known.iter().map(u16::to_string).collect::<Vec<_>>())
                ),
            ));
        }
    }

    let compose_healthcheck = compose_healthcheck(service_value);
    let (healthcheck, install_healthcheck, healthcheck_source) =
        match (&compose_healthcheck, &opts.healthcheck) {
            (Some(command), Some(_)) => {
                warnings.push(Problem::warning(
                    "healthcheck-flag-ignored",
                    Some(name),
                    "--healthcheck was ignored: the compose service already declares a \
                     healthcheck, and the override leaves it in place",
                ));
                (Some(command.clone()), None, HealthcheckSource::Compose)
            }
            (Some(command), None) => (Some(command.clone()), None, HealthcheckSource::Compose),
            (None, Some(flag)) => (
                Some(flag.clone()),
                Some(flag.clone()),
                HealthcheckSource::Flag,
            ),
            (None, None) => (None, None, HealthcheckSource::Missing),
        };

    let health_path = match &healthcheck {
        Some(command) => match health_path_from_command(command) {
            Some(path) => path,
            None => {
                warnings.push(Problem::warning(
                    "health-path-unknown",
                    Some(name),
                    format!(
                        "could not derive an HTTP path from the healthcheck {command:?}; \
                         health_url is http://127.0.0.1/ and the deploy gate will GET /. Set \
                         health_url in the registry if the app answers elsewhere."
                    ),
                ));
                "/".to_string()
            }
        },
        None => "/".to_string(),
    };

    if healthcheck_source == HealthcheckSource::Missing {
        warnings.push(Problem::warning(
            "no-healthcheck",
            Some(name),
            format!(
                "compose service {service:?} has no healthcheck; the deploy gate will refuse until \
                 one is added. Re-run with --healthcheck \"<command>\" or add one to the compose \
                 file."
            ),
        ));
    }

    let named_volumes = named_volumes(service_value);
    let dependencies = dependencies(service_value);
    let writes_state = !named_volumes.is_empty();
    let strategy = if writes_state || !dependencies.is_empty() {
        // Both cases break a swap: a green project mounts fresh volumes and
        // does not contain the depended-on services. `replace` stays in the
        // original project, so the volume and the dependencies survive.
        Strategy::Replace
    } else {
        Strategy::Swap
    };

    if !named_volumes.is_empty() {
        let mounted = named_volumes
            .iter()
            .map(|volume| format!("{:?} at {:?}", volume.source, volume.target))
            .collect::<Vec<_>>()
            .join(", ");
        warnings.push(Problem::warning(
            "named-volume-detected",
            Some(name),
            format!(
                "service {service:?} mounts named volume(s) {mounted}; strategy is 'replace' and \
                 writes_state is true, so a swap never runs against a fresh, empty volume"
            ),
        ));
    }
    if !dependencies.is_empty() {
        warnings.push(Problem::warning(
            "compose-dependency-detected",
            Some(name),
            format!(
                "service {service:?} depends on {}; strategy is 'replace' because a swap starts \
                 in a separate compose project where those services do not exist",
                dependencies.join(", ")
            ),
        ));
    }

    Ok(Detection {
        image,
        image_repo,
        container_port,
        container_port_source,
        strategy,
        writes_state,
        healthcheck,
        install_healthcheck,
        healthcheck_source,
        health_path,
        named_volumes,
        dependencies,
        warnings,
        notes,
    })
}

/// The container-side port, with its provenance for the plan summary.
fn container_port(
    service_value: &Value,
    service: &str,
    opts: &Options,
) -> Result<(u16, &'static str), ImportError> {
    if let Some(port) = opts.container_port {
        if port == 0 {
            return Err(ImportError::message(
                "--container-port 0 is not a usable port",
            ));
        }
        return Ok((port, "command line"));
    }

    let (published, exposed) = detected_container_ports(service_value);
    match published.as_slice() {
        [only] => return Ok((*only, "ports")),
        [] => {}
        many => return Err(ambiguous_port(service, "publishes", many)),
    }
    match exposed.as_slice() {
        [only] => Ok((*only, "expose")),
        [] => Ok((80, "default (no ports or expose)")),
        many => Err(ambiguous_port(service, "exposes", many)),
    }
}

fn ambiguous_port(service: &str, verb: &str, ports: &[u16]) -> ImportError {
    ImportError::message(format!(
        "service {service:?} {verb} several container ports ({}); pass --container-port <PORT> to \
         choose the one the app serves",
        ports
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// Unique `ports[].target` and `expose[]` values, in compose's order.
fn detected_container_ports(service_value: &Value) -> (Vec<u16>, Vec<u16>) {
    let mut published: Vec<u16> = service_value
        .get("ports")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(port_target)
        .collect();
    published.sort_unstable();
    published.dedup();

    let mut exposed: Vec<u16> = service_value
        .get("expose")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(port_target)
        .collect();
    exposed.sort_unstable();
    exposed.dedup();

    (published, exposed)
}

/// A port entry in the merged model is an object; string/number shapes are
/// accepted so unit tests can describe the model tersely.
fn port_target(value: &Value) -> Option<u16> {
    if let Some(object) = value.as_object() {
        return object
            .get("target")
            .and_then(Value::as_u64)
            .and_then(|port| u16::try_from(port).ok());
    }
    value
        .as_u64()
        .and_then(|port| u16::try_from(port).ok())
        .or_else(|| value.as_str().and_then(parse_port_spec))
}

/// The container side of `HOST:CONTAINER`, `CONTAINER`, or `IP:HOST:CONTAINER`.
fn parse_port_spec(spec: &str) -> Option<u16> {
    let spec = spec.split('/').next()?.trim();
    spec.rsplit(':').next()?.trim().parse().ok()
}

/// The command inside `healthcheck.test`, if there is a real one.
fn compose_healthcheck(service_value: &Value) -> Option<String> {
    let healthcheck = service_value.get("healthcheck")?;
    if healthcheck.get("disable").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let test = healthcheck.get("test")?;
    let parts: Vec<String> = match test {
        Value::String(command) => vec![command.clone()],
        Value::Array(values) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => return None,
    };
    if parts.is_empty() || parts[0].eq_ignore_ascii_case("NONE") {
        return None;
    }
    Some(healthcheck_command(&parts))
}

/// Turns a `CMD`/`CMD-SHELL` test array into one command string for display and
/// for the generated `CMD-SHELL` healthcheck.
fn healthcheck_command(parts: &[String]) -> String {
    match parts[0].as_str() {
        "CMD" | "CMD-SHELL" => parts[1..].join(" "),
        _ => parts.join(" "),
    }
}

/// The path of the first HTTP(S) URL in a healthcheck command, if any.
fn health_path_from_command(command: &str) -> Option<String> {
    let lower = command.to_ascii_lowercase();
    let start = lower.find("http://").or_else(|| lower.find("https://"))?;
    let rest = &command[start..];
    let end = rest
        .find(|c: char| {
            c.is_whitespace() || matches!(c, '"' | '\'' | '`' | ')' | ']' | '}' | ';' | '|')
        })
        .unwrap_or(rest.len());
    crate::health::checked_probe_path(&rest[..end]).ok()
}

/// Named volumes mounted by the service; bind mounts and anonymous volumes are
/// not state, because they resolve to the same host path or a fresh volume.
fn named_volumes(service_value: &Value) -> Vec<NamedVolume> {
    let mut out = Vec::new();
    for volume in service_value
        .get("volumes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if volume.get("type").and_then(Value::as_str) != Some("volume") {
            continue;
        }
        let Some(source) = volume
            .get("source")
            .and_then(Value::as_str)
            .filter(|source| !source.is_empty())
        else {
            continue;
        };
        out.push(NamedVolume {
            source: source.to_string(),
            target: volume
                .get("target")
                .and_then(Value::as_str)
                .unwrap_or("<unknown target>")
                .to_string(),
        });
    }
    out.sort_by(|a, b| a.source.cmp(&b.source));
    out.dedup_by(|a, b| a.source == b.source);
    out
}

/// Service names in `depends_on`, `links` and `volumes_from`.
fn dependencies(service_value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    match service_value.get("depends_on") {
        Some(Value::Object(map)) => out.extend(map.keys().cloned()),
        Some(Value::Array(items)) => {
            out.extend(items.iter().filter_map(Value::as_str).map(str::to_string))
        }
        _ => {}
    }
    for key in ["links", "volumes_from"] {
        for item in service_value
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let name = item.split(':').next().unwrap_or(item).trim();
            if !name.is_empty() {
                out.push(name.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

// ---------------------------------------------------------------------------
// naming and allocation
// ---------------------------------------------------------------------------

/// `--name`, or the service slug.
fn resolve_name(service: &str, explicit: Option<&str>) -> Result<String, ImportError> {
    match explicit {
        Some(name) => {
            if !registry::is_valid_slug(name) {
                return Err(ImportError::message(format!(
                    "--name {name:?} is not a valid app name: lowercase letters, digits and \
                     dashes, starting with a letter and not ending with a dash"
                )));
            }
            Ok(name.to_string())
        }
        None => {
            let slug = slugify(service);
            if !registry::is_valid_slug(&slug) {
                return Err(ImportError::message(format!(
                    "service {service:?} maps to app name {slug:?}, which is not a valid slug; \
                     pass --name with a lowercase name"
                )));
            }
            Ok(slug)
        }
    }
}

/// Lowercases a compose service name and turns runs of other characters into one
/// dash, so `web_api` becomes `web-api`.
fn slugify(service: &str) -> String {
    let mut out = String::new();
    for character in service.chars() {
        if character.is_ascii_alphanumeric() {
            out.push(character.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// `NAME` -> `NAME_PORT`.
fn env_name(name: &str) -> String {
    format!("{}_PORT", name.to_uppercase().replace('-', "_"))
}

/// First free front port from 8001 and first free slot, or the validated
/// explicit values. Never reuses another app's port or slot.
fn allocate(
    apps: &[App],
    front_port: Option<u16>,
    slot: Option<u8>,
) -> Result<(u16, u8), ImportError> {
    let front_port = match front_port {
        Some(port) => {
            if port == 0 {
                return Err(ImportError::message("--front-port 0 is not a usable port"));
            }
            if ports::RESERVED_BACK.contains(&port) {
                return Err(ImportError::message(format!(
                    "--front-port {port} is inside the reserved back-end range {}..={}; choose a \
                     port outside it",
                    ports::RESERVED_BACK.start(),
                    ports::RESERVED_BACK.end()
                )));
            }
            if let Some(owner) = port_owner(apps, port) {
                return Err(ImportError::message(format!(
                    "--front-port {port} is already used by app {owner:?}"
                )));
            }
            port
        }
        None => (FIRST_FRONT_PORT..=u16::MAX)
            .find(|port| !ports::RESERVED_BACK.contains(port) && port_owner(apps, *port).is_none())
            .ok_or_else(|| {
                ImportError::message(format!(
                    "no free front port at or above {FIRST_FRONT_PORT} outside the reserved \
                     back-end range"
                ))
            })?,
    };

    let slot = match slot {
        Some(slot) => {
            if slot >= ports::MAX_SLOTS {
                return Err(ImportError::message(format!(
                    "--slot {slot} is out of range; the maximum is {}",
                    ports::MAX_SLOTS - 1
                )));
            }
            if let Some(owner) = apps.iter().find(|app| app.slot == slot) {
                return Err(ImportError::message(format!(
                    "--slot {slot} is already owned by app {:?}",
                    owner.name
                )));
            }
            slot
        }
        None => (0..ports::MAX_SLOTS)
            .find(|candidate| !apps.iter().any(|app| app.slot == *candidate))
            .ok_or_else(|| {
                ImportError::message(format!(
                    "no free slot 0..={}; the registry already has {} apps",
                    ports::MAX_SLOTS - 1,
                    apps.len()
                ))
            })?,
    };

    Ok((front_port, slot))
}

/// The app already holding `port` as a front or back port.
fn port_owner(apps: &[App], port: u16) -> Option<&str> {
    apps.iter()
        .find(|app| {
            app.front_port == port || app.live_port == Some(port) || app.old_port == Some(port)
        })
        .map(|app| app.name.as_str())
}

// ---------------------------------------------------------------------------
// paths and small helpers
// ---------------------------------------------------------------------------

/// Absolute compose directory. Canonicalised because the compose project name
/// is derived from the last component, and a relative `.` must not depend on the
/// caller's working directory later.
fn compose_dir_abs(dir: &Path) -> Result<PathBuf, ImportError> {
    let absolute = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| {
                ImportError::message(format!(
                    "could not resolve compose directory {} against the working directory: {e}",
                    dir.display()
                ))
            })?
            .join(dir)
    };
    match std::fs::canonicalize(&absolute) {
        Ok(path) if path.is_dir() => Ok(path),
        Ok(path) => Err(ImportError::message(format!(
            "compose directory {} is not a directory",
            path.display()
        ))),
        Err(e) => Err(ImportError::message(format!(
            "cannot read compose directory {}: {e}; pass --compose-dir DIR",
            absolute.display()
        ))),
    }
}

/// Makes a path absolute without requiring it to exist (the state directory may
/// not have been created yet).
fn absolute_path(path: &Path) -> Result<PathBuf, ImportError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .map_err(|e| {
            ImportError::message(format!(
                "could not resolve {} against the working directory: {e}",
                path.display()
            ))
        })
}

/// Same derivation as [`App::compose_project`]: the compose directory basename.
fn compose_project(dir: &Path) -> String {
    dir.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "apps".to_string())
}

/// Compose project names are lowercase alphanumerics, dashes and underscores,
/// starting with a letter or digit. Checking here turns a generic compose error
/// into a directory-rename instruction.
fn is_valid_project_name(name: &str) -> bool {
    !name.is_empty()
        && name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Strips a trailing `:tag` or `@digest`, keeping a registry port.
fn image_repo_from(image: &str) -> String {
    let image = image.trim();
    let without_digest = image.split('@').next().unwrap_or(image);
    let last_slash = without_digest.rfind('/');
    match without_digest.rfind(':') {
        Some(colon) if last_slash.is_none_or(|slash| colon > slash) => {
            without_digest[..colon].to_string()
        }
        _ => without_digest.to_string(),
    }
}

fn list_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        items.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn options() -> Options {
        Options {
            compose_dir: PathBuf::from("/tmp/example-compose"),
            ..Options::default()
        }
    }

    fn merged(service: Value) -> Value {
        json!({"services": {"web": service}})
    }

    fn detect_service(service: Value, opts: &Options) -> Detection {
        detect(&merged(service), "web", "shop", opts).unwrap()
    }

    fn app(name: &str, front_port: u16, slot: u8) -> App {
        let mut app = crate::registry::tests::sample();
        app.name = name.to_string();
        app.front_port = front_port;
        app.slot = slot;
        app.live_port = None;
        app.old_port = None;
        app
    }

    // ---- image ----

    #[test]
    fn image_repo_strips_a_tag() {
        assert_eq!(
            image_repo_from("ghcr.io/acme/shop:1.4.2"),
            "ghcr.io/acme/shop"
        );
        assert_eq!(image_repo_from("shop:latest"), "shop");
    }

    #[test]
    fn image_repo_strips_a_digest() {
        assert_eq!(
            image_repo_from("ghcr.io/acme/shop@sha256:0123abcd"),
            "ghcr.io/acme/shop"
        );
        assert_eq!(
            image_repo_from("ghcr.io/acme/shop:1.4.2@sha256:0123abcd"),
            "ghcr.io/acme/shop"
        );
    }

    #[test]
    fn image_repo_keeps_a_registry_port() {
        assert_eq!(
            image_repo_from("localhost:5000/shop:2"),
            "localhost:5000/shop"
        );
        assert_eq!(
            image_repo_from("localhost:5000/shop"),
            "localhost:5000/shop"
        );
        assert_eq!(image_repo_from("ghcr.io/acme/shop"), "ghcr.io/acme/shop");
    }

    #[test]
    fn a_build_only_service_is_refused() {
        let err = detect(
            &merged(json!({"build": {"context": "."}})),
            "web",
            "shop",
            &options(),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("no image"), "{err}");
        assert!(err.contains("build-only"), "{err}");
    }

    // ---- service selection ----

    #[test]
    fn the_only_service_is_selected_implicitly() {
        assert_eq!(choose_service(&["web".to_string()], None).unwrap(), "web");
    }

    #[test]
    fn several_services_require_a_name_and_are_listed() {
        let services = vec!["api".to_string(), "web".to_string()];
        let err = choose_service(&services, None).unwrap_err().to_string();
        assert!(err.contains("--service"), "{err}");
        assert!(err.contains("api") && err.contains("web"), "{err}");
    }

    #[test]
    fn a_requested_service_must_exist() {
        let services = vec!["api".to_string(), "web".to_string()];
        assert_eq!(choose_service(&services, Some("web")).unwrap(), "web");
        let err = choose_service(&services, Some("db"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("db"), "{err}");
        assert!(err.contains("api") && err.contains("web"), "{err}");
    }

    #[test]
    fn a_service_name_becomes_a_slug() {
        assert_eq!(slugify("web"), "web");
        assert_eq!(slugify("Shop_API"), "shop-api");
        assert_eq!(slugify("_web_"), "web");
        assert_eq!(slugify("web.api"), "web-api");
        assert_eq!(resolve_name("Web_Api", None).unwrap(), "web-api");
        // A leading digit is not a valid slug; the operator must choose a name.
        assert!(resolve_name("1app", None).is_err());
    }

    // ---- ports ----

    #[test]
    fn ports_win_over_expose() {
        let detection = detect_service(
            json!({
                "image": "shop:1",
                "ports": [{"target": 8080}],
                "expose": ["9090"]
            }),
            &options(),
        );
        assert_eq!(detection.container_port, 8080);
        assert_eq!(detection.container_port_source, "ports");
    }

    #[test]
    fn expose_is_the_fallback() {
        let detection = detect_service(json!({"image": "shop:1", "expose": ["9090"]}), &options());
        assert_eq!(detection.container_port, 9090);
        assert_eq!(detection.container_port_source, "expose");
    }

    #[test]
    fn port_80_is_the_last_resort() {
        let detection = detect_service(json!({"image": "shop:1"}), &options());
        assert_eq!(detection.container_port, 80);
        assert!(detection.container_port_source.starts_with("default"));
    }

    #[test]
    fn several_distinct_targets_are_ambiguous() {
        let err = detect(
            &merged(json!({
                "image": "shop:1",
                "ports": [{"target": 80}, {"target": 443}]
            })),
            "web",
            "shop",
            &options(),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("--container-port"), "{err}");
        assert!(err.contains("80") && err.contains("443"), "{err}");
    }

    #[test]
    fn a_repeated_target_is_not_ambiguous() {
        let detection = detect_service(
            json!({
                "image": "shop:1",
                "ports": [{"target": 80}, {"target": 80}]
            }),
            &options(),
        );
        assert_eq!(detection.container_port, 80);
    }

    #[test]
    fn an_explicit_container_port_wins() {
        let mut opts = options();
        opts.container_port = Some(3000);
        let detection = detect_service(
            json!({
                "image": "shop:1",
                "ports": [{"target": 80}, {"target": 443}]
            }),
            &opts,
        );
        assert_eq!(detection.container_port, 3000);
        assert_eq!(detection.container_port_source, "command line");
    }

    // ---- healthcheck ----

    #[test]
    fn a_shell_healthcheck_yields_its_http_path() {
        assert_eq!(
            health_path_from_command(
                "wget -q -O /dev/null http://127.0.0.1:3000/healthz || exit 1"
            )
            .as_deref(),
            Some("/healthz")
        );
    }

    #[test]
    fn a_quoted_url_still_yields_a_path() {
        assert_eq!(
            health_path_from_command("sh -c 'curl -fsS https://example.com/api/health?full=1'")
                .as_deref(),
            Some("/api/health?full=1")
        );
    }

    #[test]
    fn a_healthcheck_without_a_url_has_no_path() {
        assert_eq!(health_path_from_command("pg_isready -U postgres"), None);
    }

    #[test]
    fn a_url_without_a_path_means_root() {
        assert_eq!(
            health_path_from_command("curl -f http://localhost:3000").as_deref(),
            Some("/")
        );
    }

    #[test]
    fn an_existing_healthcheck_is_used_and_not_reinstalled() {
        let detection = detect_service(
            json!({
                "image": "shop:1",
                "healthcheck": {
                    "test": ["CMD", "wget", "-q", "-O", "/dev/null", "http://127.0.0.1:3000/healthz"]
                }
            }),
            &options(),
        );
        assert_eq!(detection.healthcheck_source, HealthcheckSource::Compose);
        assert_eq!(detection.health_path, "/healthz");
        assert!(detection.install_healthcheck.is_none());
        assert_eq!(
            detection.healthcheck.as_deref(),
            Some("wget -q -O /dev/null http://127.0.0.1:3000/healthz")
        );
    }

    #[test]
    fn a_flag_healthcheck_is_installed_when_compose_has_none() {
        let mut opts = options();
        opts.healthcheck = Some("curl -fsS http://127.0.0.1:3000/ready".to_string());
        let detection = detect_service(json!({"image": "shop:1"}), &opts);
        assert_eq!(detection.healthcheck_source, HealthcheckSource::Flag);
        assert_eq!(detection.health_path, "/ready");
        assert_eq!(
            detection.install_healthcheck.as_deref(),
            Some("curl -fsS http://127.0.0.1:3000/ready")
        );
    }

    #[test]
    fn a_missing_healthcheck_warns_but_does_not_fail() {
        let detection = detect_service(json!({"image": "shop:1"}), &options());
        assert_eq!(detection.healthcheck_source, HealthcheckSource::Missing);
        assert_eq!(detection.health_path, "/");
        assert!(
            detection
                .warnings
                .iter()
                .any(|warning| warning.code == "no-healthcheck"),
            "{:?}",
            detection.warnings
        );
    }

    #[test]
    fn a_disabled_healthcheck_is_treated_as_absent() {
        let detection = detect_service(
            json!({"image": "shop:1", "healthcheck": {"test": ["NONE"]}}),
            &options(),
        );
        assert_eq!(detection.healthcheck_source, HealthcheckSource::Missing);
    }

    // ---- state and dependencies ----

    #[test]
    fn a_named_volume_forces_replace_and_state() {
        let detection = detect_service(
            json!({
                "image": "shop:1",
                "volumes": [{"type": "volume", "source": "shop-data", "target": "/data"}]
            }),
            &options(),
        );
        assert_eq!(detection.strategy, Strategy::Replace);
        assert!(detection.writes_state);
        assert_eq!(
            detection.named_volumes,
            vec![NamedVolume {
                source: "shop-data".into(),
                target: "/data".into()
            }]
        );
        assert!(
            detection
                .warnings
                .iter()
                .any(|warning| warning.code == "named-volume-detected"),
            "{:?}",
            detection.warnings
        );
    }

    #[test]
    fn bind_and_anonymous_volumes_do_not_make_the_app_stateful() {
        let detection = detect_service(
            json!({
                "image": "shop:1",
                "volumes": [
                    {"type": "bind", "source": "/srv/data", "target": "/data"},
                    {"type": "volume", "target": "/cache"}
                ]
            }),
            &options(),
        );
        assert_eq!(detection.strategy, Strategy::Swap);
        assert!(!detection.writes_state);
        assert!(detection.named_volumes.is_empty());
    }

    #[test]
    fn dependencies_switch_to_replace_without_state() {
        let detection = detect_service(
            json!({
                "image": "shop:1",
                "depends_on": {"db": {"condition": "service_started"}},
                "links": ["redis:cache"],
                "volumes_from": ["db:ro"]
            }),
            &options(),
        );
        assert_eq!(detection.strategy, Strategy::Replace);
        assert!(!detection.writes_state);
        assert_eq!(
            detection.dependencies,
            vec!["db".to_string(), "redis".to_string()]
        );
        assert!(
            detection
                .warnings
                .iter()
                .any(|warning| warning.code == "compose-dependency-detected"),
            "{:?}",
            detection.warnings
        );
    }

    // ---- allocation ----

    #[test]
    fn the_first_free_port_and_slot_are_allocated() {
        assert_eq!(allocate(&[], None, None).unwrap(), (8001, 0));

        let apps = vec![app("portfolio", 8001, 0), app("jobs", 8002, 1)];
        assert_eq!(allocate(&apps, None, None).unwrap(), (8003, 2));
    }

    #[test]
    fn an_occupied_front_port_is_refused() {
        let apps = vec![app("portfolio", 8001, 0)];
        let err = allocate(&apps, Some(8001), None).unwrap_err().to_string();
        assert!(err.contains("portfolio"), "{err}");
    }

    #[test]
    fn a_reserved_front_port_is_refused() {
        let err = allocate(&[], Some(9000), None).unwrap_err().to_string();
        assert!(err.contains("reserved"), "{err}");
    }

    #[test]
    fn an_occupied_slot_is_refused() {
        let apps = vec![app("portfolio", 8001, 0)];
        let err = allocate(&apps, None, Some(0)).unwrap_err().to_string();
        assert!(err.contains("portfolio"), "{err}");
    }

    #[test]
    fn a_slot_out_of_range_is_refused() {
        assert!(allocate(&[], None, Some(ports::MAX_SLOTS)).is_err());
        assert!(allocate(&[], None, Some(u8::MAX)).is_err());
    }

    #[test]
    fn allocation_skips_the_reserved_back_range() {
        let apps: Vec<App> = (8001..=8999)
            .map(|port| app(&format!("app{port}"), port, 0))
            .collect();
        assert_eq!(allocate(&apps, None, Some(7)).unwrap(), (9256, 7));
    }

    #[test]
    fn slot_allocation_stops_at_the_maximum() {
        let apps: Vec<App> = (0..ports::MAX_SLOTS)
            .map(|slot| app(&format!("app{slot}"), 8001 + u16::from(slot), slot))
            .collect();
        let err = allocate(&apps, None, None).unwrap_err().to_string();
        assert!(err.contains("no free slot"), "{err}");
    }

    // ---- plan + dry run ----

    fn setup(dir: &Path, registry_dir: PathBuf) -> Setup {
        let compose_dir = dir.join("shop");
        std::fs::create_dir_all(&compose_dir).unwrap();
        Setup {
            compose_file: compose_dir.join("compose.yaml"),
            compose_dir,
            project: "shop".into(),
            registry_dir,
        }
    }

    fn importable() -> Value {
        merged(json!({
            "image": "ghcr.io/acme/shop:1.4.2",
            "ports": [{"target": 3000}],
            "healthcheck": {
                "test": ["CMD-SHELL", "curl -f http://127.0.0.1:3000/healthz"]
            }
        }))
    }

    #[test]
    fn a_dry_run_writes_nothing() {
        let temp = tempfile::TempDir::new().unwrap();
        let registry_dir = temp.path().join("apps");
        let setup = setup(temp.path(), registry_dir.clone());
        let mut opts = options();
        opts.name = Some("shop".into());
        opts.state_dir = temp.path().join("state/green");

        let plan = build_plan(
            &importable(),
            "web",
            &opts,
            &Loaded::default(),
            &setup,
            &TunnelRoutes::new(),
            &[],
        )
        .unwrap();
        let override_path = plan.override_path.clone();
        assert!(!override_path.exists());

        let report = execute(&plan, true).unwrap();
        assert!(report.dry_run);
        assert!(!report.wrote);
        assert!(
            !registry_dir.exists(),
            "a dry run must not create the registry directory"
        );
        assert!(
            !override_path.exists(),
            "a dry run must not create the override"
        );
        assert!(!report.registry_path.exists());
        assert!(report.registry_path.ends_with("shop.toml"));
        assert_eq!(report.app.front_port, 8001);
        assert_eq!(report.app.slot, 0);
        assert_eq!(report.app.strategy, Strategy::Swap);
        assert_eq!(report.app.image_repo.as_deref(), Some("ghcr.io/acme/shop"));
        assert_eq!(
            report.app.health_url.as_deref(),
            Some("http://127.0.0.1/healthz")
        );
        assert_eq!(report.app.env_name, "SHOP_PORT");
        assert_eq!(report.app.compose_svc, "web");
        assert!(report.override_text.contains("!override"));
        assert!(
            report
                .override_text
                .contains("127.0.0.1:${SHOP_PORT:-9000}:3000")
        );
    }

    #[test]
    fn an_existing_app_is_not_overwritten() {
        let temp = tempfile::TempDir::new().unwrap();
        let registry_dir = temp.path().join("apps");
        std::fs::create_dir_all(&registry_dir).unwrap();
        std::fs::write(registry_dir.join("shop.toml"), "existing\n").unwrap();
        let setup = setup(temp.path(), registry_dir);
        let mut opts = options();
        opts.name = Some("shop".into());
        opts.state_dir = temp.path().join("state/green");

        let err = build_plan(
            &importable(),
            "web",
            &opts,
            &Loaded::default(),
            &setup,
            &TunnelRoutes::new(),
            &[],
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("already exists"), "{err}");
    }

    #[test]
    fn the_project_name_matches_the_registry_derivation() {
        let mut sample = crate::registry::tests::sample();
        sample.compose_dir = PathBuf::from("/srv/example/apps");
        assert_eq!(
            compose_project(&sample.compose_dir),
            sample.compose_project()
        );
    }

    #[test]
    fn a_project_name_with_uppercase_or_dots_is_refused() {
        assert!(is_valid_project_name("apps"));
        assert!(is_valid_project_name("shop-2"));
        assert!(is_valid_project_name("shop_2"));
        assert!(!is_valid_project_name("Shop"));
        assert!(!is_valid_project_name("shop.2"));
        assert!(!is_valid_project_name(""));
        assert!(!is_valid_project_name("-shop"));
    }
}
