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

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};

use crate::apply::{self, ApplyPaths};
use crate::deploy::{self, Ctx};
use crate::exec::{ExecError, StepSpec};
use crate::id::RunId;
use crate::redact::Redactor;
use crate::registry::{App, Loaded, Problem};
use crate::render;
use crate::trace::{Run, RunMode, RunStatus, TraceEvent, TraceLog};
use crate::tunnel;
use crate::validator::{self, TunnelRoutes};

const ABOUT: &str = "Deploy tool for a multi-app Docker host.\n\n\
                      Milestone 1: subprocess chokepoint, run log, redaction.";

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
    #[arg(long, default_value = "/srv/deploy/green", value_name = "DIR")]
    pub state_dir: PathBuf,

    /// Directory for lock files.
    #[arg(long, default_value = "/run/deploy", value_name = "DIR")]
    pub lock_dir: PathBuf,

    /// Seconds to let old workers drain after the flip.
    #[arg(long, default_value_t = crate::deploy::DRAIN_SECS)]
    pub drain_secs: u64,
}

#[derive(Debug, Parser)]
#[command(name = "deploy", version, about = ABOUT, long_about = None)]
pub struct Cli {
    /// Path to the append-only run log.
    #[arg(
        long,
        global = true,
        default_value = "deploy.jsonl",
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
        default_value = "/srv/deploy/apps",
        value_name = "DIR"
    )]
    pub registry: PathBuf,

    /// cloudflared config to cross-check routes against.
    #[arg(long, global = true, value_name = "PATH")]
    pub tunnel_config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

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
        /// Run id, as shown by `deploy runs`.
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

    /// Deploy one app through its registry strategy.
    Up {
        /// App name, as in the registry.
        app: String,
        /// Commit to deploy. Defaults to the recorded release.
        #[arg(long, value_name = "SHA")]
        release: Option<String>,
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

    /// Fetch the app source and fast-forward it. Prints the new HEAD.
    Sync {
        /// App name, as in the registry.
        app: String,
    },

    /// Render the config, refuse on any error, then atomically apply and reload.
    Apply {
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
            Command::Apply { paths } => apply_config(
                cli,
                &paths.target,
                &paths.main_config,
                &paths.pid_file,
                &paths.nginx_bin,
            ),
            Command::Up {
                app,
                release,
                paths,
            } => up_app(cli, app, release.as_deref(), paths),
            Command::Rollback { app, paths } => rollback_app(cli, app, paths),
            Command::Sync { app } => sync_app(cli, app),
        }
    }
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

/// `deploy selftest`
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
    println!("  deploy --trace {} show {run_id}", cli.trace.display());
    println!(
        "  grep -o '\"step\":\"[a-z-]*\"' {} | sort | uniq -c",
        cli.trace.display()
    );

    if leaked {
        anyhow::bail!("a secret reached the log");
    }
    Ok(())
}

/// One row of `deploy runs`.
struct RunSummary {
    run_id: String,
    started: String,
    status: String,
    steps: u32,
    /// Steps that did not succeed. A non-zero exit is a fact about one step, not
    /// proof that the run stopped there, so it is counted rather than used to
    /// name a stopping point.
    non_ok: u32,
}

