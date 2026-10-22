//! Command line surface.
//!
//! Milestone 1 delivers four commands, all of which exist to make the
//! chokepoint and the run log visible:
//!
//! * `selftest` — run a fixed set of commands through the chokepoint and show
//!   what each one produced, including a secret that must be redacted.
//! * `runs` — list recent runs from the log, newest first.
//! * `show` — print every step of one run, in order.
//! * `resume` — report where a run stopped.
//!
//! The remaining commands (`register`, `build`, `up`, `rollback`, `plan`,
//! `status`, `verify`, `render`) arrive in later milestones. Their subcommands
//! are listed in `lib.rs` so the shape of the tool is fixed early.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};

use crate::apply::{self, ApplyPaths};
use crate::builder;
use crate::deploy::{self, DeployContext};
use crate::exec::{ExecError, StepSpec};
use crate::id::RunId;
use crate::redact::Redactor;
use crate::registry::{App, Loaded, Problem};
use crate::render;
use crate::trace::{Run, RunMode, RunStatus, TraceEvent, TraceLog};
use crate::tunnel;
use crate::validator::{self, TunnelRoutes};

const ABOUT: &str = "Registry-driven blue/green deploys for a multi-app Docker host,\n\
                      behind an nginx front door.";

/// Paths shared by every command that can change the host.
#[derive(Debug, Clone, clap::Args)]
pub struct DeployPaths {
    /// Where the generated nginx file goes.
    #[arg(
        long,
        default_value = "/etc/nginx/conf.d/front-door.conf",
        value_name = "PATH"
    )]
    pub target: PathBuf,

    /// Main nginx config, tested as a whole. The target must be included from it.
    #[arg(long, default_value = "/etc/nginx/nginx.conf", value_name = "PATH")]
    pub main_config: PathBuf,

    /// Master pid, for the reload-vs-restart check.
    #[arg(long, default_value = "/run/nginx.pid", value_name = "PATH")]
    pub pid_file: PathBuf,

    /// nginx binary. A bare name resolves through `PATH`.
    #[arg(long, default_value = "nginx", value_name = "PATH")]
    pub nginx_bin: PathBuf,

    /// Directory for generated compose overrides.
    #[arg(long, default_value = "/srv/swapdock/green", value_name = "DIR")]
    pub state_dir: PathBuf,

    /// Directory for lock files.
    #[arg(long, default_value = "/run/swapdock", value_name = "DIR")]
    pub lock_dir: PathBuf,

    /// Seconds to let old workers drain after the flip.
    #[arg(long, default_value_t = crate::deploy::DRAIN_SECS)]
    pub drain_secs: u64,
}

/// Arguments for `swapdock import`.
#[derive(Debug, Clone, clap::Args)]
pub struct ImportArgs {
    /// Directory that holds the compose file. Defaults to the current directory.
    #[arg(long, value_name = "DIR")]
    pub compose_dir: Option<PathBuf>,

    /// Compose service to import. Inferred when the file defines exactly one.
    #[arg(long, value_name = "NAME")]
    pub service: Option<String>,

    /// App name (slug). Defaults to the service name, slugified.
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,

    /// Hostname to route to the app. May be omitted and added later.
    #[arg(long, value_name = "HOST")]
    pub domain: Option<String>,

    /// Front port. Defaults to the first free port from 8001.
    #[arg(long, value_name = "PORT")]
    pub front_port: Option<u16>,

    /// Slot owning the back-end port pair. Defaults to the first free slot.
    #[arg(long, value_name = "SLOT")]
    pub slot: Option<u8>,

    /// Container-side port. Detected from ports/expose, else 80.
    #[arg(long, value_name = "PORT")]
    pub container_port: Option<u16>,

    /// Healthcheck command to install when the service has none.
    #[arg(long, value_name = "CMD")]
    pub healthcheck: Option<String>,

    /// Local checkout of the app source, for `swapdock build`/`sync`.
    #[arg(long, value_name = "PATH")]
    pub repo: Option<PathBuf>,

    /// Green state directory; the override lands in its sibling `overrides/`.
    #[arg(
        long,
        default_value = crate::import::DEFAULT_STATE_DIR,
        value_name = "DIR"
    )]
    pub state_dir: PathBuf,
}

/// Parsed command line: global paths, an optional run label, and one
/// subcommand.
#[derive(Debug, Parser)]
#[command(name = "swapdock", version, about = ABOUT, long_about = None)]
pub struct Cli {
    /// Path to the append-only run log.
    ///
    /// Defaults to the shared host log the installer creates and logrotate
    /// watches, so every command — CLI or API — writes and reads one timeline.
    /// Pass `--trace ./swapdock.jsonl` for a local, unprivileged log.
    #[arg(
        long,
        global = true,
        default_value = "/var/log/swapdock/swapdock.jsonl",
        value_name = "PATH"
    )]
    pub trace: PathBuf,

    /// Record the plan but spawn nothing.
    #[arg(long, global = true)]
    pub dry_run: bool,

    /// Label this run with an application name.
    #[arg(long, global = true, value_name = "NAME")]
    pub app: Option<String>,

    /// Directory holding one registry file per app.
    #[arg(
        long,
        global = true,
        default_value = "/srv/swapdock/apps",
        value_name = "DIR"
    )]
    pub registry: PathBuf,

    /// cloudflared config to cross-check routes against.
    #[arg(long, global = true, value_name = "PATH")]
    pub tunnel_config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

/// The subcommand to run.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run a fixed set of commands through the chokepoint and show the results.
    Selftest,

    /// List recent runs from the log, newest first.
    Runs {
        /// How many runs to show.
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },

    /// Print every step of one run, in order.
    Show {
        /// Run id, as shown by `swapdock runs`.
        run_id: String,
    },

    /// Report where a run stopped, so it can be resumed.
    Resume { run_id: String },

    /// Print the nginx front-door config. Changes nothing.
    Render {
        /// Render only this app's block.
        #[arg(long, value_name = "NAME")]
        app: Option<String>,
    },

    /// Check the registry: per-app rules, cross-app clashes and tunnel routes.
    Validate,

    /// Check the host as well: compose paths, mounts, ports, images and health.
    Doctor {
        /// Check only this app.
        #[arg(long, value_name = "NAME")]
        app: Option<String>,
        /// Also probe each app's public_url.
        #[arg(long)]
        public: bool,
    },

    /// Adopt a legacy container into swapdock at the release it is running.
    Adopt {
        /// App name, as in the registry.
        app: String,
        /// Commit the legacy container is already running.
        #[arg(long, value_name = "SHA")]
        release: String,
        /// Healthcheck command to add when the compose file has none.
        #[arg(long, value_name = "CMD")]
        healthcheck: Option<String>,
        /// Container-side port; inferred from the legacy container when omitted.
        #[arg(long, value_name = "PORT")]
        container_port: Option<u16>,
        #[command(flatten)]
        paths: DeployPaths,
    },

    /// Deploy one app through its registry strategy.
    Up {
        /// App name, as in the registry.
        app: String,
        /// Commit to swapdock. Defaults to the recorded release.
        #[arg(long, value_name = "SHA")]
        release: Option<String>,
        /// Deploy even when the app is already on that release.
        #[arg(long)]
        force: bool,
        #[command(flatten)]
        paths: DeployPaths,
    },

    /// Roll back by deploying the previous release through the same strategy.
    Rollback {
        /// App name, as in the registry.
        app: String,
        #[command(flatten)]
        paths: DeployPaths,
    },

    /// Report what one app is running: recorded release, live port, health.
    Status {
        /// App name, as in the registry.
        app: String,
    },

    /// Print the registry entry, live verdict and container mounts as JSON.
    Inspect {
        /// App name, as in the registry.
        app: String,
    },

    /// Write a commented registry template for a new app. Changes nothing existing.
    Init {
        /// App name. Becomes `<name>.toml` in the directory.
        name: String,
        /// Where to write the file. Defaults to the current directory.
        #[arg(long, value_name = "DIR")]
        dir: Option<PathBuf>,
        /// Kind of app: container or static.
        #[arg(long, default_value = "container", value_name = "KIND")]
        kind: String,
    },

    /// Import an existing Compose service as a registry entry.
    ///
    /// Reads the merged model with `docker compose config`, allocates a front
    /// port and slot from the existing registry, and writes `<name>.toml` plus a
    /// generated override. It never edits the compose file, starts a container
    /// or deploys anything.
    ///
    /// `--dry-run` still runs the read-only `docker compose config` probe (it is
    /// what the plan is derived from), but writes no registry file, override or
    /// run record.
    Import {
        #[command(flatten)]
        args: ImportArgs,
    },

    /// Fetch the app source and fast-forward it. Prints the new HEAD.
    Sync {
        /// App name, as in the registry.
        app: String,
    },

    /// Build a release on the configured build host. Prints the image ref.
    Build {
        /// App name, as in the registry.
        app: String,
        /// Commit to build. Defaults to the recorded release.
        #[arg(long, value_name = "SHA")]
        release: Option<String>,
        /// Build host. Defaults to SWAPDOCK_BUILD_HOST, then the registry.
        #[arg(long, value_name = "HOST")]
        build_host: Option<String>,
    },

    /// Run the app's configured `[backup]` command once.
    Backup {
        /// App name, as in the registry.
        app: Option<String>,
        /// Back up every app that has a `[backup]` command.
        #[arg(long)]
        all: bool,
    },

    /// Restore one snapshot with the app's `[backup]` restore_command.
    Restore {
        /// App name, as in the registry.
        app: String,
        /// Snapshot file to restore.
        #[arg(long, value_name = "PATH")]
        file: PathBuf,
    },

    /// Reconcile a crash between the nginx flip and the registry commit.
    ///
    /// Proves the host is serving the pending release and its health path, then
    /// advances the registry. Refuses and leaves the pending record if nginx is
    /// not on that release or the release is unhealthy.
    Reconcile {
        /// App name, as in the registry.
        app: String,
        #[command(flatten)]
        paths: DeployPaths,
    },

    /// Run the privileged apply broker. Internal; started by the broker unit.
    #[command(hide = true)]
    Broker {
        /// Unix socket to listen on.
        #[arg(long, default_value = "/run/swapdock/broker.sock", value_name = "PATH")]
        socket: PathBuf,
        /// Docker binary the broker executes (tests use a fake).
        #[arg(long, default_value = "docker", value_name = "PATH")]
        docker_bin: PathBuf,
        #[command(flatten)]
        paths: DeployPaths,
    },

    /// Check live app ports, disk pressure and certificate expiry.
    ///
    /// Read-only. Sends de-duplicated alerts through SWAPDOCK_WEBHOOK_URL when
    /// run by the optional monitor timer.
    Monitor {
        /// Print the report as JSON.
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        paths: DeployPaths,
    },

    /// Probe an app's public URL over HTTPS (or --url).
    Probe {
        /// App name, as in the registry.
        app: String,
        /// URL to probe. Defaults to the app's public_url.
        #[arg(long, value_name = "URL")]
        url: Option<String>,
    },

    /// Check the access log since a time or run: failures, upstreams, the flip.
    Verify {
        /// App name, as in the registry.
        app: String,
        /// Start of the window: RFC 3339 timestamp or a run id.
        #[arg(long, value_name = "SINCE")]
        since: String,
        /// Access log to read.
        #[arg(long, default_value = crate::verify::DEFAULT_ACCESS_LOG, value_name = "PATH")]
        access_log: PathBuf,
    },

    /// Render the config, refuse on any error, then atomically apply and reload.
    Apply {
        #[command(flatten)]
        paths: DeployPaths,
    },

    /// Remove old local release images while keeping rollback targets.
    Prune {
        /// Prune only this app. Defaults to every app in the registry.
        #[arg(long, value_name = "NAME")]
        app: Option<String>,
        /// Recent rollback-pre-* tags to keep per app.
        #[arg(long, default_value_t = 3)]
        keep: usize,
    },

    /// Install the host integration from a packaged dist directory.
    ///
    /// This is a thin root-only wrapper around the reviewed installer script;
    /// it does not reimplement the install.
    Setup {
        /// Print the installer plan without changing the host.
        #[arg(long)]
        dry_run: bool,
        /// Dist tree containing install-swapdock.sh. Defaults to $SWAPDOCK_DIST
        /// or /usr/local/lib/swapdock/dist.
        #[arg(long, value_name = "DIR")]
        dist: Option<PathBuf>,
    },

    /// Serve the HTTP API instead of one-shot CLI commands.
    ///
    /// Requires `SWAPDOCK_API_TOKEN` (at least 16 characters) unless
    /// `--allow-unauthenticated` is passed for local development. Bind loopback
    /// and terminate TLS at a reverse proxy; a query token is accepted only for
    /// SSE clients that cannot set headers.
    Serve {
        /// Address to bind, e.g. 127.0.0.1:8088.
        #[arg(long, default_value = "127.0.0.1:8088", value_name = "ADDR")]
        bind: String,
        /// Development only: run without a token. Never use on a shared host.
        #[arg(long)]
        allow_unauthenticated: bool,
        /// PEM certificate chain for HTTPS (requires --tls-key).
        #[arg(long, value_name = "PATH", requires = "tls_key")]
        tls_cert: Option<PathBuf>,
        /// PEM private key for HTTPS (requires --tls-cert).
        #[arg(long, value_name = "PATH", requires = "tls_cert")]
        tls_key: Option<PathBuf>,
        #[command(flatten)]
        paths: DeployPaths,
    },
}

