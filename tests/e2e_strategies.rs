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
use std::sync::Mutex;
use std::time::Duration;

use deploy::registry::{self, App, ImageRegistry, Kind, Strategy};
use deploy::{Ctx, Run, RunMode};
use tempfile::TempDir;

/// Serializes this file's tests. See the module docs for why.
static SERIAL: Mutex<()> = Mutex::new(());

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
            // nginx:alpine serves / with 200.
            health_url: Some("http://127.0.0.1/".into()),
            compose_dir: projdir.to_path_buf(),
            compose_svc: "web".into(),
            env_name: "WEB_PORT".into(),
            git_remote: None,
            branch: None,
            repo: None,
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
        problems.extend(deploy::validator::validate(
            &apps,
            &deploy::validator::TunnelRoutes::new(),
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
        problems.extend(deploy::validator::validate(
            &loaded.sorted(),
            &deploy::validator::TunnelRoutes::new(),
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

    fn ctx(&self) -> Ctx {
        Ctx {
            registry_dir: self.registry_dir(),
            state_dir: self.dir.path().join("green"),
            lock_dir: self.dir.path().join("locks"),
            apply_paths: deploy::apply::ApplyPaths {
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
        let log = deploy::TraceLog::open(self.dir.path().join("deploy.jsonl")).unwrap();
        Run::start(
            log,
            RunMode::Live,
            Some("e2e".into()),
            &["deploy".to_string()],
            deploy::Redactor::new(),
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
            match deploy::health::http_status("127.0.0.1", port, "/") {
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
    }
}

#[test]
fn swap_deploys_twice_and_retires_the_old() {
    let Some(estate) = Estate::setup("swapapp", Strategy::Swap) else {
        return;
    };
    let mut app = estate.load_app("swapapp");
    let ctx = estate.ctx();
    let release1 = estate.new_release(&app, 1);

    // First deploy: nothing live, starts on the pair's first port.
    let mut run = estate.run();
    deploy::run_swap(&mut run, &ctx, &mut app, &release1).unwrap();
    run.finish(deploy::RunStatus::Ok).unwrap();

    let live1 = app.live_port.unwrap();
    assert_eq!(live1, 9200, "slot 100 starts on 9200");
    Estate::assert_serves(live1);

    // Second deploy: green on the other port, flip, retire.
    let release2 = estate.new_release(&app, 2);
    let mut run = estate.run();
    deploy::run_swap(&mut run, &ctx, &mut app, &release2).unwrap();
    run.finish(deploy::RunStatus::Ok).unwrap();

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
    if let Err(e) = deploy::run_replace(&mut run, &ctx, &mut app, &release2) {
        estate.dump_problems();
        panic!("replace failed: {e:?}");
    }
    run.finish(deploy::RunStatus::Ok).unwrap();

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
    deploy::run_swap(&mut run, &ctx, &mut app, &r1).unwrap();
    run.finish(deploy::RunStatus::Ok).unwrap();

    let r2 = estate.new_release(&app, 2);
    let mut run = estate.run();
    if let Err(e) = deploy::run_swap(&mut run, &ctx, &mut app, &r2) {
        estate.dump_problems();
        estate.dump_new_state(9201);
        panic!("second swap failed: {e:?}");
    }
    run.finish(deploy::RunStatus::Ok).unwrap();
    assert_eq!(app.release.as_deref(), Some(r2.as_str()));

    let mut run = estate.run();
    let back = deploy::rollback(&mut run, &ctx, &mut app).unwrap();
    run.finish(deploy::RunStatus::Ok).unwrap();

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

    let log = deploy::TraceLog::open(dir.path().join("deploy.jsonl")).unwrap();
    let mut run = Run::start(
        log,
        RunMode::Live,
        None,
        &["deploy".to_string()],
        deploy::Redactor::new(),
    )
    .unwrap();

    let mut app = deploy::registry::App {
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
        health_url: None,
        compose_dir: dir.path().to_path_buf(),
        compose_svc: "web".into(),
        env_name: "WEB_PORT".into(),
        git_remote: None,
        branch: Some("main".into()),
        repo: None,
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

    let sha = deploy::deploy::sync_repo(&mut run, &app).unwrap();
    assert_eq!(sha.len(), 40, "full commit name: {sha}");

    let head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repo)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&head.stdout).trim(), sha);

    // Dirty checkout refuses.
    std::fs::write(repo.join("uncommitted.txt"), "x").unwrap();
    let err = deploy::deploy::sync_repo(&mut run, &app).unwrap_err();
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
        health_url: None,
        compose_dir: dir.path().to_path_buf(),
        compose_svc: "web".into(),
        env_name: "WEB_PORT".into(),
        git_remote: Some(format!("file://{}", origin.display())),
        branch: Some("main".into()),
        repo: None,
    };

    let log = deploy::TraceLog::open(dir.path().join("deploy.jsonl")).unwrap();
    let mut run = Run::start(
        log,
        RunMode::Live,
        None,
        &["deploy".to_string()],
        deploy::Redactor::new(),
    )
    .unwrap();

    let built =
        deploy::builder::build_local(&mut run, &app, &sha, &dir.path().join("work")).unwrap();
    run.finish(deploy::RunStatus::Ok).unwrap();

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
    let read = deploy::TraceLog::read(dir.path().join("deploy.jsonl")).unwrap();
    let steps: Vec<&str> = read
        .events
        .iter()
        .filter_map(|e| match e {
            deploy::trace::TraceEvent::Step { step, .. } => Some(step.as_str()),
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
    let err = deploy::run_swap(&mut run, &ctx, &mut app, &release).unwrap_err();
    assert!(
        matches!(err, deploy::DeployError::ValidationFailed(_)),
        "expected a validation refusal, got {err:?}"
    );

    // Nothing started: no container publishes anything.
    assert!(Estate::published_back_ports().is_empty());
    // And the trace says where it stopped.
    let read = deploy::TraceLog::read(estate.dir.path().join("deploy.jsonl")).unwrap();
    assert!(read.events.iter().any(|e| matches!(
        e,
        deploy::trace::TraceEvent::Step { step, .. } if step == "refuse-broken-estate"
    )));
}
