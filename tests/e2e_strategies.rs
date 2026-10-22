//! End-to-end strategy tests against a real Docker daemon.
//!
//! These build a tiny throwaway estate — one compose project, one service, one
//! registry file — and run the actual `run_swap` / `run_replace` machinery
//! against it. The only fake is nginx: a shell script standing in for the
//! binary, because the point here is the container orchestration, and the nginx
//! interaction was already proven against the real binary in milestone 3.
//!
//! Skipped without a Docker daemon. Tests run serially (one global lock): the
//! fake nginx and the seed step configure children through process environment,
//! which is process-global. Each test cleans up its containers even on failure.

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use swapdock::registry::{self, App, ImageRegistry, Kind, Strategy};
use swapdock::{DeployContext, Run, RunMode};
use tempfile::TempDir;

/// Serializes this file's tests. See the module docs for why.
static SERIAL: Mutex<()> = Mutex::new(());

/// A writable `DOCKER_CONFIG` for this test process.
///
/// Buildx writes an activity marker under `DOCKER_CONFIG`; in a sandboxed test
/// run the real `$HOME/.docker` may be read-only, which would fail `docker
/// build` for reasons unrelated to swapdock.
fn docker_config_dir() -> &'static std::path::Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("swapdock-e2e-docker-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: this runs during serialized setup, before the tests that
        // build images spawn their children.
        unsafe { std::env::set_var("DOCKER_CONFIG", &dir) };
        dir
    })
}

/// Skip when there is no daemon. Prints once so the skip is visible.
fn daemon() -> bool {
    let ok = Command::new("docker")
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        eprintln!("SKIP e2e_strategies: no Docker daemon");
    } else {
        let _ = docker_config_dir();
    }
    ok
}

fn docker(argv: &[&str]) -> std::process::Output {
    Command::new("docker")
        .args(argv)
        .output()
        .expect("docker must run")
}