impl Cli {
    /// Dispatches and maps the result to a process exit code.
    pub fn dispatch(self) -> ExitCode {
        match self.command.run(&self) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                let mut source = e.source();
                while let Some(cause) = source {
                    eprintln!("  caused by: {cause}");
                    source = cause.source();
                }
                ExitCode::FAILURE
            }
        }
    }
}

impl Command {
    fn run(&self, cli: &Cli) -> anyhow::Result<()> {
        match self {
            Command::Selftest => selftest(cli),
            Command::Runs { limit } => list_runs(cli, *limit),
            Command::Show { run_id } => show_run(cli, run_id),
            Command::Resume { run_id } => resume_run(cli, run_id),
            Command::Render { app } => render_config(cli, app.as_deref()),
            Command::Validate => validate_registry(cli),
            Command::Doctor { app, public } => doctor_app(cli, app.as_deref(), *public),
            Command::Apply { paths } => apply_config(
                cli,
                &paths.target,
                &paths.main_config,
                &paths.pid_file,
                &paths.nginx_bin,
                &paths.lock_dir,
            ),
            Command::Adopt {
                app,
                release,
                healthcheck,
                container_port,
                paths,
            } => adopt_app(
                cli,
                app,
                release,
                healthcheck.as_deref(),
                *container_port,
                paths,
            ),
            Command::Up {
                app,
                release,
                force,
                paths,
            } => up_app(cli, app, release.as_deref(), *force, paths),
            Command::Rollback { app, paths } => rollback_app(cli, app, paths),
            Command::Status { app } => status_app(cli, app),
            Command::Inspect { app } => inspect_app(cli, app),
            Command::Sync { app } => sync_app(cli, app),
            Command::Init { name, dir, kind } => init_app(name, dir.clone(), kind),
            Command::Import { args } => import_app(cli, args),
            Command::Build {
                app,
                release,
                build_host,
            } => build_app(cli, app, release.as_deref(), build_host.as_deref()),
            Command::Backup { app, all } => backup_app(cli, app.as_deref(), *all),
            Command::Restore { app, file } => restore_app(cli, app, file),
            Command::Reconcile { app, paths } => reconcile_app(cli, app, paths),
            Command::Broker {
                socket,
                docker_bin,
                paths,
            } => broker_app(cli, socket, docker_bin, paths),
            Command::Monitor { json, paths } => monitor_app(cli, *json, paths),
            Command::Probe { app, url } => probe_app(cli, app, url.as_deref()),
            Command::Verify {
                app,
                since,
                access_log,
            } => verify_app(cli, app, since, access_log),
            Command::Prune { app, keep } => prune_app(cli, app.as_deref(), *keep),
            Command::Setup { dry_run, dist } => setup_app(*dry_run, dist.as_deref()),
            Command::Serve {
                bind,
                allow_unauthenticated,
                tls_cert,
                tls_key,
                paths,
            } => serve(
                cli,
                bind,
                paths,
                *allow_unauthenticated,
                tls_cert.as_deref(),
                tls_key.as_deref(),
            ),
        }
    }
}

/// `swapdock serve` — the same operations over HTTP. Everything else about the
/// CLI (paths, dry-run, the run log) already lives on `Cli`, so the server
/// just forwards it.
fn setup_app(dry_run: bool, dist: Option<&Path>) -> anyhow::Result<()> {
    if !is_root() {
        anyhow::bail!("swapdock setup must run as root (use sudo)");
    }
    let dist = dist
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("SWAPDOCK_DIST").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/usr/local/lib/swapdock/dist"));
    let installer = dist.join("install-swapdock.sh");
    if !installer.is_file() {
        anyhow::bail!(
            "installer not found at {}; pass --dist or set SWAPDOCK_DIST",
            installer.display()
        );
    }
    let mut args = vec![installer.display().to_string()];
    if dry_run {
        args.push("--dry-run".into());
    }
    args.push(dist.display().to_string());
    let mut spec = crate::exec::StepSpec::new("setup", "bash")
        .args(args)
        .timeout(std::time::Duration::from_secs(600));
    // The installer uses SUDO_USER to pick the deploy user; exec clears the
    // environment, so forward this one explicitly.
    if let Ok(user) = std::env::var("SUDO_USER") {
        spec = spec.env("SUDO_USER", user);
    }
    let outcome = crate::exec::run_capture(&spec)?;
    print!("{}", outcome.stdout);
    eprint!("{}", outcome.stderr);
    if !outcome.success() {
        anyhow::bail!("setup failed with exit code {:?}", outcome.exit_code);
    }
    Ok(())
}

fn is_root() -> bool {
    // `/proc/self` is owned by the process user, not root; read the effective
    // uid from /proc/self/status instead.
    let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
        return false;
    };
    status.lines().find_map(|line| {
        let rest = line.strip_prefix("Uid:")?;
        rest.split_whitespace().nth(1)?.parse::<u32>().ok()
    }) == Some(0)
}

fn serve(
    cli: &Cli,
    bind: &str,
    paths: &DeployPaths,
    allow_unauthenticated: bool,
    tls_cert: Option<&Path>,
    tls_key: Option<&Path>,
) -> anyhow::Result<()> {
    let tls = match (tls_cert, tls_key) {
        (Some(cert), Some(key)) => {
            for (kind, path) in [("certificate", cert), ("private key", key)] {
                if !path.is_file() {
                    anyhow::bail!("TLS {kind} {} does not exist", path.display());
                }
            }
            Some(crate::api::TlsConfig {
                cert: cert.to_path_buf(),
                key: key.to_path_buf(),
            })
        }
        (None, None) => None,
        _ => anyhow::bail!("--tls-cert and --tls-key must be supplied together"),
    };
    let auth = crate::auth::AuthConfig::try_from_env(allow_unauthenticated)
        .map_err(|message| anyhow::anyhow!(message))?;
    if auth.allows_unauthenticated() && !crate::auth::AuthConfig::loopback_bind(bind) {
        anyhow::bail!(
            "--allow-unauthenticated is a loopback-only development escape hatch; \
             {bind:?} is not a loopback literal. Set SWAPDOCK_API_TOKEN instead."
        );
    }
    if auth.allows_unauthenticated() {
        tracing::warn!(
            "API authentication is disabled (--allow-unauthenticated); \
             bind loopback only and never use this on a shared host"
        );
    }
    // A process that died between the nginx flip and the registry commit left a
    // pending journal. Reconcile it before opening the API, so the first request
    // cannot see a registry that disagrees with the host.
    let startup_ctx = crate::deploy::DeployContext {
        registry_dir: cli.registry.clone(),
        state_dir: paths.state_dir.clone(),
        lock_dir: paths.lock_dir.clone(),
        apply_paths: crate::apply::ApplyPaths {
            nginx_bin: paths.nginx_bin.clone(),
            main_config: paths.main_config.clone(),
            target: paths.target.clone(),
            pid_file: Some(paths.pid_file.clone()),
        },
        drain_secs: paths.drain_secs,
        lock_timeout: crate::lock::ACQUIRE_TIMEOUT,
    };
    for outcome in crate::reconcile::reconcile_pending(&startup_ctx, &cli.trace) {
        match outcome.result {
            Ok(result) => eprintln!("startup reconcile {}: {result:?}", outcome.app),
            Err(e) => eprintln!(
                "startup reconcile {} refused (journal kept): {e}",
                outcome.app
            ),
        }
    }

    // The async runtime exists only for the HTTP listener; the handlers block
    // on their own threads, so nothing inside the runtime needs to be Send.
    let rt = tokio::runtime::Runtime::new()?;
    let state = crate::api::ServerState {
        registry: cli.registry.clone(),
        trace: cli.trace.clone(),
        tunnel_config: cli.tunnel_config.clone(),
        target: paths.target.clone(),
        main_config: paths.main_config.clone(),
        pid_file: paths.pid_file.clone(),
        nginx_bin: paths.nginx_bin.clone(),
        state_dir: paths.state_dir.clone(),
        lock_dir: paths.lock_dir.clone(),
        drain_secs: paths.drain_secs,
        transfer_dir: std::env::temp_dir().join("swapdock-transfer"),
        auth,
        rate: std::sync::Arc::new(crate::api::RateLimiter::new(240, 60)),
    };
    rt.block_on(crate::api::serve(state, bind, tls))
}

fn mode(cli: &Cli) -> RunMode {
    if cli.dry_run {
        RunMode::DryRun
    } else {
        RunMode::Live
    }
}

/// Opens the log and starts a run whose argv is recorded verbatim.
fn start(cli: &Cli) -> anyhow::Result<Run> {
    let log = TraceLog::open(&cli.trace)?;
    let argv: Vec<String> = std::env::args().collect();
    Run::start(log, mode(cli), cli.app.clone(), &argv, Redactor::new())
}