/// `deploy runs`
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
                run_id, ts, mode, ..
            } => {
                summaries.push(RunSummary {
                    run_id: run_id.clone(),
                    started: ts.clone(),
                    status: format!("{mode:?}"),
                    steps: 0,
                    non_ok: 0,
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
                s.status.clone(),
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

/// `deploy show <run-id>`
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

/// `deploy resume <run-id>`
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
                (None, false) => {
                    println!("the run has no final record; continue from the next step");
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

/// `deploy render`
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

/// `deploy validate`
///
/// Reads only. Exits non-zero when anything is an error, so it can gate a deploy.
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

/// `deploy apply`
///
/// Render, refuse on any error, then stage, test, commit and reload. The only
/// command that changes the host.
fn apply_config(
    cli: &Cli,
    target: &Path,
    main_config: &Path,
    pid_file: &Path,
    nginx_bin: &Path,
) -> anyhow::Result<()> {
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
            println!("applied  {}", target.display());
            println!("reloaded {}", outcome.reloaded);
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
fn ctx_from(cli: &Cli, paths: &DeployPaths) -> Ctx {
    Ctx {
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

/// `deploy up <app> [--release]`
fn up_app(cli: &Cli, name: &str, release: Option<&str>, paths: &DeployPaths) -> anyhow::Result<()> {
    let mut app = load_app(cli, name)?;
    let release = deploy::resolve_release(&app, release).map_err(|e| anyhow::anyhow!("{e}"))?;
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

/// `deploy rollback <app>`
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

/// `deploy sync <app>`
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

/// Prints an aligned table to stdout.
fn print_table(headers: &[&str], rows: &[Vec<String>]) {
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
        let cli = Cli::try_parse_from(["deploy", "selftest"]).unwrap();
        assert!(matches!(cli.command, Command::Selftest));
        assert_eq!(cli.trace, PathBuf::from("deploy.jsonl"));
        assert!(!cli.dry_run);
    }

    #[test]
    fn global_flags_work_before_and_after_the_subcommand() {
        let a = Cli::try_parse_from(["deploy", "--dry-run", "selftest"]).unwrap();
        let b = Cli::try_parse_from(["deploy", "selftest", "--dry-run"]).unwrap();
        assert!(a.dry_run && b.dry_run);

        let c = Cli::try_parse_from(["deploy", "--trace", "/tmp/x.jsonl", "runs"]).unwrap();
        assert_eq!(c.trace, PathBuf::from("/tmp/x.jsonl"));
    }

    #[test]
    fn parses_run_identification_arguments() {
        let cli = Cli::try_parse_from(["deploy", "show", "01JQ7FABCDEFGHJKMNPQRSTUVWXYZ"]).unwrap();
        match cli.command {
            Command::Show { run_id } => {
                assert_eq!(run_id, "01JQ7FABCDEFGHJKMNPQRSTUVWXYZ")
            }
            _ => panic!("expected Show"),
        }

        let cli = Cli::try_parse_from(["deploy", "runs", "--limit", "5"]).unwrap();
        match cli.command {
            Command::Runs { limit } => assert_eq!(limit, 5),
            _ => panic!("expected Runs"),
        }
    }

    #[test]
    fn rejects_an_unknown_subcommand() {
        assert!(Cli::try_parse_from(["deploy", "frobnicate"]).is_err());
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
            Cli::try_parse_from(["deploy", "render"]).unwrap().command,
            Command::Render { .. }
        ));
        assert!(matches!(
            Cli::try_parse_from(["deploy", "validate"]).unwrap().command,
            Command::Validate
        ));
        let cli = Cli::try_parse_from(["deploy", "render", "--app", "portfolio"]).unwrap();
        match cli.command {
            Command::Render { app } => assert_eq!(app.as_deref(), Some("portfolio")),
            _ => panic!("expected Render"),
        }
    }

    #[test]
    fn up_rollback_and_sync_parse() {
        let cli = Cli::try_parse_from(["deploy", "up", "portfolio"]).unwrap();
        match cli.command {
            Command::Up { app, release, .. } => {
                assert_eq!(app, "portfolio");
                assert_eq!(release, None);
            }
            _ => panic!("expected Up"),
        }

        let cli =
            Cli::try_parse_from(["deploy", "up", "portfolio", "--release", "9c1f2ab"]).unwrap();
        match cli.command {
            Command::Up { release, .. } => assert_eq!(release.as_deref(), Some("9c1f2ab")),
            _ => panic!("expected Up"),
        }

        assert!(matches!(
            Cli::try_parse_from(["deploy", "rollback", "portfolio"])
                .unwrap()
                .command,
            Command::Rollback { .. }
        ));
        assert!(matches!(
            Cli::try_parse_from(["deploy", "sync", "portfolio"])
                .unwrap()
                .command,
            Command::Sync { .. }
        ));
    }

    #[test]
    fn deploy_paths_have_documented_defaults() {
        let cli = Cli::try_parse_from(["deploy", "up", "portfolio"]).unwrap();
        match cli.command {
            Command::Up { paths, .. } => {
                assert_eq!(
                    paths.target,
                    PathBuf::from("/etc/nginx/conf.d/front-door.conf")
                );
                assert_eq!(paths.main_config, PathBuf::from("/etc/nginx/nginx.conf"));
                assert_eq!(paths.pid_file, PathBuf::from("/run/nginx.pid"));
                assert_eq!(paths.nginx_bin, PathBuf::from("nginx"));
                assert_eq!(paths.state_dir, PathBuf::from("/srv/deploy/green"));
                assert_eq!(paths.lock_dir, PathBuf::from("/run/deploy"));
                assert_eq!(paths.drain_secs, crate::deploy::DRAIN_SECS);
            }
            _ => panic!("expected Up"),
        }
    }

    #[test]
    fn apply_has_documented_defaults() {
        let cli = Cli::try_parse_from(["deploy", "apply"]).unwrap();
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
            "deploy",
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
    fn registry_and_tunnel_flags_have_documented_defaults() {
        let cli = Cli::try_parse_from(["deploy", "validate"]).unwrap();
        assert_eq!(cli.registry, PathBuf::from("/srv/deploy/apps"));
        assert_eq!(cli.tunnel_config, None);

        let cli = Cli::try_parse_from([
            "deploy",
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