/// A throwaway estate: compose project, registry, state, locks, fake nginx.
struct Estate {
    dir: TempDir,
    /// The compose project directory. Stored, never discovered: the temp dir
    /// holds more than one subdirectory, so discovery picks the wrong one.
    projdir: PathBuf,
    /// Holds SERIAL until the test ends, including Drop cleanup.
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl Estate {
    fn setup(app_name: &str, strategy: Strategy) -> Option<Self> {
        if !daemon() {
            return None;
        }
        let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let dir = TempDir::new().unwrap();

        // Fixed lowercase project dir, unique per process: compose_project()
        // derives from this name, so it is stable and already valid.
        let projdir = dir
            .path()
            .join(format!("e2e{}-{}", std::process::id(), app_name));
        std::fs::create_dir_all(&projdir).unwrap();

        // One service. Port and image come from the environment; the strategy
        // sets both per invocation, exactly like production.
        std::fs::write(
            projdir.join("docker-compose.yml"),
            "services:\n  web:\n    image: ${IMAGE:-nginx:alpine}\n    ports:\n      - \"127.0.0.1:${WEB_PORT}:80\"\n    healthcheck:\n      test: [\"CMD\", \"wget\", \"-q\", \"-O\", \"/dev/null\", \"http://127.0.0.1/\"]\n      interval: 2s\n      timeout: 2s\n      retries: 2\n      start_period: 3s\n",
        )
        .unwrap();

        assert!(
            docker(&["image", "inspect", "nginx:alpine"])
                .status
                .success(),
            "nginx:alpine must be available locally"
        );

        // The harness nginx.conf only has to exist; the fake never reads it.
        std::fs::write(projdir.join("nginx.conf"), "# test harness\n").unwrap();

        let estate = Self {
            dir,
            projdir,
            _guard: guard,
        };
        estate.write_app(app_name, strategy);
        estate.write_fake_nginx();
        Some(estate)
    }

    fn compose_dir(&self) -> PathBuf {
        self.projdir.clone()
    }

    fn registry_dir(&self) -> PathBuf {
        self.dir.path().join("apps")
    }

    /// Writes (or rewrites) the registry file and returns the app.
    fn write_app(&self, name: &str, strategy: Strategy) -> App {
        let projdir = self.projdir.clone();
        std::fs::create_dir_all(self.registry_dir()).unwrap();
        let app = App {
            name: name.into(),
            kind: Kind::Container,
            strategy,
            hostnames: vec![],
            listen: vec![],
            front_port: 18999,
            slot: 100,
            live_port: None,
            old_port: None,
            writes_state: strategy == Strategy::Replace,
            image_repo: Some(format!("e2e-{name}")),
            registry: Some(ImageRegistry::Local),
            release: None,
            old_release: None,
            build_host: Some("local".into()),
            root: None,
            spa: false,
            // nginx:alpine serves / with 200.
            health_url: Some("http://127.0.0.1/".into()),
            compose_dir: projdir.to_path_buf(),
            compose_svc: "web".into(),
            env_name: "WEB_PORT".into(),
            git_remote: None,
            branch: None,
            repo: None,
            build_repo: None,
            compose_override: None,
            public_url: None,
            backup: None,
        };
        registry::save_app(&self.registry_dir(), &app).unwrap();
        app
    }

    /// Prints every problem in the on-disk estate. Debug aid for failures.
    /// Prints problems for a hypothetical live port. Debug aid for failures.
    fn dump_new_state(&self, live: u16) {
        let loaded = registry::load_dir(&self.registry_dir()).unwrap();
        let mut apps = loaded.sorted();
        if let Some(entry) = apps.iter_mut().find(|a| a.name == "rollbackapp") {
            entry.live_port = Some(live);
        }
        let mut problems = Vec::new();
        for a in &apps {
            problems.extend(a.problems());
        }
        problems.extend(swapdock::validator::validate(
            &apps,
            &swapdock::validator::TunnelRoutes::new(),
        ));
        for p in &problems {
            eprintln!("NEW-STATE PROBLEM {p}");
        }
    }

    fn dump_problems(&self) {
        let loaded = registry::load_dir(&self.registry_dir()).unwrap();
        let mut problems = loaded.problems.clone();
        for a in &loaded.apps {
            problems.extend(a.problems());
        }
        problems.extend(swapdock::validator::validate(
            &loaded.sorted(),
            &swapdock::validator::TunnelRoutes::new(),
        ));
        for p in &problems {
            eprintln!("PROBLEM {p}");
        }
    }

    fn load_app(&self, name: &str) -> App {
        let loaded = registry::load_dir(&self.registry_dir()).unwrap();
        assert!(loaded.problems.is_empty(), "{:?}", loaded.problems);
        loaded
            .sorted()
            .into_iter()
            .find(|a| a.name == name)
            .unwrap()
    }

    /// Tags nginx:alpine as a new fake release and returns its name.
    fn new_release(&self, app: &App, n: u32) -> String {
        // A hex name of 7+ characters, shaped like a short commit.
        let sha = format!("a1b2c3{n:x}");
        assert!(sha.len() >= 7);
        let image_ref = app.image_ref(&sha).unwrap();
        assert!(
            docker(&["tag", "nginx:alpine", &image_ref])
                .status
                .success()
        );
        sha
    }

    /// Builds a release image that is *not* `nginx:alpine`, so its image id
    /// differs. Rollback tag integrity cannot be tested with one shared image.
    fn new_distinct_release(&self, app: &App, n: u32, dockerfile: &str) -> String {
        let sha = format!("b1c2d3{n:x}");
        assert!(sha.len() >= 7);
        let image_ref = app.image_ref(&sha).unwrap();
        let context = self.dir.path().join(format!("build-{sha}"));
        std::fs::create_dir_all(&context).unwrap();
        std::fs::write(context.join("Dockerfile"), dockerfile).unwrap();
        // Buildx writes its activity mark under DOCKER_CONFIG. Point it inside
        // the test's temp dir so a read-only real home cannot fail the build.
        let docker_config = self.dir.path().join("docker-config");
        std::fs::create_dir_all(&docker_config).unwrap();
        let out = Command::new("docker")
            .args([
                "build",
                "--network",
                "none",
                "-t",
                &image_ref,
                &context.display().to_string(),
            ])
            .env("DOCKER_CONFIG", &docker_config)
            .output()
            .expect("docker must run");
        assert!(
            out.status.success(),
            "building {image_ref} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        sha
    }

    /// Starts one service through compose, the way a strategy would, without
    /// going through the tool (used to seed a "previous release is live" state).
    fn seed_running(&self, app: &App, release: &str, port: u16) {
        let compose = self.compose_dir().join("docker-compose.yml");
        let out = Command::new("docker")
            .args([
                "compose",
                "-p",
                &app.compose_project(),
                "-f",
                &compose.display().to_string(),
                "up",
                "-d",
                "--wait",
                &app.compose_svc,
            ])
            .env(&app.env_name, port.to_string())
            .env("IMAGE", app.image_ref(release).unwrap())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "seed failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// A fake nginx: `-t` passes, `-s reload` records and passes.
    fn write_fake_nginx(&self) {
        let fake = self.dir.path().join("nginx");
        std::fs::write(
            &fake,
            "#!/bin/sh\nif [ \"$1\" = \"-t\" ]; then exit 0; fi\nif [ \"$1\" = \"-s\" ]; then echo reload >>\"$NGINX_JOURNAL\"; exit 0; fi\nexit 2\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        // SAFETY: SERIAL holds all tests in this file to one at a time.
        unsafe {
            std::env::set_var("NGINX_JOURNAL", self.dir.path().join("journal"));
        }
    }

    fn ctx(&self) -> DeployContext {
        DeployContext {
            registry_dir: self.registry_dir(),
            state_dir: self.dir.path().join("green"),
            lock_dir: self.dir.path().join("locks"),
            apply_paths: swapdock::apply::ApplyPaths {
                nginx_bin: self.dir.path().join("nginx"),
                main_config: self.compose_dir().join("nginx.conf"),
                target: self.dir.path().join("front-door.conf"),
                pid_file: None,
            },
            drain_secs: 1,
            lock_timeout: Duration::from_secs(30),
        }
    }

    fn run(&self) -> Run {
        let log = swapdock::TraceLog::open(self.dir.path().join("swapdock.jsonl")).unwrap();
        Run::start(
            log,
            RunMode::Live,
            Some("e2e".into()),
            &["swapdock".to_string()],
            swapdock::Redactor::new(),
        )
        .unwrap()
    }

    /// Ports in the back-end range published by any container on this daemon.
    /// Serial execution means only this test's containers can hold them.
    fn published_back_ports() -> HashSet<u16> {
        let out = docker(&["ps", "--format", "{{.Ports}}"]);
        let text = String::from_utf8_lossy(&out.stdout);
        let mut ports = HashSet::new();
        for token in text.split(|c: char| !c.is_ascii_digit()) {
            if let Ok(p) = token.parse::<u16>()
                && (9200..=9201).contains(&p)
            {
                ports.insert(p);
            }
        }
        ports
    }

    /// GETs a port directly until it answers 200.
    fn assert_serves(port: u16) {
        let mut last = String::new();
        for _ in 0..30 {
            match swapdock::health::http_status("127.0.0.1", port, "/") {
                Ok(200) => return,
                Ok(code) => last = format!("HTTP {code}"),
                Err(e) => last = e,
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        panic!("port {port} never served 200 (last: {last})");
    }
}

impl Drop for Estate {
    fn drop(&mut self) {
        // Never leave containers behind, even when an assertion failed. Both the
        // live and the green project, since a failed swap can strand either.
        let compose = self.compose_dir().join("docker-compose.yml");
        let project = self
            .compose_dir()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        for suffix in ["", "-green"] {
            let _ = docker(&[
                "compose",
                "-p",
                &format!("{project}{suffix}"),
                "-f",
                &compose.display().to_string(),
                "down",
                "--volumes",
                "--remove-orphans",
            ]);
        }
        // `run_swap` now uses one Compose project per green release, so the
        // fixed `-green` project above no longer names every container/network.
        // Remove the test's green containers and networks by the names the
        // deploy engine uses for this fixture.
        let project_prefix = format!("{project}-green-");
        let listed = docker(&[
            "ps",
            "-a",
            "--format",
            "{{.Names}} {{.Label \"com.docker.compose.project\"}}",
        ]);
        for line in String::from_utf8_lossy(&listed.stdout).lines() {
            let mut parts = line.split_whitespace();
            let name = parts.next().unwrap_or("");
            let project = parts.next().unwrap_or("");
            if name.starts_with("web-green-")
                || name.starts_with("web-blue-")
                || name.starts_with("b-green-")
                || project.starts_with(&project_prefix)
                || (project.starts_with("e2e") && project.contains("-green-"))
            {
                let _ = docker(&["rm", "-f", name]);
            }
        }
        let networks = docker(&["network", "ls", "--format", "{{.Name}}"]);
        for name in String::from_utf8_lossy(&networks.stdout).lines() {
            if name.starts_with(&project_prefix)
                || (name.starts_with("e2e") && name.contains("-green-"))
            {
                let _ = docker(&["network", "rm", name]);
            }
        }
    }
}

/// Docker's image id for a tag or container.
fn image_id(image_ref: &str) -> String {
    let out = docker(&["image", "inspect", "--format", "{{.Id}}", image_ref]);
    assert!(
        out.status.success(),
        "image inspect {image_ref} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// The image id of the one running container publishing `port`.
fn container_image(port: u16) -> String {
    let filter = format!("publish={port}");
    let out = docker(&["ps", "--format", "{{.Names}}", "--filter", &filter]);
    let names: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    assert_eq!(
        names.len(),
        1,
        "port {port} must have one container: {names:?}"
    );
    let out = docker(&["inspect", "--format", "{{.Image}}", &names[0]]);
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn assert_tag_is(app: &App, release: &str, expected_image_id: &str) {
    let tag = app.image_ref(release).unwrap();
    assert_eq!(
        image_id(&tag),
        expected_image_id,
        "tag {tag} must still name release {release}'s image"
    );
}

/// Removes a container this test created, even on panic.
struct ContainerGuard(String);

impl Drop for ContainerGuard {
    fn drop(&mut self) {
        let _ = docker(&["rm", "-f", &self.0]);
    }
}

/// Milliseconds of the first step with `name` in a trace file.
fn step_ts_ms(trace: &std::path::Path, name: &str) -> i64 {
    let read = swapdock::TraceLog::read(trace).unwrap();
    read.events
        .iter()
        .find_map(|e| match e {
            swapdock::trace::TraceEvent::Step { step, ts_ms, .. } if step == name => Some(*ts_ms),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no {name} step in {}", trace.display()))
}

#[test]
fn swap_deploys_twice_and_retires_the_old() {
    let Some(estate) = Estate::setup("swapapp", Strategy::Swap) else {
        return;
    };
    let mut app = estate.load_app("swapapp");
    let ctx = estate.ctx();
    let release1 = estate.new_release(&app, 1);

    // First swapdock: nothing live, starts on the pair's first port.
    let mut run = estate.run();
    swapdock::run_swap(&mut run, &ctx, &mut app, &release1).unwrap();
    run.finish(swapdock::RunStatus::Ok).unwrap();

    let live1 = app.live_port.unwrap();
    assert_eq!(live1, 9200, "slot 100 starts on 9200");
    Estate::assert_serves(live1);

    // Second swapdock: green on the other port, flip, retire.
    let release2 = estate.new_release(&app, 2);
    let mut run = estate.run();
    swapdock::run_swap(&mut run, &ctx, &mut app, &release2).unwrap();
    run.finish(swapdock::RunStatus::Ok).unwrap();

    let live2 = app.live_port.unwrap();
    assert_eq!(live2, 9201, "the flip must move to the other port");
    assert_eq!(app.release.as_deref(), Some(release2.as_str()));
    assert_eq!(app.old_release.as_deref(), Some(release1.as_str()));
    Estate::assert_serves(live2);

    // The old container is gone: only the live port publishes anything.
    assert_eq!(Estate::published_back_ports(), HashSet::from([live2]));
}

#[test]
fn replace_restarts_on_the_same_port() {
    let Some(estate) = Estate::setup("replaceapp", Strategy::Replace) else {
        return;
    };
    let mut app = estate.load_app("replaceapp");
    let ctx = estate.ctx();

    // Seed a running deployment: record release 1 on 9200 and start it the way
    // the strategy would have, with per-process env (no global mutation).
    let release1 = estate.new_release(&app, 1);
    app.advance(release1.clone(), 9200);
    registry::save_app(&estate.registry_dir(), &app).unwrap();
    let out = Command::new("docker")
        .args([
            "compose",
            "-p",
            &app.compose_project(),
            "-f",
            &estate
                .compose_dir()
                .join("docker-compose.yml")
                .display()
                .to_string(),
            "up",
            "-d",
            "--wait",
            "web",
        ])
        .env("WEB_PORT", "9200")
        .env("IMAGE", app.image_ref(&release1).unwrap())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "seed failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Replace it with release 2 on the same port.
    let release2 = estate.new_release(&app, 2);
    let mut run = estate.run();
    if let Err(e) = swapdock::run_replace(&mut run, &ctx, &mut app, &release2) {
        estate.dump_problems();
        panic!("replace failed: {e:?}");
    }
    run.finish(swapdock::RunStatus::Ok).unwrap();

    assert_eq!(app.live_port, Some(9200), "replace keeps the port");
    assert_eq!(app.release.as_deref(), Some(release2.as_str()));
    assert_eq!(app.old_release.as_deref(), Some(release1.as_str()));
    Estate::assert_serves(9200);
    assert_eq!(Estate::published_back_ports(), HashSet::from([9200]));
}

#[test]
fn rollback_redeploys_the_previous_release() {
    let Some(estate) = Estate::setup("rollbackapp", Strategy::Swap) else {
        return;
    };
    let mut app = estate.load_app("rollbackapp");
    let ctx = estate.ctx();

    let r1 = estate.new_release(&app, 1);
    let mut run = estate.run();
    swapdock::run_swap(&mut run, &ctx, &mut app, &r1).unwrap();
    run.finish(swapdock::RunStatus::Ok).unwrap();

    let r2 = estate.new_release(&app, 2);
    let mut run = estate.run();
    if let Err(e) = swapdock::run_swap(&mut run, &ctx, &mut app, &r2) {
        estate.dump_problems();
        estate.dump_new_state(9201);
        panic!("second swap failed: {e:?}");
    }
    run.finish(swapdock::RunStatus::Ok).unwrap();
    assert_eq!(app.release.as_deref(), Some(r2.as_str()));

    let mut run = estate.run();
    let back = swapdock::rollback(&mut run, &ctx, &mut app).unwrap();
    run.finish(swapdock::RunStatus::Ok).unwrap();

    assert_eq!(back, r1);
    assert_eq!(app.release.as_deref(), Some(r1.as_str()));
    Estate::assert_serves(app.live_port.unwrap());
}

/// Builds an origin repo with one commit and returns its path.
fn git_origin(dir: &std::path::Path, name: &str) -> PathBuf {
    let origin = dir.join(format!("{name}-origin"));
    std::fs::create_dir_all(&origin).unwrap();
    let run = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(&origin)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(&["init", "-b", "main"]);
    run(&["config", "user.email", "test@localhost"]);
    run(&["config", "user.name", "test"]);
    run(&["commit", "--allow-empty", "-m", "init"]);
    origin
}

#[test]
fn sync_clones_fetches_and_fast_forwards() {
    let Some(_e) = Estate::setup("syncapp", Strategy::Swap) else {
        return;
    };
    // Reuse the serial lock via a fresh estate guard: setup already holds it.
    let dir = TempDir::new().unwrap();
    let origin = git_origin(dir.path(), "syncapp");
    let repo = dir.path().join("checkout");

    let log = swapdock::TraceLog::open(dir.path().join("swapdock.jsonl")).unwrap();
    let mut run = Run::start(
        log,
        RunMode::Live,
        None,
        &["swapdock".to_string()],
        swapdock::Redactor::new(),
    )
    .unwrap();

    let mut app = swapdock::registry::App {
        name: "syncapp".into(),
        kind: Kind::Container,
        strategy: Strategy::Swap,
        hostnames: vec![],
        listen: vec![],
        front_port: 18999,
        slot: 100,
        live_port: None,
        old_port: None,
        writes_state: false,
        image_repo: None,
        registry: None,
        release: None,
        old_release: None,
        build_host: None,
        root: None,
        spa: false,
        health_url: None,
        compose_dir: dir.path().to_path_buf(),
        compose_svc: "web".into(),
        env_name: "WEB_PORT".into(),
        git_remote: None,
        branch: Some("main".into()),
        repo: None,
        build_repo: None,
        compose_override: None,
        public_url: None,
        backup: None,
    };
    // Pre-clone from a file URL: no network, same semantics. sync must then
    // fast-forward (short-form git_remote is GitHub-only, so it stays unset).
    app.repo = Some(repo.clone());
    let out = Command::new("git")
        .args(["clone", "--depth", "1", "--branch", "main"])
        .arg(&origin)
        .arg(&repo)
        .output()
        .unwrap();
    assert!(out.status.success());

    // A new commit on origin: sync must pick it up.
    let out = Command::new("git")
        .args(["commit", "--allow-empty", "-m", "second"])
        .current_dir(&origin)
        .output()
        .unwrap();
    assert!(out.status.success());

    let sha = swapdock::deploy::sync_repo(&mut run, &app).unwrap();
    assert_eq!(sha.len(), 40, "full commit name: {sha}");

    let head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&head.stdout).trim(), sha);

    // Dirty checkout refuses.
    std::fs::write(repo.join("uncommitted.txt"), "x").unwrap();
    let err = swapdock::deploy::sync_repo(&mut run, &app).unwrap_err();
    assert!(err.to_string().contains("uncommitted"), "{err}");
}

#[test]
fn build_local_clones_checks_out_and_builds() {
    if !daemon() {
        return;
    }
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new().unwrap();

    // A real source repo: Dockerfile plus a commit to build.
    let origin = dir.path().join("origin");
    std::fs::create_dir_all(&origin).unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(&origin)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-b", "main"]);
    git(&["config", "user.email", "test@localhost"]);
    git(&["config", "user.name", "test"]);
    std::fs::write(
        origin.join("Dockerfile"),
        "FROM nginx:alpine\nRUN echo built > /built\n",
    )
    .unwrap();
    git(&["add", "Dockerfile"]);
    git(&["commit", "-m", "build me"]);
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&origin)
        .output()
        .unwrap();
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    assert_eq!(sha.len(), 40);

    let app = App {
        name: "buildapp".into(),
        kind: Kind::Container,
        strategy: Strategy::Swap,
        hostnames: vec![],
        listen: vec![],
        front_port: 18999,
        slot: 100,
        live_port: None,
        old_port: None,
        writes_state: false,
        image_repo: Some("e2e-buildapp".into()),
        registry: Some(ImageRegistry::Local),
        release: None,
        old_release: None,
        build_host: Some("local".into()),
        root: None,
        spa: false,
        health_url: None,
        compose_dir: dir.path().to_path_buf(),
        compose_svc: "web".into(),
        env_name: "WEB_PORT".into(),
        git_remote: Some(format!("file://{}", origin.display())),
        branch: Some("main".into()),
        repo: None,
        build_repo: None,
        compose_override: None,
        public_url: None,
        backup: None,
    };

    let log = swapdock::TraceLog::open(dir.path().join("swapdock.jsonl")).unwrap();
    let mut run = Run::start(
        log,
        RunMode::Live,
        None,
        &["swapdock".to_string()],
        swapdock::Redactor::new(),
    )
    .unwrap();

    let built =
        swapdock::builder::build_local(&mut run, &app, &sha, &dir.path().join("work")).unwrap();
    run.finish(swapdock::RunStatus::Ok).unwrap();

    assert_eq!(built.release, sha);
    assert_eq!(built.image_ref, format!("e2e-buildapp:{sha}"));
    assert!(
        docker(&["image", "inspect", &built.image_ref])
            .status
            .success(),
        "the image must exist in the daemon"
    );
    assert!(
        !dir.path()
            .join("work")
            .join(format!("buildapp-{sha}-build"))
            .exists(),
        "the build context must be cleaned up"
    );

    // The trace shows clone, checkout, build — and no push for a local image.
    let read = swapdock::TraceLog::read(dir.path().join("swapdock.jsonl")).unwrap();
    let steps: Vec<&str> = read
        .events
        .iter()
        .filter_map(|e| match e {
            swapdock::trace::TraceEvent::Step { step, .. } => Some(step.as_str()),
            _ => None,
        })
        .collect();
    for expected in ["build-clone", "build-checkout", "docker-build"] {
        assert!(steps.contains(&expected), "{steps:?}");
    }
    assert!(
        !steps.contains(&"docker-push"),
        "local images are not pushed: {steps:?}"
    );

    docker(&["rmi", &built.image_ref]);
}

#[test]
fn swap_on_a_stateful_app_is_refused_before_anything_starts() {
    let Some(estate) = Estate::setup("badapp", Strategy::Swap) else {
        return;
    };
    // Flip the flag on disk: the engine must refuse what the validator refuses.
    let mut app = estate.load_app("badapp");
    app.writes_state = true;
    registry::save_app(&estate.registry_dir(), &app).unwrap();

    let ctx = estate.ctx();
    let release = estate.new_release(&app, 1);
    let mut run = estate.run();
    let err = swapdock::run_swap(&mut run, &ctx, &mut app, &release).unwrap_err();
    assert!(
        matches!(err, swapdock::DeployError::ValidationFailed(_)),
        "expected a validation refusal, got {err:?}"
    );

    // Nothing started: no container publishes anything.
    assert!(Estate::published_back_ports().is_empty());
    // And the trace says where it stopped.
    let read = swapdock::TraceLog::read(estate.dir.path().join("swapdock.jsonl")).unwrap();
    assert!(read.events.iter().any(|e| matches!(
        e,
        swapdock::trace::TraceEvent::Step { step, .. } if step == "refuse-broken-estate"
    )));
}

#[test]
fn rollback_preserves_distinct_release_tags_across_two_rollbacks() {
    let Some(estate) = Estate::setup("tagapp", Strategy::Swap) else {
        return;
    };
    let mut app = estate.load_app("tagapp");
    let ctx = estate.ctx();

    let r1 = estate.new_release(&app, 1);
    let r2 = estate.new_distinct_release(
        &app,
        2,
        "FROM nginx:alpine\nRUN echo release-two > /release-two\n",
    );
    let image1 = image_id(&app.image_ref(&r1).unwrap());
    let image2 = image_id(&app.image_ref(&r2).unwrap());
    assert_ne!(image1, image2, "the test needs two distinct image ids");

    // Forward deploy r1, then r2. r1 must remain tagged with its own image.
    let mut run = estate.run();
    swapdock::run_swap(&mut run, &ctx, &mut app, &r1).unwrap();
    run.finish(swapdock::RunStatus::Ok).unwrap();
    assert_tag_is(&app, &r1, &image1);
    assert_eq!(container_image(9200), image1);

    let mut run = estate.run();
    swapdock::run_swap(&mut run, &ctx, &mut app, &r2).unwrap();
    run.finish(swapdock::RunStatus::Ok).unwrap();
    assert_eq!(app.release.as_deref(), Some(r2.as_str()));
    assert_tag_is(&app, &r1, &image1);
    assert_tag_is(&app, &r2, &image2);
    assert_eq!(
        container_image(9201),
        image2,
        "r2 must be the running image"
    );

    // First rollback: r1 is the target. The old implementation retagged the
    // live r2 image as r1 here, corrupting the very tag being deployed.
    let mut run = estate.run();
    let back = swapdock::rollback(&mut run, &ctx, &mut app).unwrap();
    run.finish(swapdock::RunStatus::Ok).unwrap();
    assert_eq!(back, r1);
    assert_eq!(app.release.as_deref(), Some(r1.as_str()));
    assert_tag_is(&app, &r1, &image1);
    assert_tag_is(&app, &r2, &image2);
    assert_eq!(
        container_image(9200),
        image1,
        "rollback must run r1's image"
    );

    // Second rollback: r2. Two consecutive rollbacks must stay safe.
    let mut run = estate.run();
    let back = swapdock::rollback(&mut run, &ctx, &mut app).unwrap();
    run.finish(swapdock::RunStatus::Ok).unwrap();
    assert_eq!(back, r2);
    assert_eq!(app.release.as_deref(), Some(r2.as_str()));
    assert_tag_is(&app, &r1, &image1);
    assert_tag_is(&app, &r2, &image2);
    assert_eq!(
        container_image(9201),
        image2,
        "the second rollback must run r2's image"
    );
}

#[test]
fn a_stale_app_snapshot_is_reloaded_under_the_lock() {
    let Some(estate) = Estate::setup("staleapp", Strategy::Swap) else {
        return;
    };
    let mut app = estate.load_app("staleapp");
    let ctx = estate.ctx();

    let r1 = estate.new_release(&app, 1);
    let mut run = estate.run();
    swapdock::run_swap(&mut run, &ctx, &mut app, &r1).unwrap();
    run.finish(swapdock::RunStatus::Ok).unwrap();
    assert_eq!(app.live_port, Some(9200));

    // This snapshot is what a second process loaded before waiting for the lock.
    let mut stale = app.clone();

    let r2 = estate.new_release(&app, 2);
    let mut run = estate.run();
    swapdock::run_swap(&mut run, &ctx, &mut app, &r2).unwrap();
    run.finish(swapdock::RunStatus::Ok).unwrap();
    assert_eq!(app.live_port, Some(9201));

    // Deploying with the stale snapshot must use the fresh live_port (9201) and
    // alternate to 9200, not race the serving container on 9201.
    let r3 = estate.new_release(&app, 3);
    let mut run = estate.run();
    swapdock::run_swap(&mut run, &ctx, &mut stale, &r3).unwrap();
    run.finish(swapdock::RunStatus::Ok).unwrap();

    assert_eq!(stale.live_port, Some(9200), "the flip must use fresh state");
    let fresh = estate.load_app("staleapp");
    assert_eq!(fresh.live_port, Some(9200));
    assert_eq!(fresh.release.as_deref(), Some(r3.as_str()));
    assert_eq!(fresh.old_release.as_deref(), Some(r2.as_str()));
    assert_eq!(Estate::published_back_ports(), HashSet::from([9200]));
    Estate::assert_serves(9200);
}

#[test]
fn swap_refuses_named_volumes_before_starting_the_green_container() {
    let Some(estate) = Estate::setup("volapp", Strategy::Swap) else {
        return;
    };
    // Compose v2 preferred name. The resolver must pick this over the
    // docker-compose.yml written by setup.
    std::fs::write(
        estate.compose_dir().join("compose.yaml"),
        "services:\n  web:\n    image: ${IMAGE:-nginx:alpine}\n    ports:\n      - \"127.0.0.1:${WEB_PORT}:80\"\n    volumes:\n      - uploads:/data/uploads\n    healthcheck:\n      test: [\"CMD\", \"wget\", \"-q\", \"-O\", \"/dev/null\", \"http://127.0.0.1/\"]\n      interval: 2s\n      timeout: 2s\n      retries: 2\n      start_period: 3s\nvolumes:\n  uploads: {}\n",
    )
    .unwrap();

    let app = estate.load_app("volapp");
    let ctx = estate.ctx();
    let release = estate.new_release(&app, 1);
    let mut run = estate.run();
    let err = swapdock::run_swap(&mut run, &ctx, &mut app.clone(), &release).unwrap_err();
    run.finish(swapdock::RunStatus::Failed).unwrap();

    assert!(
        matches!(err, swapdock::DeployError::SwapUnsafe { .. }),
        "expected a preflight refusal, got {err:?}"
    );
    let message = err.to_string();
    assert!(message.contains("uploads"), "{message}");
    assert!(message.contains("strategy = \"replace\""), "{message}");
    assert!(message.contains("bind mounts"), "{message}");

    // Nothing was started.
    assert!(Estate::published_back_ports().is_empty());

    let read = swapdock::TraceLog::read(estate.dir.path().join("swapdock.jsonl")).unwrap();
    assert!(
        read.events.iter().any(|e| matches!(
            e,
            swapdock::trace::TraceEvent::Step {
                step,
                status: swapdock::trace::StepStatus::Error,
                ..
            } if step == "swap-preflight"
        )),
        "the refusal must be visible in the trace"
    );
}

#[test]
fn replace_failure_restarts_the_previous_release_through_the_health_gate() {
    let Some(estate) = Estate::setup("replacerb", Strategy::Replace) else {
        return;
    };
    let mut app = estate.load_app("replacerb");
    let ctx = estate.ctx();

    let r1 = estate.new_release(&app, 1);
    app.advance(r1.clone(), 9200);
    registry::save_app(&estate.registry_dir(), &app).unwrap();
    estate.seed_running(&app, &r1, 9200);
    let image1 = image_id(&app.image_ref(&r1).unwrap());

    // This image starts and immediately exits, so start_live or the health gate
    // must fail and the automatic rollback must take over.
    let r2 = estate.new_distinct_release(&app, 2, "FROM nginx:alpine\nCMD [\"false\"]\n");

    let mut run = estate.run();
    let err = swapdock::run_replace(&mut run, &ctx, &mut app, &r2).unwrap_err();
    run.finish(swapdock::RunStatus::Failed).unwrap();

    assert!(
        matches!(err, swapdock::DeployError::ReplaceFailed { .. }),
        "expected an auto-rollback error, got {err:?}"
    );
    let message = err.to_string();
    assert!(
        message.contains(&r1),
        "must name the restored release: {message}"
    );
    assert!(
        message.contains(&r2),
        "must name the failed release: {message}"
    );
    assert!(message.contains("restored"), "{message}");

    // The registry was not committed and the previous image is serving again.
    assert_eq!(app.release.as_deref(), Some(r1.as_str()));
    assert_eq!(app.live_port, Some(9200));
    Estate::assert_serves(9200);
    assert_eq!(
        container_image(9200),
        image1,
        "the old release's image must be back on the port"
    );

    let read = swapdock::TraceLog::read(estate.dir.path().join("swapdock.jsonl")).unwrap();
    assert!(
        read.events.iter().any(|e| matches!(
            e,
            swapdock::trace::TraceEvent::Step {
                step,
                status: swapdock::trace::StepStatus::Ok,
                ..
            } if step == "replace-rollback"
        )),
        "the rollback must be recorded"
    );
}

#[test]
fn a_failed_adopt_restarts_the_legacy_container() {
    let Some(estate) = Estate::setup("adoptapp", Strategy::Replace) else {
        return;
    };
    let legacy = format!("e2e-legacy-{}", std::process::id());
    let _guard = ContainerGuard(legacy.clone());
    let started = docker(&[
        "run",
        "-d",
        "--name",
        &legacy,
        "-p",
        "127.0.0.1:18999:80",
        "nginx:alpine",
    ]);
    assert!(
        started.status.success(),
        "could not start the legacy container: {}",
        String::from_utf8_lossy(&started.stderr)
    );

    let mut app = estate.load_app("adoptapp");
    // A hostname is what makes probe_front actually dial the front port; the
    // fake nginx never listens there, so the cutover fails after the health gate.
    app.hostnames = vec!["adopt.example.com".into()];
    registry::save_app(&estate.registry_dir(), &app).unwrap();
    let ctx = estate.ctx();
    let release = estate.new_release(&app, 1);

    // The fake nginx never listens on the front port, so the cutover passes the
    // container health gate and then fails at the front probe. That is exactly
    // the window where the legacy container must come back.
    let mut run = estate.run();
    let err =
        swapdock::deploy::adopt(&mut run, &ctx, &mut app, &release, None, Some(80)).unwrap_err();
    run.finish(swapdock::RunStatus::Failed).unwrap();

    let message = err.to_string();
    assert!(
        message.contains(&legacy),
        "the error must name the legacy container: {message}"
    );
    assert!(
        message.contains("restarted"),
        "the error must say the legacy container was restarted: {message}"
    );

    let running = docker(&["inspect", "--format", "{{.State.Running}}", &legacy]);
    assert_eq!(
        String::from_utf8_lossy(&running.stdout).trim(),
        "true",
        "the legacy container must be running again"
    );
    Estate::assert_serves(18999);

    // The registry was not committed: the next attempt is still a first adopt.
    let fresh = estate.load_app("adoptapp");
    assert_eq!(fresh.live_port, None);
    assert_eq!(fresh.release, None);
    assert_eq!(fresh.compose_override, None);

    let read = swapdock::TraceLog::read(estate.dir.path().join("swapdock.jsonl")).unwrap();
    assert!(
        read.events.iter().any(|e| matches!(
            e,
            swapdock::trace::TraceEvent::Step {
                step,
                status: swapdock::trace::StepStatus::Ok,
                ..
            } if step == "adopt-restore-legacy"
        )),
        "the restart must be recorded"
    );
    assert!(
        !read.events.iter().any(|e| matches!(
            e,
            swapdock::trace::TraceEvent::Step { step, .. } if step == "remove-legacy"
        )),
        "the legacy container must not be removed on failure"
    );
}

#[test]
fn two_app_deploys_are_serialized_by_the_estate_lock() {
    let Some(estate) = Estate::setup("concbase", Strategy::Swap) else {
        return;
    };
    // Two independent services in one Compose project: the shape that races on
    // the rendered registry when the estate lock is missing.
    std::fs::write(
        estate.compose_dir().join("compose.yaml"),
        "services:\n  a:\n    image: ${IMAGE:-nginx:alpine}\n    ports:\n      - \"127.0.0.1:${WEB_PORT}:80\"\n    healthcheck:\n      test: [\"CMD\", \"wget\", \"-q\", \"-O\", \"/dev/null\", \"http://127.0.0.1/\"]\n      interval: 2s\n      timeout: 2s\n      retries: 2\n      start_period: 3s\n  b:\n    image: ${IMAGE:-nginx:alpine}\n    ports:\n      - \"127.0.0.1:${WEB_PORT}:80\"\n    healthcheck:\n      test: [\"CMD\", \"wget\", \"-q\", \"-O\", \"/dev/null\", \"http://127.0.0.1/\"]\n      interval: 2s\n      timeout: 2s\n      retries: 2\n      start_period: 3s\n",
    )
    .unwrap();

    let base = estate.load_app("concbase");
    std::fs::remove_file(estate.registry_dir().join("concbase.toml")).unwrap();

    let mut app_a = base.clone();
    app_a.name = "appa".into();
    app_a.compose_svc = "a".into();
    app_a.slot = 100;
    app_a.front_port = 18999;
    app_a.image_repo = Some("e2e-appa".into());
    registry::save_app(&estate.registry_dir(), &app_a).unwrap();

    let mut app_b = base;
    app_b.name = "appb".into();
    app_b.compose_svc = "b".into();
    app_b.slot = 101;
    app_b.front_port = 18998;
    app_b.image_repo = Some("e2e-appb".into());
    registry::save_app(&estate.registry_dir(), &app_b).unwrap();

    let release_a = estate.new_release(&app_a, 1);
    let release_b = estate.new_release(&app_b, 2);
    let ctx = estate.ctx();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));

    let spawn = |mut app: App,
                 release: String,
                 trace: PathBuf,
                 ctx: DeployContext,
                 barrier: std::sync::Arc<std::sync::Barrier>| {
        std::thread::spawn(move || {
            let log = swapdock::TraceLog::open(trace).unwrap();
            let mut run = Run::start(
                log,
                RunMode::Live,
                Some(app.name.clone()),
                &["swapdock".to_string()],
                swapdock::Redactor::new(),
            )
            .unwrap();
            barrier.wait();
            let result = swapdock::run_swap(&mut run, &ctx, &mut app, &release);
            let status = if result.is_ok() {
                swapdock::RunStatus::Ok
            } else {
                swapdock::RunStatus::Failed
            };
            let _ = run.finish(status);
            result
        })
    };

    let trace_a = estate.dir.path().join("trace-appa.jsonl");
    let trace_b = estate.dir.path().join("trace-appb.jsonl");
    let ha = spawn(
        app_a,
        release_a.clone(),
        trace_a.clone(),
        ctx.clone(),
        barrier.clone(),
    );
    let hb = spawn(
        app_b,
        release_b.clone(),
        trace_b.clone(),
        ctx.clone(),
        barrier,
    );
    ha.join().unwrap().expect("appa deploy failed");
    hb.join().unwrap().expect("appb deploy failed");

    // The first deploy commits while the second is still blocked on the lock:
    // its commit precedes the other's preflight (which is after the lock).
    let a_commit = step_ts_ms(&trace_a, "registry-commit");
    let b_commit = step_ts_ms(&trace_b, "registry-commit");
    let a_start = step_ts_ms(&trace_a, "swap-preflight");
    let b_start = step_ts_ms(&trace_b, "swap-preflight");
    assert!(
        a_commit <= b_start || b_commit <= a_start,
        "the two deploys overlapped (a: preflight {a_start}, commit {a_commit}; \
         b: preflight {b_start}, commit {b_commit})"
    );

    // Both final live ports are in the one rendered config, and both apps serve.
    let app_a = estate.load_app("appa");
    let app_b = estate.load_app("appb");
    let a_live = app_a.live_port.unwrap();
    let b_live = app_b.live_port.unwrap();
    assert_ne!(a_live, b_live);
    let rendered = std::fs::read_to_string(&ctx.apply_paths.target).unwrap();
    assert!(
        rendered.contains(&format!("proxy_pass http://127.0.0.1:{a_live};")),
        "{rendered}"
    );
    assert!(
        rendered.contains(&format!("proxy_pass http://127.0.0.1:{b_live};")),
        "{rendered}"
    );
    Estate::assert_serves(a_live);
    Estate::assert_serves(b_live);
}