/// `swapdock selftest`
///
/// Exercises every behaviour the chokepoint promises: success, non-zero exit,
/// stderr capture, argument pass-through without a shell, timeout enforcement,
/// large output, a missing program, and secret redaction.
fn selftest(cli: &Cli) -> anyhow::Result<()> {
    let mut run = start(cli)?;
    let dry = mode(cli) == RunMode::DryRun;
    let mut rows: Vec<Vec<String>> = Vec::new();
    let redactor = run.redactor().clone();

    let cases: Vec<(&str, StepSpec, &str)> = vec![
        (
            "success",
            StepSpec::new("selftest-success", "sh").args(["-c", "echo hello"]),
            "exit 0, stdout captured",
        ),
        (
            "nonzero-exit",
            StepSpec::new("selftest-nonzero", "sh").args(["-c", "exit 3"]),
            "a non-zero exit is a result, not a tool error",
        ),
        (
            "stderr",
            StepSpec::new("selftest-stderr", "sh").args(["-c", "echo oops 1>&2"]),
            "stderr kept separate from stdout",
        ),
        (
            "no-shell",
            StepSpec::new("selftest-noshell", "printf").args(["%s|", "a b", "$HOME", "*"]),
            "no shell means no globbing or expansion",
        ),
        (
            "large-output",
            StepSpec::new("selftest-large", "sh")
                .args(["-c", "seq 1 200000"])
                .timeout(Duration::from_secs(20)),
            "drains both pipes, keeps the tail",
        ),
        (
            "timeout",
            StepSpec::new("selftest-timeout", "sh")
                .args(["-c", "sleep 30"])
                .timeout(Duration::from_millis(200)),
            "killed at 200 ms instead of hanging",
        ),
        (
            "missing-program",
            StepSpec::new("selftest-missing", "definitely-not-a-real-program-xyz"),
            "spawn failure is reported, not panicked",
        ),
        (
            "redaction",
            StepSpec::new("selftest-redact", "sh")
                .args(["-c", "echo using ghp_abcdefghijklmnop1234 to authenticate"])
                .env("GITHUB_TOKEN", "ghp_abcdefghijklmnop1234"),
            "token must not appear in the log",
        ),
    ];

    for (name, spec, proves) in cases {
        let (status, observed) = match run.exec(&spec) {
            Ok(outcome) => {
                if dry {
                    ("ok", "would run (dry run)".to_string())
                } else {
                    let code = outcome
                        .exit_code
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "none".into());
                    let mut parts = vec![format!("exit={code}")];
                    // Redact before display: a token in a terminal scrollback
                    // is a leak even when the log is clean.
                    let so = first_line(&outcome.stdout_safe(&redactor));
                    let se = first_line(&outcome.stderr_safe(&redactor));
                    if !so.is_empty() {
                        parts.push(format!("stdout={so}"));
                    }
                    if !se.is_empty() {
                        parts.push(format!("stderr={se}"));
                    }
                    ("ok", parts.join(" "))
                }
            }
            Err(ExecError::Timeout { timeout_ms, .. }) => {
                ("timeout", format!("killed at {timeout_ms} ms"))
            }
            Err(e) => ("error", first_line(&e.to_string())),
        };
        rows.push(vec![name.into(), status.into(), observed, proves.into()]);
    }

    let failed = rows.iter().any(|r| r[1] == "error" || r[1] == "timeout");
    // Capture the id before `finish` consumes the run.
    let run_id = run.id().to_string();
    run.finish(if dry {
        RunStatus::DryRun
    } else if failed {
        RunStatus::Failed
    } else {
        RunStatus::Ok
    })?;

    print_table(&["case", "status", "observed", "what it proves"], &rows);

    let raw = std::fs::read_to_string(&cli.trace)?;
    let leaked = raw.contains("ghp_abcdefghijklmnop1234");
    let redacted = raw.matches("[REDACTED").count();
    let lines = raw.lines().count();

    println!();
    println!("run id       {run_id}");
    println!("trace        {}", cli.trace.display());
    println!("log lines    {lines}");
    println!("redactions   {redacted}");
    println!("secret leak  {}", if leaked { "YES - BUG" } else { "no" });
    println!();
    println!("inspect it with:");
    println!("  swapdock --trace {} show {run_id}", cli.trace.display());
    println!(
        "  grep -o '\"step\":\"[a-z-]*\"' {} | sort | uniq -c",
        cli.trace.display()
    );

    if leaked {
        anyhow::bail!("a secret reached the log");
    }
    Ok(())
}

/// One row of `swapdock runs`.
struct RunSummary {
    run_id: String,
    started: String,
    status: String,
    steps: u32,
    /// Steps that did not succeed. A non-zero exit is a fact about one step, not
    /// proof that the run stopped there, so it is counted rather than used to
    /// name a stopping point.
    non_ok: u32,
    pid: Option<u32>,
    boot: Option<String>,
    ended: bool,
}

/// `swapdock runs`
fn list_runs(cli: &Cli, limit: usize) -> anyhow::Result<()> {
    let read = TraceLog::read(&cli.trace)?;

    if read.damaged > 0 {
        eprintln!(
            "note: {} line(s) in {} did not parse (torn write?)",
            read.damaged,
            cli.trace.display()
        );
    }

    let mut summaries: Vec<RunSummary> = Vec::new();

    for ev in &read.events {
        match ev {
            TraceEvent::RunStart {
                run_id,
                ts,
                mode,
                pid,
                boot,
                ..
            } => {
                summaries.push(RunSummary {
                    run_id: run_id.clone(),
                    started: ts.clone(),
                    status: format!("{mode:?}"),
                    steps: 0,
                    non_ok: 0,
                    pid: *pid,
                    boot: boot.clone(),
                    ended: false,
                });
            }
            TraceEvent::Step { run_id, status, .. } => {
                // Attribute to the run that is currently open.
                if let Some(current) = summaries.iter_mut().rev().find(|s| &s.run_id == run_id) {
                    current.steps += 1;
                    if status.is_failure() {
                        current.non_ok += 1;
                    }
                }
            }
            TraceEvent::RunEnd { run_id, status, .. } => {
                if let Some(current) = summaries.iter_mut().rev().find(|s| &s.run_id == run_id) {
                    current.status = format!("{status:?}");
                    current.ended = true;
                }
            }
        }
    }

    summaries.reverse();
    summaries.truncate(limit);

    if summaries.is_empty() {
        println!("no runs in {}", cli.trace.display());
        return Ok(());
    }

    let rows: Vec<Vec<String>> = summaries
        .iter()
        .map(|s| {
            vec![
                s.run_id.clone(),
                s.started.clone(),
                if s.ended {
                    s.status.clone()
                } else {
                    format!(
                        "{:?}",
                        crate::trace::run_state(s.pid, s.boot.as_deref(), None)
                    )
                },
                s.steps.to_string(),
                if s.non_ok == 0 {
                    "-".to_string()
                } else {
                    s.non_ok.to_string()
                },
            ]
        })
        .collect();

    print_table(&["run id", "started", "status", "steps", "non-ok"], &rows);
    Ok(())
}

/// `swapdock show <run-id>`
fn show_run(cli: &Cli, run_id: &str) -> anyhow::Result<()> {
    let run_id = RunId::parse(run_id).ok_or_else(|| {
        anyhow::anyhow!("not a valid run id: {run_id:?} (expected 27 lowercase hex characters)")
    })?;

    let read = TraceLog::read(&cli.trace)?;
    let mut rows: Vec<Vec<String>> = Vec::new();

    for ev in &read.events {
        if let TraceEvent::Step {
            run_id: id,
            seq,
            step,
            status,
            argv,
            exit_code,
            duration_ms,
            ..
        } = ev
            && id == run_id.as_str()
        {
            // Filesystem steps have no exit code and no duration; showing
            // `exit=- -` for them reads as missing data rather than not applicable.
            let result = match (exit_code, duration_ms) {
                (Some(code), Some(ms)) => format!("{status:?} exit={code} {ms}ms"),
                (Some(code), None) => format!("{status:?} exit={code}"),
                (None, _) => format!("{status:?}"),
            };
            rows.push(vec![format!("{seq}"), step.clone(), result, argv.join(" ")]);
        }
    }

    if rows.is_empty() {
        anyhow::bail!("no steps for run {run_id} in {}", cli.trace.display());
    }

    print_table(&["seq", "step", "result", "argv"], &rows);
    Ok(())
}

/// `swapdock resume <run-id>`
fn resume_run(cli: &Cli, run_id: &str) -> anyhow::Result<()> {
    let run_id =
        RunId::parse(run_id).ok_or_else(|| anyhow::anyhow!("not a valid run id: {run_id:?}"))?;

    match TraceLog::resume_point(&cli.trace, &run_id)? {
        Some(point) => {
            println!("run            {run_id}");
            println!(
                "last step      {} (seq {})",
                point.last_step, point.last_seq
            );
            println!("last status    {:?}", point.last_status);
            println!("non-ok steps   {}", point.non_ok_steps);
            println!("state          {:?}", point.state());
            println!(
                "ended          {}",
                match point.ended {
                    Some(status) => format!("{status:?}"),
                    None => "no final record".to_string(),
                }
            );
            println!();
            match (point.ended, point.last_status.is_failure()) {
                (Some(status), _) => {
                    println!("the run finished with status {status:?}; there is nothing to resume");
                }
                (None, true) => {
                    println!(
                        "the run stopped on a failed step; re-run that step before continuing"
                    );
                }
                (None, false) if matches!(point.state(), RunStatus::Interrupted) => {
                    println!(
                        "the process died before writing a final record; inspect the pending journal and run `swapdock reconcile`"
                    );
                }
                (None, false) => {
                    println!("the run is still in flight; wait for it to finish");
                }
            }
            println!("no step catalogue exists yet, so nothing resumes automatically");
            Ok(())
        }
        None => {
            println!("no steps recorded for {run_id} in {}", cli.trace.display());
            Ok(())
        }
    }
}

/// Loads the registry and reports anything unreadable, or `None` on failure.
fn load(cli: &Cli) -> anyhow::Result<Option<Loaded>> {
    let loaded = crate::registry::load_dir(&cli.registry)?;
    if loaded.apps.is_empty() && loaded.problems.is_empty() {
        eprintln!(
            "no registry files in {}; nothing to do",
            cli.registry.display()
        );
        return Ok(None);
    }
    Ok(Some(loaded))
}

/// `swapdock render`
///
/// Pure: reads the registry, writes stdout. Nothing is applied, so this is safe
/// to run at any time and against any registry.
fn render_config(cli: &Cli, only: Option<&str>) -> anyhow::Result<()> {
    let Some(loaded) = load(cli)? else {
        return Ok(());
    };

    // Load-time problems still belong on stderr: the config on stdout must stay
    // usable in a pipeline.
    for problem in &loaded.problems {
        eprintln!("{problem}");
    }

    let apps = loaded.sorted();
    let selected: Vec<App> = match only {
        None => apps,
        Some(name) => {
            let found: Vec<App> = apps.into_iter().filter(|a| a.name == name).collect();
            if found.is_empty() {
                let mut known: Vec<&str> = loaded.apps.iter().map(|a| a.name.as_str()).collect();
                known.sort_unstable();
                anyhow::bail!(
                    "no app named {name:?} in {}; known apps: {}",
                    cli.registry.display(),
                    known.join(", ")
                );
            }
            found
        }
    };

    print!("{}", render::render(&selected));

    let errors = selected
        .iter()
        .flat_map(|a| a.problems())
        .filter(|p| p.is_error())
        .count();
    if errors > 0 {
        eprintln!(
            "note: {errors} problem(s) in the registry; the config above must not be applied"
        );
    }
    Ok(())
}

/// `swapdock validate`
///
/// Reads only. Exits non-zero when anything is an error, so it can gate a swapdock.
fn validate_registry(cli: &Cli) -> anyhow::Result<()> {
    let Some(loaded) = load(cli)? else {
        return Ok(());
    };

    let apps = loaded.sorted();
    let (problems, routes) = collect_problems(cli, &loaded, &apps);

    let (errors, warnings) = validator::counts(&problems);

    println!("registry  {}", cli.registry.display());
    println!("apps      {}", apps.len());
    println!(
        "tunnel    {}",
        match &cli.tunnel_config {
            Some(p) => format!("{} ({} routes)", p.display(), routes.len()),
            None => "not checked".to_string(),
        }
    );
    println!();

    if problems.is_empty() {
        println!("no problems");
    } else {
        let rows: Vec<Vec<String>> = problems
            .iter()
            .map(|p| {
                vec![
                    p.severity.to_string(),
                    p.code.to_string(),
                    p.app.clone().unwrap_or_else(|| "-".to_string()),
                    p.message.clone(),
                ]
            })
            .collect();
        print_table(&["severity", "code", "app", "message"], &rows);
    }

    println!();
    println!("{errors} error(s), {warnings} warning(s)");

    if errors > 0 {
        std::process::exit(1);
    }
    Ok(())
}

/// `swapdock doctor`
///
/// Unlike `validate`, this reads the compose model and the Docker daemon. It is
/// still read-only: no container is started, stopped or tagged.
fn doctor_app(cli: &Cli, only: Option<&str>, public: bool) -> anyhow::Result<()> {
    let Some(loaded) = load(cli)? else {
        return Ok(());
    };
    let apps = loaded.sorted();
    let selected: Vec<App> = match only {
        None => apps,
        Some(name) => {
            let found: Vec<App> = apps.into_iter().filter(|a| a.name == name).collect();
            if found.is_empty() {
                let mut known: Vec<&str> = loaded.apps.iter().map(|a| a.name.as_str()).collect();
                known.sort_unstable();
                anyhow::bail!(
                    "no app named {name:?} in {}; known apps: {}",
                    cli.registry.display(),
                    known.join(", ")
                );
            }
            found
        }
    };

    let (mut problems, routes) = collect_problems(cli, &loaded, &selected);
    let mut run = start(cli)?;
    problems.extend(crate::doctor::check(&mut run, &selected, only, public));
    validator::sort_problems(&mut problems);

    let (errors, warnings) = validator::counts(&problems);
    println!("registry  {}", cli.registry.display());
    println!("apps      {}", selected.len());
    println!(
        "tunnel    {}",
        match &cli.tunnel_config {
            Some(p) => format!("{} ({} routes)", p.display(), routes.len()),
            None => "not checked".to_string(),
        }
    );
    println!(
        "public    {}",
        if public { "checked" } else { "not checked" }
    );
    println!();

    if problems.is_empty() {
        println!("no problems");
    } else {
        let rows: Vec<Vec<String>> = problems
            .iter()
            .map(|p| {
                vec![
                    p.severity.to_string(),
                    p.code.to_string(),
                    p.app.clone().unwrap_or_else(|| "-".to_string()),
                    p.message.clone(),
                ]
            })
            .collect();
        print_table(&["severity", "code", "app", "message"], &rows);
    }

    println!();
    println!("{errors} error(s), {warnings} warning(s)");
    run.finish(if cli.dry_run {
        RunStatus::DryRun
    } else if errors > 0 {
        RunStatus::Failed
    } else {
        RunStatus::Ok
    })?;

    if errors > 0 {
        std::process::exit(1);
    }
    Ok(())
}

/// Flattens a cell to one line.
///
/// A multi-line cell — a TOML parse error carries its own source excerpt — would
/// otherwise break every column after it and make the table unreadable. The
/// content is kept; only the line breaks go.
fn flatten_cell(cell: &str) -> String {
    let mut out = String::with_capacity(cell.len());
    let mut last_was_space = false;
    for ch in cell.chars() {
        if ch.is_whitespace() {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
        } else {
            out.push(ch);
            last_was_space = false;
        }
    }
    // Leading and trailing whitespace is never meaningful in a cell, and a
    // trailing space would inflate the column width.
    out.trim().to_string()
}

/// Shared by `validate` and `apply`: every problem in the registry, the apps
/// they concern, and the tunnel routes when a config was supplied.
///
/// Returns the apps, so callers do not load twice.
fn collect_problems(
    cli: &Cli,
    loaded: &Loaded,
    apps: &[App],
) -> (Vec<Problem>, validator::TunnelRoutes) {
    let mut problems: Vec<Problem> = loaded.problems.clone();
    for app in apps {
        problems.extend(app.problems());
    }

    let mut routes = TunnelRoutes::new();
    match &cli.tunnel_config {
        None => problems.push(Problem::warning(
            "tunnel-not-checked",
            None,
            format!(
                "no --tunnel-config given, so {} route cross-checks were skipped",
                apps.len()
            ),
        )),
        Some(path) => match tunnel::read_config(path) {
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

    problems.extend(validator::validate(apps, &routes));
    validator::sort_problems(&mut problems);
    (problems, routes)
}

/// `swapdock apply`
///
/// Render, refuse on any error, then stage, test, commit and reload. The only
/// command that changes the host.
fn apply_config(
    cli: &Cli,
    target: &Path,
    main_config: &Path,
    pid_file: &Path,
    nginx_bin: &Path,
    lock_dir: &Path,
) -> anyhow::Result<()> {
    // Serialize with deploys before loading/rendering: a deploy can commit a
    // new live_port while we wait, and a config rendered from a pre-commit
    // snapshot would clobber it.
    let _estate = crate::lock::Lock::acquire(
        lock_dir,
        crate::lock::estate_lock_name(),
        crate::lock::ACQUIRE_TIMEOUT,
    )?;
    let Some(loaded) = load(cli)? else {
        return Ok(());
    };
    let apps = loaded.sorted();
    let (problems, _) = collect_problems(cli, &loaded, &apps);
    let (errors, _) = validator::counts(&problems);

    if errors > 0 {
        for problem in &problems {
            eprintln!("{problem}");
        }
        anyhow::bail!("{errors} error(s) in the registry; nothing was applied");
    }

    let text = render::render(&apps);
    let paths = ApplyPaths {
        nginx_bin: nginx_bin.to_path_buf(),
        main_config: main_config.to_path_buf(),
        target: target.to_path_buf(),
        pid_file: Some(pid_file.to_path_buf()),
    };

    let mut run = start(cli)?;
    // Capture the id before `finish` consumes the run.
    let run_id = run.id().to_string();
    match apply::apply(&mut run, &text, &paths) {
        Ok(outcome) => {
            run.finish(RunStatus::Ok)?;
            if cli.dry_run {
                println!("would apply  {}", target.display());
                println!("would reload");
            } else {
                println!("applied  {}", target.display());
                println!("reloaded {}", outcome.reloaded);
            }
            match (outcome.master_pid_before, outcome.master_pid_after) {
                (Some(b), Some(a)) => println!("master   before={b} after={a} (unchanged)"),
                _ => println!("master   unknown (no pid file to compare)"),
            }
            match &outcome.backup {
                Some(b) => println!("backup   {}", b.display()),
                None => println!("backup   none (first apply)"),
            }
            println!("run      {run_id}");
            Ok(())
        }
        Err(e) => {
            // The run log already holds every step; finish it as failed so the
            // failure is attributable, then report.
            let _ = run.finish(RunStatus::Failed);
            Err(e.into())
        }
    }
}

/// Builds the strategy context from CLI flags.
fn ctx_from(cli: &Cli, paths: &DeployPaths) -> DeployContext {
    DeployContext {
        registry_dir: cli.registry.clone(),
        state_dir: paths.state_dir.clone(),
        lock_dir: paths.lock_dir.clone(),
        apply_paths: ApplyPaths {
            nginx_bin: paths.nginx_bin.clone(),
            main_config: paths.main_config.clone(),
            target: paths.target.clone(),
            pid_file: Some(paths.pid_file.clone()),
        },
        drain_secs: paths.drain_secs,
        lock_timeout: crate::lock::ACQUIRE_TIMEOUT,
    }
}

/// Loads one app by name, or lists what exists.
fn load_app(cli: &Cli, name: &str) -> anyhow::Result<crate::registry::App> {
    let loaded = crate::registry::load_dir(&cli.registry)?;
    let mut apps = loaded.sorted();
    match apps.iter_mut().find(|a| a.name == name) {
        Some(app) => Ok(app.clone()),
        None => {
            let mut known: Vec<&str> = loaded.apps.iter().map(|a| a.name.as_str()).collect();
            known.sort_unstable();
            anyhow::bail!(
                "no app named {name:?} in {}; known apps: {}",
                cli.registry.display(),
                known.join(", ")
            );
        }
    }
}

/// `swapdock adopt <app> --release <current>`
fn adopt_app(
    cli: &Cli,
    name: &str,
    release: &str,
    healthcheck: Option<&str>,
    container_port: Option<u16>,
    paths: &DeployPaths,
) -> anyhow::Result<()> {
    let mut app = load_app(cli, name)?;
    let ctx = ctx_from(cli, paths);
    let mut run = start(cli)?;
    let run_id = run.id().to_string();
    println!("adopting {name} at release {release}");
    match deploy::adopt(
        &mut run,
        &ctx,
        &mut app,
        release,
        healthcheck,
        container_port,
    ) {
        Ok(()) => {
            run.finish(if cli.dry_run {
                RunStatus::DryRun
            } else {
                RunStatus::Ok
            })?;
            if cli.dry_run {
                println!("would adopt {name} at release {release}");
            } else {
                println!("adopted {name} at release {release}");
            }
            println!("run      {run_id}");
            Ok(())
        }
        Err(e) => {
            let _ = run.finish(RunStatus::Failed);
            Err(anyhow::anyhow!("{e}"))
        }
    }
}

/// `swapdock up <app> [--release] [--force]`
fn up_app(
    cli: &Cli,
    name: &str,
    release: Option<&str>,
    force: bool,
    paths: &DeployPaths,
) -> anyhow::Result<()> {
    let mut app = load_app(cli, name)?;
    let release =
        deploy::resolve_release(&app, release, force).map_err(|e| anyhow::anyhow!("{e}"))?;
    let ctx = ctx_from(cli, paths);

    let mut run = start(cli)?;
    let run_id = run.id().to_string();
    println!(
        "deploying {} release {release} via {}",
        app.name,
        app.strategy.as_str()
    );
    match deploy::deploy(&mut run, &ctx, &mut app, &release) {
        Ok(()) => {
            run.finish(RunStatus::Ok)?;
            println!("deployed {name} release {release}");
            println!("run      {run_id}");
            Ok(())
        }
        Err(e) => {
            let _ = run.finish(RunStatus::Failed);
            Err(anyhow::anyhow!("{e}"))
        }
    }
}

/// `swapdock rollback <app>`
fn rollback_app(cli: &Cli, name: &str, paths: &DeployPaths) -> anyhow::Result<()> {
    let mut app = load_app(cli, name)?;
    let ctx = ctx_from(cli, paths);

    let mut run = start(cli)?;
    let run_id = run.id().to_string();
    match deploy::rollback(&mut run, &ctx, &mut app) {
        Ok(old) => {
            run.finish(RunStatus::Ok)?;
            println!("rolled back {name} to release {old}");
            println!("run      {run_id}");
            Ok(())
        }
        Err(e) => {
            let _ = run.finish(RunStatus::Failed);
            Err(anyhow::anyhow!("{e}"))
        }
    }
}

/// `swapdock status <app>`
///
/// Read-only and spawn-free: it probes the live port and prints what the
/// registry recorded. No run is opened, so asking never pollutes the log.
fn status_app(cli: &Cli, name: &str) -> anyhow::Result<()> {
    let app = load_app(cli, name)?;
    let status = deploy::live_status(&app).map_err(|e| anyhow::anyhow!("{e}"))?;
    let port = status
        .live_port
        .map(|p| p.to_string())
        .unwrap_or_else(|| "-".to_string());
    let verdict = match status.code {
        Some(code) if status.up => format!("up ({code})"),
        Some(code) => format!("not healthy ({code})"),
        None => "down (no answer)".to_string(),
    };
    println!("app          {}", app.name);
    println!("release      {}", status.release.as_deref().unwrap_or("-"));
    println!(
        "previous     {}",
        status.old_release.as_deref().unwrap_or("-")
    );
    println!("live port    {port}");
    println!("health       {verdict}");
    Ok(())
}

/// `swapdock inspect <app>`
///
/// JSON for scripts and for the "why is this app mounting the wrong data?"
/// question. `status` stays spawn-free; this command deliberately asks Docker.
fn inspect_app(cli: &Cli, name: &str) -> anyhow::Result<()> {
    let app = load_app(cli, name)?;
    let live = deploy::live_status(&app).map_err(|e| anyhow::anyhow!("{e}"))?;

    let mut value = serde_json::json!({
        "app": &app,
        "live": {
            "release": live.release,
            "old_release": live.old_release,
            "live_port": live.live_port,
            "up": live.up,
            "code": live.code,
        },
    });

    let port = app.live_port.unwrap_or(app.front_port);
    let mut run = start(cli)?;
    let run_id = run.id().to_string();
    let found = run.exec(&crate::docker::ps_publishing(port))?;
    let names: Vec<String> = found
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();

    if let Some(container) = names.first() {
        let mut details = serde_json::json!({"name": container});
        if let Ok(out) = run.exec(&crate::docker::inspect_image_ref(container))
            && out.success()
        {
            details["image"] = serde_json::json!(out.stdout_trimmed());
        }
        if let Ok(out) = run.exec(&crate::docker::inspect_health(container))
            && out.success()
        {
            details["health"] = serde_json::json!(out.stdout_trimmed());
        }
        if let Ok(out) = run.exec(&crate::docker::inspect_mounts(container))
            && out.success()
            && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(out.stdout_trimmed())
        {
            details["mounts"] = parsed;
        }
        value["container"] = details;
    }

    run.finish(RunStatus::Ok)?;
    println!("{}", serde_json::to_string_pretty(&value)?);
    eprintln!("run      {run_id}");
    Ok(())
}

/// The template `init` writes. Every field is explained where it is used, so a
/// first registry file teaches the format instead of just filling it.
pub fn init_template(name: &str, kind: &str) -> anyhow::Result<String> {
    if !crate::registry::is_valid_slug(name) {
        anyhow::bail!("{name:?} is not a valid app name: lowercase letters, digits and dashes");
    }
    let (kind_block, strategy_note) = match kind {
        "container" => (
            "kind = \"container\"\nstrategy = \"swap\"\n",
            "# strategy is \"swap\" for stateless apps, \"replace\" when the app\n# writes to a database file or other shared local state.",
        ),
        "static" => (
            "kind = \"static\"\nstrategy = \"swap\"\nroot = \"/srv/www/NAME/current\"\n\
             # spa = true serves a single-page app: extensionless paths fall back\n\
             # to index.html, so a deep link survives a refresh.\n",
            "# static apps always swap: the symlink rename is atomic.",
        ),
        other => anyhow::bail!("kind must be \"container\" or \"static\", not {other:?}"),
    };
    let kind_block = kind_block.replace("NAME", name);
    Ok(format!(
        r#"# Registry file for {name}. One file per app; the file name must match.
# Validate it any time with: swapdock validate --registry <this directory>
name = "{name}"
{kind_block}# Hostnames routed here. Empty until the app is onboarded.
hostnames = ["{name}.example.com"]
# Extra bind addresses beyond loopback, e.g. a private interface.
listen = []
# The front port: what the tunnel or DNS points at. Never changes.
front_port = 8001
# Slot owning this app's back-end port pair (9000 + 2*slot, +1). Unique per app.
slot = 0
# True when the app writes to a database file or other shared local state.
writes_state = false
# Image name; the release tag is appended (repo:release). Omit for static apps.
image_repo = "{name}"
# Where images live. "local" when they are built and kept on this host and
# nothing is pushed or pulled. Defaults to ghcr when omitted.
registry = "local"
# Where releases are built: "local" or an SSH destination.
build_host = "local"
# Commit currently running. Never "latest": a moving tag has no rollback target.
# release = ""
# Health endpoint path; host and port always come from the gated container.
health_url = "http://127.0.0.1/"
# Compose project layout and the env var carrying the back-end port.
compose_dir = "/srv/example/compose"
compose_svc = "{name}"
env_name = "{upper}_PORT"
# Source location for `swapdock sync`. Omit when the tool does not manage it.
# git_remote = "example-org/{name}"
branch = "main"
# repo = "/srv/example/{name}"
# Optional: public URL used by `swapdock probe`.
# public_url = "https://{name}.example.com/health"
# Optional pre-deploy snapshot, run before every `up` when before_deploy = true.
# [backup]
# command = ["sqlite3", "/srv/example/{name}/data/app.db", ".backup {{file}}"]
# dir = "/srv/swapdock/backups/{name}"
# before_deploy = true
{strategy_note}
"#,
        upper = name.to_uppercase().replace('-', "_")
    ))
}

/// `swapdock init <name>`
fn init_app(name: &str, dir: Option<PathBuf>, kind: &str) -> anyhow::Result<()> {
    let text = init_template(name, kind)?;
    let dir = dir.unwrap_or_else(|| PathBuf::from("."));
    let path = dir.join(format!("{name}.toml"));
    if path.exists() {
        anyhow::bail!("{path:?} already exists; refusing to overwrite");
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, &text)?;

    // Prove the template is valid by loading it back.
    let back: crate::registry::App = toml::from_str(&text)
        .map_err(|e| anyhow::anyhow!("generated template does not parse: {e}"))?;
    assert_eq!(back.name, name);
    println!("wrote {path:?}");
    println!(
        "next: set front_port, slot, hostnames, then swapdock validate --registry {}",
        dir.display()
    );
    Ok(())
}

/// `swapdock import`
///
/// Read-only compose probe plus two file writes. No run is opened: import must
/// work for an unprivileged user with writable `--registry`/`--state-dir`, and
/// `--dry-run` must write nothing at all, including a log.
fn import_app(cli: &Cli, args: &ImportArgs) -> anyhow::Result<()> {
    let compose_dir = match &args.compose_dir {
        Some(path) => path.clone(),
        None => std::env::current_dir()
            .map_err(|e| anyhow::anyhow!("could not read the working directory: {e}"))?,
    };
    let options = crate::import::Options {
        compose_dir,
        service: args.service.clone(),
        name: args.name.clone(),
        domain: args.domain.clone(),
        front_port: args.front_port,
        slot: args.slot,
        container_port: args.container_port,
        healthcheck: args.healthcheck.clone(),
        repo: args.repo.clone(),
        state_dir: args.state_dir.clone(),
        dry_run: cli.dry_run,
    };
    let report = crate::import::run(&options, &cli.registry, cli.tunnel_config.as_deref())
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    print_import_report(cli, args, &report);
    Ok(())
}

fn print_import_report(cli: &Cli, args: &ImportArgs, report: &crate::import::Report) {
    for warning in &report.warnings {
        eprintln!("warning: {warning}");
    }

    println!("app          {}", report.app.name);
    println!("compose      {}", report.compose_file.display());
    println!(
        "service      {} (project {})",
        report.service,
        report.app.compose_project()
    );
    println!(
        "image        {} (image_repo {})",
        report.image,
        report.app.image_repo.as_deref().unwrap_or("-")
    );
    println!(
        "container    {}/tcp (from {})",
        report.container_port, report.container_port_source
    );
    match &report.healthcheck {
        Some(command) => {
            let path = report
                .app
                .health_url
                .as_deref()
                .map(crate::health::probe_path)
                .unwrap_or_else(|| "/".to_string());
            println!(
                "health       {}: {} -> GET {path}",
                report.healthcheck_source.label(),
                first_line(command)
            );
        }
        None => println!("health       none (the deploy gate will refuse until one is added)"),
    }
    println!("strategy     {}", report.strategy_reason);
    let back = crate::ports::pair(report.app.slot)
        .map(|(a, b)| format!("{a}, {b}"))
        .unwrap_or_else(|_| "-".to_string());
    println!(
        "ports        front {} / slot {} (back {back})",
        report.app.front_port, report.app.slot
    );
    println!(
        "hostnames    {}",
        if report.app.hostnames.is_empty() {
            "none yet".to_string()
        } else {
            report.app.hostnames.join(", ")
        }
    );

    println!();
    if report.dry_run {
        println!("dry run      nothing written; the compose probe was read-only");
        println!("would write  {}", report.registry_path.display());
        println!("would write  {}", report.override_path.display());
        println!();
        println!("registry toml");
        print!("{}", crate::registry::to_toml(&report.app));
        println!();
        println!("compose override (would write)");
        print!("{}", report.override_text);
    } else {
        println!("wrote        {}", report.registry_path.display());
        println!("override     {}", report.override_path.display());
    }

    if !report.notes.is_empty() {
        println!();
        for note in &report.notes {
            println!("note         {note}");
        }
    }

    println!();
    print_import_next_steps(cli, args, report);
}

fn print_import_next_steps(cli: &Cli, args: &ImportArgs, report: &crate::import::Report) {
    let mut prefix = String::from("swapdock");
    if cli.registry != Path::new("/srv/swapdock/apps") {
        prefix.push_str(&format!(" --registry {}", shell_path(&cli.registry)));
    }
    let mut up_suffix = String::new();
    if args.state_dir != Path::new(crate::import::DEFAULT_STATE_DIR) {
        up_suffix.push_str(&format!(" --state-dir {}", shell_path(&args.state_dir)));
    }

    if report.dry_run {
        println!("next steps (re-run without --dry-run first):");
    } else {
        println!("next steps:");
    }

    let mut step = 1;
    if report.app.hostnames.is_empty() {
        println!("  {step}. add a hostname (traffic cannot reach the app without one):");
        println!("       edit {} and set:", report.registry_path.display());
        println!("         hostnames = [\"app.example.com\"]");
    } else {
        println!(
            "  {step}. hostname: {} (edit {} to add more)",
            report.app.hostnames.join(", "),
            report.registry_path.display()
        );
    }
    step += 1;

    let back = crate::ports::pair(report.app.slot)
        .map(|(back, _)| back)
        .unwrap_or(0);
    let health = match report.healthcheck_source {
        crate::import::HealthcheckSource::Compose => "keeps the compose healthcheck".to_string(),
        crate::import::HealthcheckSource::Flag => "installs the --healthcheck command".to_string(),
        crate::import::HealthcheckSource::Missing => {
            "installs no healthcheck (the deploy gate will refuse until one is added)".to_string()
        }
    };
    println!(
        "  {step}. the generated override {} pins ${{IMAGE}} (default {}) and publishes",
        report.override_path.display(),
        report.app.image_repo.as_deref().unwrap_or("<repo>")
    );
    println!(
        "     127.0.0.1:${{{}:-{back}}}:{} and {health}; your compose file was not edited.",
        report.app.env_name, report.container_port
    );
    step += 1;

    println!("  {step}. make the release image (the release tag is appended at deploy time):");
    println!("       {prefix} build {} --release SHA", report.app.name);
    if report.app.repo.is_none() {
        println!(
            "     (`build` needs a source checkout; set repo in {} or tag instead)",
            report.registry_path.display()
        );
    }
    println!("     or tag the image the service already runs:");
    println!(
        "       docker tag {} {}:SHA",
        report.image,
        report.app.image_repo.as_deref().unwrap_or("<repo>")
    );
    println!("     (SHA is the 7-64 character commit you are deploying)");
    step += 1;

    println!("  {step}. deploy:");
    println!(
        "       {prefix} up {} --release SHA{up_suffix}",
        report.app.name
    );
    step += 1;

    println!("  {step}. check the host wiring first (read-only):");
    println!("       {prefix} doctor {}", report.app.name);

    println!();
    println!("import does not deploy, start or stop anything. The first `up` is the deploy.");
}

/// A path quoted for a shell only when it needs it.
fn shell_path(path: &Path) -> String {
    let text = path.display().to_string();
    if !text.is_empty()
        && text.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '/' | '.' | '_' | '-')
        })
    {
        text
    } else {
        format!("{text:?}")
    }
}

/// `swapdock sync <app>`
fn sync_app(cli: &Cli, name: &str) -> anyhow::Result<()> {
    let app = load_app(cli, name)?;
    let mut run = start(cli)?;
    let run_id = run.id().to_string();
    match deploy::sync_repo(&mut run, &app) {
        Ok(sha) => {
            run.finish(RunStatus::Ok)?;
            println!("{name} is at {sha}");
            println!("run      {run_id}");
            Ok(())
        }
        Err(e) => {
            let _ = run.finish(RunStatus::Failed);
            Err(anyhow::anyhow!("{e}"))
        }
    }
}

/// `swapdock build <app> [--release] [--build-host]`
fn build_app(
    cli: &Cli,
    name: &str,
    release: Option<&str>,
    build_host: Option<&str>,
) -> anyhow::Result<()> {
    let app = load_app(cli, name)?;
    let release = release
        .map(str::to_string)
        .or(app.release.clone())
        .ok_or_else(|| anyhow::anyhow!("no release: pass --release, or sync the source first"))?;
    if !crate::registry::is_valid_release_arg(&release) {
        anyhow::bail!("invalid release {release:?}: must be a commit SHA (hex, 7-64 chars)");
    }

    let env_host = std::env::var("SWAPDOCK_BUILD_HOST").ok();
    let host = builder::resolve_host(build_host, env_host.as_deref(), &app);

    let mut run = start(cli)?;
    let run_id = run.id().to_string();
    let built = if builder::is_local(&host) {
        let work_parent = std::env::temp_dir().join("swapdock-build");
        builder::build_local(&mut run, &app, &release, &work_parent)
    } else {
        builder::build_ssh(&mut run, &app, &release, &host)
    }
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    run.finish(RunStatus::Ok)?;
    println!("built    {}", built.image_ref);
    println!("release  {}", built.release);
    println!("host     {host}");
    println!("run      {run_id}");
    println!();
    println!(
        "swapdock it with: swapdock up {name} --release {}",
        built.release
    );
    Ok(())
}

/// `swapdock backup <app>`
fn backup_app(cli: &Cli, name: Option<&str>, all: bool) -> anyhow::Result<()> {
    if all {
        let Some(loaded) = load(cli)? else {
            anyhow::bail!("no registry found at {}", cli.registry.display());
        };
        let apps: Vec<App> = loaded
            .sorted()
            .into_iter()
            .filter(|app| app.backup.is_some())
            .collect();
        if apps.is_empty() {
            anyhow::bail!("no app has a [backup] command");
        }
        let mut run = start(cli)?;
        let run_id = run.id().to_string();
        let mut failed = 0usize;
        for app in &apps {
            match crate::backup::run_backup(&mut run, app, app.release.as_deref()) {
                Ok(file) => match file {
                    Some(path) => println!("backup   {} {}", app.name, path.display()),
                    None => println!("backup   {} command completed", app.name),
                },
                Err(e) => {
                    failed += 1;
                    eprintln!("backup   {} failed: {e}", app.name);
                    if !cli.dry_run
                        && let Err(notify_err) =
                            crate::notify::send_failure(Some(&app.name), &run_id, &e.to_string())
                    {
                        tracing::warn!(app = %app.name, "failure webhook failed: {notify_err}");
                    }
                }
            }
        }
        let status = if failed > 0 {
            RunStatus::Failed
        } else if cli.dry_run {
            RunStatus::DryRun
        } else {
            RunStatus::Ok
        };
        run.finish(status)?;
        println!("apps     {} ({} failed)", apps.len(), failed);
        println!("run      {run_id}");
        if failed > 0 {
            anyhow::bail!("{failed} of {} backup(s) failed", apps.len());
        }
        return Ok(());
    }
    let Some(name) = name else {
        anyhow::bail!("pass an app name, or --all to back up every configured app");
    };
    let app = load_app(cli, name)?;
    let mut run = start(cli)?;
    let run_id = run.id().to_string();
    match crate::backup::run_backup(&mut run, &app, app.release.as_deref()) {
        Ok(file) => {
            run.finish(if cli.dry_run {
                RunStatus::DryRun
            } else {
                RunStatus::Ok
            })?;
            match file {
                Some(path) => println!("backup   {}", path.display()),
                None => println!("backup   command completed (no {{file}} placeholder)"),
            }
            println!("run      {run_id}");
            Ok(())
        }
        Err(e) => {
            let _ = run.finish(RunStatus::Failed);
            Err(anyhow::anyhow!("{e}"))
        }
    }
}

/// `swapdock restore <app> --file <snapshot>`
fn restore_app(cli: &Cli, name: &str, file: &Path) -> anyhow::Result<()> {
    let app = load_app(cli, name)?;
    let mut run = start(cli)?;
    let run_id = run.id().to_string();
    match crate::backup::run_restore(&mut run, &app, file) {
        Ok(()) => {
            run.finish(if cli.dry_run {
                RunStatus::DryRun
            } else {
                RunStatus::Ok
            })?;
            println!("restore  {}", file.display());
            println!("run      {run_id}");
            Ok(())
        }
        Err(e) => {
            let _ = run.finish(RunStatus::Failed);
            Err(anyhow::anyhow!("{e}"))
        }
    }
}

/// `swapdock reconcile <app>`
fn reconcile_app(cli: &Cli, name: &str, paths: &DeployPaths) -> anyhow::Result<()> {
    if cli.dry_run {
        anyhow::bail!(
            "reconcile repairs a crashed deploy; it has no dry-run because proving the host \
             state is the point. Inspect `swapdock status` and the pending record first."
        );
    }
    let mut app = load_app(cli, name)?;
    let ctx = ctx_from(cli, paths);
    let mut run = start(cli)?;
    let run_id = run.id().to_string();
    match crate::reconcile::reconcile(&mut run, &ctx, &mut app) {
        Ok(outcome) => {
            run.finish(RunStatus::Ok)?;
            match outcome {
                crate::reconcile::Outcome::Committed { release, live_port } => {
                    println!("reconciled {release}");
                    if let Some(port) = live_port {
                        println!("live port  :{port}");
                    }
                }
                crate::reconcile::Outcome::NotFlipped {
                    release,
                    expected_port,
                } => {
                    println!("not flipped {release} (expected upstream :{expected_port})");
                }
            }
            println!("run      {run_id}");
            Ok(())
        }
        Err(e) => {
            let _ = run.finish(RunStatus::Failed);
            if !cli.dry_run
                && let Err(notify_err) =
                    crate::notify::send_failure(Some(name), &run_id, &e.to_string())
            {
                tracing::warn!(app = %name, "failure webhook failed: {notify_err}");
            }
            Err(anyhow::anyhow!("{e}"))
        }
    }
}

/// `swapdock broker`
fn broker_app(
    cli: &Cli,
    socket: &Path,
    docker_bin: &Path,
    paths: &DeployPaths,
) -> anyhow::Result<()> {
    let token = std::env::var("SWAPDOCK_BROKER_TOKEN")
        .ok()
        .or_else(|| std::env::var("SWAPDOCK_API_TOKEN").ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("broker requires SWAPDOCK_BROKER_TOKEN or SWAPDOCK_API_TOKEN")
        })?;
    let apply_paths = crate::apply::ApplyPaths {
        nginx_bin: paths.nginx_bin.clone(),
        main_config: paths.main_config.clone(),
        target: paths.target.clone(),
        pid_file: Some(paths.pid_file.clone()),
    };
    crate::broker::serve(
        socket,
        cli.trace.clone(),
        apply_paths,
        token,
        docker_bin.to_path_buf(),
    )
}

/// `swapdock monitor [--json]`/// `swapdock monitor [--json]`
fn monitor_app(cli: &Cli, json: bool, paths: &DeployPaths) -> anyhow::Result<()> {
    let apps = load(cli)?.map(|loaded| loaded.sorted()).unwrap_or_default();
    let report = crate::monitor::run(
        &apps,
        &paths.state_dir,
        crate::monitor::disk_threshold_from_env(),
        &crate::monitor::certs_from_env(),
        crate::monitor::repeat_secs_from_env(),
        !cli.dry_run,
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "findings": report.findings.iter().map(|f| serde_json::json!({
                    "key": f.key,
                    "kind": f.kind,
                    "message": f.message,
                    "detail": f.detail,
                })).collect::<Vec<_>>(),
                "sent": report.sent,
                "suppressed": report.suppressed,
                "errors": report.errors,
            }))?
        );
    } else {
        for finding in &report.findings {
            println!("{}  {}  {}", finding.kind, finding.key, finding.message);
        }
        for error in &report.errors {
            eprintln!("monitor  {error}");
        }
        if report.findings.is_empty() {
            println!("monitor  no findings");
        }
        println!(
            "monitor  {} finding(s), {} sent, {} suppressed, {} error(s)",
            report.findings.len(),
            report.sent,
            report.suppressed,
            report.errors.len()
        );
    }
    if !report.errors.is_empty() {
        anyhow::bail!("{} monitor error(s)", report.errors.len());
    }
    Ok(())
}

/// `swapdock prune [--app] [--keep N]`/// `swapdock prune [--app] [--keep N]`
fn prune_app(cli: &Cli, only: Option<&str>, keep: usize) -> anyhow::Result<()> {
    let Some(loaded) = load(cli)? else {
        return Ok(());
    };
    let apps = loaded.sorted();
    let selected: Vec<App> = match only {
        None => apps,
        Some(name) => {
            let found: Vec<App> = apps.into_iter().filter(|a| a.name == name).collect();
            if found.is_empty() {
                let mut known: Vec<&str> = loaded.apps.iter().map(|a| a.name.as_str()).collect();
                known.sort_unstable();
                anyhow::bail!(
                    "no app named {name:?} in {}; known apps: {}",
                    cli.registry.display(),
                    known.join(", ")
                );
            }
            found
        }
    };

    let mut run = start(cli)?;
    let run_id = run.id().to_string();
    let mut removed = 0usize;
    let mut failed = 0usize;

    for app in &selected {
        let Some(repo) = app.image_repo.as_deref() else {
            continue;
        };
        if cli.dry_run {
            println!(
                "would prune {repo}: keep release, old_release, latest and {keep} rollback tag(s)"
            );
            let seq = run.next_seq();
            run.record_step(
                crate::trace::StepRecord::new(
                    seq,
                    "prune",
                    crate::trace::StepStatus::DryRun,
                    &["docker".to_string(), "images".to_string(), repo.to_string()],
                )
                .with_detail(serde_json::json!({"keep": keep, "app": app.name})),
            )?;
            continue;
        }
        let outcome = run.exec(&crate::docker::image_tags(repo))?;
        if !outcome.success() {
            eprintln!(
                "warning: could not list images for {repo}: {}",
                outcome.stderr_trimmed()
            );
            continue;
        }
        let tags: Vec<String> = outcome
            .stdout
            .lines()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect();
        let keep_set = crate::prune::Keep {
            current: app.release.clone(),
            old: app.old_release.clone(),
        };
        let removals = crate::prune::select_removals(&tags, &keep_set, keep);
        for tag in &removals {
            let image = format!("{repo}:{tag}");
            let result = run.exec(&crate::docker::rmi(&image))?;
            if result.success() {
                println!("removed {image}");
                removed += 1;
            } else {
                eprintln!(
                    "warning: could not remove {image}: {}",
                    result.stderr_trimmed()
                );
                failed += 1;
            }
        }
    }

    run.finish(if cli.dry_run {
        RunStatus::DryRun
    } else if failed > 0 {
        RunStatus::Failed
    } else {
        RunStatus::Ok
    })?;
    println!();
    println!("removed   {removed}");
    println!("failed    {failed}");
    println!("run       {run_id}");
    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}

/// `swapdock probe <app> [--url]`
fn probe_app(cli: &Cli, name: &str, url: Option<&str>) -> anyhow::Result<()> {
    let app = load_app(cli, name)?;
    let url = url
        .map(str::to_string)
        .or_else(|| app.public_url.clone())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{name} has no public_url; pass --url https://host/path or set public_url in the registry"
            )
        })?;
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        anyhow::bail!("{url:?} must start with http:// or https://");
    }

    let mut run = start(cli)?;
    let run_id = run.id().to_string();
    let spec = StepSpec::new("probe-public", "curl")
        .args([
            "-fsS",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "--max-time",
            "15",
            &url,
        ])
        .timeout(Duration::from_secs(20));

    if cli.dry_run {
        run.exec(&spec)?;
        run.finish(RunStatus::DryRun)?;
        println!("would probe {url}");
        return Ok(());
    }

    let outcome = run.exec(&spec)?;
    let code = outcome.stdout_trimmed().parse::<u16>().unwrap_or(0);
    let ok = (200..400).contains(&code);
    println!("url     {url}");
    println!("status  {code}");
    println!("run     {run_id}");
    run.finish(if ok { RunStatus::Ok } else { RunStatus::Failed })?;
    if !ok {
        std::process::exit(1);
    }
    Ok(())
}

/// `swapdock verify <app> --since <ts|run-id>`
fn verify_app(cli: &Cli, name: &str, since: &str, access_log: &Path) -> anyhow::Result<()> {
    let app = load_app(cli, name)?;

    // A run id resolves to when that run started; anything else must parse as
    // an RFC 3339 timestamp.
    let since_ms = match crate::id::RunId::parse(since) {
        Some(id) => run_started_at(cli, &id)
            .ok_or_else(|| anyhow::anyhow!("no run {id} in {}", cli.trace.display()))?,
        None => crate::time::parse_rfc3339(since).ok_or_else(|| {
            anyhow::anyhow!("{since:?} is neither a run id nor an RFC 3339 timestamp")
        })?,
    };

    let lines = crate::verify::read_lines(access_log)?;
    let report = crate::verify::verify(&app.hostnames, since_ms, lines.into_iter());

    println!("app       {name}");
    println!(
        "since     {}",
        crate::time::Timestamp::from_epoch_millis(since_ms).to_rfc3339()
    );
    println!("requests  {}", report.requests);
    println!("errors    {} (5xx)", report.errors_5xx);
    if report.malformed > 0 {
        println!("malformed {} line(s) skipped", report.malformed);
    }
    println!();
    if report.upstreams.is_empty() {
        println!("no requests in the window");
    } else {
        let rows: Vec<Vec<String>> = report
            .upstreams
            .iter()
            .map(|(up, info)| {
                vec![
                    up.clone(),
                    info.requests.to_string(),
                    info.errors_5xx.to_string(),
                    crate::time::Timestamp::from_epoch_millis(info.first_ms).to_rfc3339(),
                ]
            })
            .collect();
        print_table(&["upstream", "requests", "5xx", "first seen"], &rows);
    }
    if let Some(flip) = &report.flip {
        println!();
        println!(
            "flip      {} -> {} at {}",
            flip.from,
            flip.to,
            crate::time::Timestamp::from_epoch_millis(flip.at_ms).to_rfc3339()
        );
    }
    if report.errors_5xx > 0 {
        std::process::exit(1);
    }
    Ok(())
}

/// When a run started, for `--since <run-id>`.
fn run_started_at(cli: &Cli, id: &crate::id::RunId) -> Option<i64> {
    let read = crate::trace::TraceLog::read(&cli.trace).ok()?;
    read.events.iter().find_map(|e| match e {
        crate::trace::TraceEvent::RunStart { run_id, ts_ms, .. } if run_id == id.as_str() => {
            Some(*ts_ms)
        }
        _ => None,
    })
}

/// Prints an aligned table to stdout.
fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    use std::io::Write;

    let cols = headers.len();
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|row| row.iter().map(|c| flatten_cell(c)).collect())
        .collect();

    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in &cells {
        for (i, cell) in row.iter().enumerate().take(cols) {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let header_line = headers
        .iter()
        .enumerate()
        .map(|(i, h)| pad(h, widths[i]))
        .collect::<Vec<_>>()
        .join("  ");
    let _ = writeln!(out, "{header_line}");
    let rule = widths
        .iter()
        .map(|w| "-".repeat(*w))
        .collect::<Vec<_>>()
        .join("  ");
    let _ = writeln!(out, "{rule}");

    for row in &cells {
        let line = row
            .iter()
            .enumerate()
            .map(|(i, c)| pad(c, widths.get(i).copied().unwrap_or(0)))
            .collect::<Vec<_>>()
            .join("  ");
        let _ = writeln!(out, "{line}");
    }
}

fn pad(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - len))
    }
}

fn first_line(s: &str) -> String {
    let line = s.lines().next().unwrap_or("").trim();
    if line.chars().count() > 60 {
        let head: String = line.chars().take(57).collect();
        format!("{head}...")
    } else {
        line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_bare_selftest() {
        let cli = Cli::try_parse_from(["swapdock", "selftest"]).unwrap();
        assert!(matches!(cli.command, Command::Selftest));
        assert_eq!(cli.trace, PathBuf::from("/var/log/swapdock/swapdock.jsonl"));
        assert!(!cli.dry_run);
    }

    #[test]
    fn global_flags_work_before_and_after_the_subcommand() {
        let a = Cli::try_parse_from(["swapdock", "--dry-run", "selftest"]).unwrap();
        let b = Cli::try_parse_from(["swapdock", "selftest", "--dry-run"]).unwrap();
        assert!(a.dry_run && b.dry_run);

        let c = Cli::try_parse_from(["swapdock", "--trace", "/tmp/x.jsonl", "runs"]).unwrap();
        assert_eq!(c.trace, PathBuf::from("/tmp/x.jsonl"));
    }

    #[test]
    fn parses_run_identification_arguments() {
        let cli =
            Cli::try_parse_from(["swapdock", "show", "01JQ7FABCDEFGHJKMNPQRSTUVWXYZ"]).unwrap();
        match cli.command {
            Command::Show { run_id } => {
                assert_eq!(run_id, "01JQ7FABCDEFGHJKMNPQRSTUVWXYZ")
            }
            _ => panic!("expected Show"),
        }

        let cli = Cli::try_parse_from(["swapdock", "runs", "--limit", "5"]).unwrap();
        match cli.command {
            Command::Runs { limit } => assert_eq!(limit, 5),
            _ => panic!("expected Runs"),
        }
    }

    #[test]
    fn rejects_an_unknown_subcommand() {
        assert!(Cli::try_parse_from(["swapdock", "frobnicate"]).is_err());
    }

    #[test]
    fn pads_columns_to_a_common_width() {
        assert_eq!(pad("ab", 5), "ab   ");
        assert_eq!(pad("abcdef", 3), "abcdef");
        assert_eq!(pad("", 2), "  ");
    }

    #[test]
    fn shortens_a_long_first_line() {
        assert_eq!(first_line("  hello  "), "hello");
        assert_eq!(first_line(""), "");
        let long = "x".repeat(100);
        assert_eq!(first_line(&long).chars().count(), 60);
    }

    #[test]
    fn parses_the_new_subcommands() {
        assert!(matches!(
            Cli::try_parse_from(["swapdock", "render"]).unwrap().command,
            Command::Render { .. }
        ));
        assert!(matches!(
            Cli::try_parse_from(["swapdock", "validate"])
                .unwrap()
                .command,
            Command::Validate
        ));
        let cli = Cli::try_parse_from(["swapdock", "render", "--app", "portfolio"]).unwrap();
        match cli.command {
            Command::Render { app } => assert_eq!(app.as_deref(), Some("portfolio")),
            _ => panic!("expected Render"),
        }
    }

    #[test]
    fn up_rollback_and_sync_parse() {
        let cli = Cli::try_parse_from(["swapdock", "up", "portfolio"]).unwrap();
        match cli.command {
            Command::Up { app, release, .. } => {
                assert_eq!(app, "portfolio");
                assert_eq!(release, None);
            }
            _ => panic!("expected Up"),
        }

        let cli =
            Cli::try_parse_from(["swapdock", "up", "portfolio", "--release", "9c1f2ab"]).unwrap();
        match cli.command {
            Command::Up { release, .. } => assert_eq!(release.as_deref(), Some("9c1f2ab")),
            _ => panic!("expected Up"),
        }

        assert!(matches!(
            Cli::try_parse_from(["swapdock", "rollback", "portfolio"])
                .unwrap()
                .command,
            Command::Rollback { .. }
        ));
        assert!(matches!(
            Cli::try_parse_from(["swapdock", "sync", "portfolio"])
                .unwrap()
                .command,
            Command::Sync { .. }
        ));
    }

    #[test]
    fn deploy_paths_have_documented_defaults() {
        let cli = Cli::try_parse_from(["swapdock", "up", "portfolio"]).unwrap();
        match cli.command {
            Command::Up { paths, .. } => {
                assert_eq!(
                    paths.target,
                    PathBuf::from("/etc/nginx/conf.d/front-door.conf")
                );
                assert_eq!(paths.main_config, PathBuf::from("/etc/nginx/nginx.conf"));
                assert_eq!(paths.pid_file, PathBuf::from("/run/nginx.pid"));
                assert_eq!(paths.nginx_bin, PathBuf::from("nginx"));
                assert_eq!(paths.state_dir, PathBuf::from("/srv/swapdock/green"));
                assert_eq!(paths.lock_dir, PathBuf::from("/run/swapdock"));
                assert_eq!(paths.drain_secs, crate::deploy::DRAIN_SECS);
            }
            _ => panic!("expected Up"),
        }
    }

    #[test]
    fn apply_has_documented_defaults() {
        let cli = Cli::try_parse_from(["swapdock", "apply"]).unwrap();
        match cli.command {
            Command::Apply { paths } => {
                assert_eq!(
                    paths.target,
                    PathBuf::from("/etc/nginx/conf.d/front-door.conf")
                );
                assert_eq!(paths.main_config, PathBuf::from("/etc/nginx/nginx.conf"));
                assert_eq!(paths.pid_file, PathBuf::from("/run/nginx.pid"));
                assert_eq!(paths.nginx_bin, PathBuf::from("nginx"));
            }
            _ => panic!("expected Apply"),
        }

        let cli = Cli::try_parse_from([
            "swapdock",
            "apply",
            "--target",
            "/tmp/f.conf",
            "--nginx-bin",
            "/tmp/fake",
        ])
        .unwrap();
        match cli.command {
            Command::Apply { paths } => {
                assert_eq!(paths.target, PathBuf::from("/tmp/f.conf"));
                assert_eq!(paths.nginx_bin, PathBuf::from("/tmp/fake"));
            }
            _ => panic!("expected Apply"),
        }
    }

    #[test]
    fn operations_commands_parse() {
        let cli = Cli::try_parse_from([
            "swapdock",
            "adopt",
            "chats",
            "--release",
            "69ff7b5",
            "--healthcheck",
            "true",
            "--container-port",
            "3000",
        ])
        .unwrap();
        match cli.command {
            Command::Adopt {
                app,
                release,
                healthcheck,
                container_port,
                ..
            } => {
                assert_eq!(app, "chats");
                assert_eq!(release, "69ff7b5");
                assert_eq!(healthcheck.as_deref(), Some("true"));
                assert_eq!(container_port, Some(3000));
            }
            _ => panic!("expected Adopt"),
        }

        let cli = Cli::try_parse_from(["swapdock", "doctor", "--app", "chat", "--public"]).unwrap();
        match cli.command {
            Command::Doctor { app, public } => {
                assert_eq!(app.as_deref(), Some("chat"));
                assert!(public);
            }
            _ => panic!("expected Doctor"),
        }

        let cli = Cli::try_parse_from(["swapdock", "backup", "chats"]).unwrap();
        match cli.command {
            Command::Backup { app, all } => {
                assert_eq!(app.as_deref(), Some("chats"));
                assert!(!all);
            }
            _ => panic!("expected Backup"),
        }
        let cli = Cli::try_parse_from(["swapdock", "backup", "--all"]).unwrap();
        assert!(matches!(cli.command, Command::Backup { all: true, .. }));

        let cli =
            Cli::try_parse_from(["swapdock", "restore", "chats", "--file", "/tmp/chats.dump"])
                .unwrap();
        match cli.command {
            Command::Restore { app, file } => {
                assert_eq!(app, "chats");
                assert_eq!(file, PathBuf::from("/tmp/chats.dump"));
            }
            _ => panic!("expected Restore"),
        }

        let cli = Cli::try_parse_from(["swapdock", "reconcile", "chats"]).unwrap();
        assert!(matches!(cli.command, Command::Reconcile { .. }));

        let cli = Cli::try_parse_from([
            "swapdock",
            "serve",
            "--tls-cert",
            "/tmp/cert.pem",
            "--tls-key",
            "/tmp/key.pem",
        ])
        .unwrap();
        match cli.command {
            Command::Serve {
                tls_cert, tls_key, ..
            } => {
                assert_eq!(tls_cert.as_deref(), Some(Path::new("/tmp/cert.pem")));
                assert_eq!(tls_key.as_deref(), Some(Path::new("/tmp/key.pem")));
            }
            _ => panic!("expected Serve"),
        }
        assert!(
            Cli::try_parse_from(["swapdock", "serve", "--tls-cert", "/tmp/cert.pem"]).is_err(),
            "TLS cert without key must be refused"
        );

        let cli = Cli::try_parse_from(["swapdock", "monitor", "--json"]).unwrap();
        match cli.command {
            Command::Monitor { json, .. } => assert!(json),
            _ => panic!("expected Monitor"),
        }

        let cli = Cli::try_parse_from(["swapdock", "probe", "chat"]).unwrap();
        assert!(matches!(cli.command, Command::Probe { .. }));

        let cli =
            Cli::try_parse_from(["swapdock", "prune", "--app", "chat", "--keep", "2"]).unwrap();
        match cli.command {
            Command::Prune { app, keep } => {
                assert_eq!(app.as_deref(), Some("chat"));
                assert_eq!(keep, 2);
            }
            _ => panic!("expected Prune"),
        }

        let cli = Cli::try_parse_from(["swapdock", "inspect", "chat"]).unwrap();
        assert!(matches!(cli.command, Command::Inspect { .. }));
    }

    #[test]
    fn init_parses_and_writes_a_valid_template() {
        let cli = Cli::try_parse_from(["swapdock", "init", "blog"]).unwrap();
        match cli.command {
            Command::Init { name, dir, kind } => {
                assert_eq!(name, "blog");
                assert_eq!(dir, None);
                assert_eq!(kind, "container");
            }
            _ => panic!("expected Init"),
        }

        for kind in ["container", "static"] {
            let text = init_template("blog", kind).unwrap();
            let app: crate::registry::App = toml::from_str(&text).unwrap();
            assert_eq!(app.name, "blog");
            // A fresh template validates except for the placeholder port/slot,
            // which the operator must set.
            assert!(app.front_port == 8001 && app.slot == 0);
        }

        assert!(init_template("Bad Name", "container").is_err());
        assert!(init_template("blog", "vm").is_err());
    }

    #[test]
    fn init_refuses_to_overwrite() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("blog.toml"), "name = \"x\"\n").unwrap();
        let err = init_app("blog", Some(dir.path().to_path_buf()), "container").unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");
    }

    #[test]
    fn init_writes_a_file_that_validates() {
        let dir = tempfile::TempDir::new().unwrap();
        init_app("blog", Some(dir.path().to_path_buf()), "static").unwrap();
        let raw = std::fs::read_to_string(dir.path().join("blog.toml")).unwrap();
        assert!(raw.contains("root = \"/srv/www/blog/current\""), "{raw}");
        let app: crate::registry::App = toml::from_str(&raw).unwrap();
        assert_eq!(app.kind, crate::registry::Kind::Static);
    }

    #[test]
    fn build_and_verify_parse() {
        let cli = Cli::try_parse_from(["swapdock", "build", "portfolio"]).unwrap();
        match cli.command {
            Command::Build {
                app,
                release,
                build_host,
            } => {
                assert_eq!(app, "portfolio");
                assert_eq!(release, None);
                assert_eq!(build_host, None);
            }
            _ => panic!("expected Build"),
        }

        let cli = Cli::try_parse_from([
            "swapdock",
            "build",
            "portfolio",
            "--release",
            "9c1f2ab",
            "--build-host",
            "buildbox",
        ])
        .unwrap();
        match cli.command {
            Command::Build {
                release,
                build_host,
                ..
            } => {
                assert_eq!(release.as_deref(), Some("9c1f2ab"));
                assert_eq!(build_host.as_deref(), Some("buildbox"));
            }
            _ => panic!("expected Build"),
        }

        let cli = Cli::try_parse_from([
            "swapdock",
            "verify",
            "portfolio",
            "--since",
            "2026-10-04T21:00:00+00:00",
        ])
        .unwrap();
        match cli.command {
            Command::Verify {
                app,
                since,
                access_log,
            } => {
                assert_eq!(app, "portfolio");
                assert_eq!(since, "2026-10-04T21:00:00+00:00");
                assert_eq!(access_log, PathBuf::from(crate::verify::DEFAULT_ACCESS_LOG));
            }
            _ => panic!("expected Verify"),
        }
    }

    #[test]
    fn up_takes_force_and_status_parses() {
        let cli = Cli::try_parse_from(["swapdock", "up", "portfolio", "--force"]).unwrap();
        match cli.command {
            Command::Up { app, force, .. } => {
                assert_eq!(app, "portfolio");
                assert!(force);
            }
            _ => panic!("expected Up"),
        }
        // The default stays a plain deploy.
        let cli = Cli::try_parse_from(["swapdock", "up", "portfolio"]).unwrap();
        match cli.command {
            Command::Up { force, .. } => assert!(!force),
            _ => panic!("expected Up"),
        }

        let cli = Cli::try_parse_from(["swapdock", "status", "portfolio"]).unwrap();
        match cli.command {
            Command::Status { app } => assert_eq!(app, "portfolio"),
            _ => panic!("expected Status"),
        }
    }

    #[test]
    fn registry_and_tunnel_flags_have_documented_defaults() {
        let cli = Cli::try_parse_from(["swapdock", "validate"]).unwrap();
        assert_eq!(cli.registry, PathBuf::from("/srv/swapdock/apps"));
        assert_eq!(cli.tunnel_config, None);

        let cli = Cli::try_parse_from([
            "swapdock",
            "--registry",
            "/tmp/apps",
            "--tunnel-config",
            "/tmp/cloudflared.yaml",
            "validate",
        ])
        .unwrap();
        assert_eq!(cli.registry, PathBuf::from("/tmp/apps"));
        assert_eq!(
            cli.tunnel_config,
            Some(PathBuf::from("/tmp/cloudflared.yaml"))
        );
    }

    #[test]
    fn a_multi_line_cell_is_flattened() {
        // A TOML parse error spans several lines; it must not break the columns.
        assert_eq!(flatten_cell("a\nb"), "a b");
        assert_eq!(flatten_cell("a\n  b"), "a b");
        assert_eq!(flatten_cell("a\n\n\nb"), "a b");
        assert_eq!(flatten_cell("no breaks"), "no breaks");
        assert_eq!(flatten_cell("trailing\n"), "trailing");
        assert_eq!(flatten_cell("  leading"), "leading");
        assert_eq!(flatten_cell("   "), "");
        assert_eq!(flatten_cell(""), "");
    }

    #[test]
    fn print_table_survives_a_multi_line_cell() {
        let rows = vec![
            vec!["short".to_string(), "ok".to_string()],
            vec!["a\nb\nc".to_string(), "also-ok".to_string()],
        ];
        print_table(&["code", "app"], &rows);
    }

    #[test]
    fn print_table_handles_rows_of_differing_width() {
        // Regression: the table printer must not assume a fixed column count.
        let rows = vec![
            vec!["a".to_string()],
            vec!["b".to_string(), "c".to_string()],
        ];
        print_table(&["one", "two"], &rows);
    }
}
