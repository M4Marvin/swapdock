//! Failure-injection tests that need no Docker daemon.
//!
//! The deploy engine reaches the operating system only through `exec::Run::exec`,
//! which resolves `docker` through `PATH`. A fake `docker` on a private `PATH` is
//! therefore enough to prove what the engine does when a command fails without
//! starting any real container. This file runs as its own test binary, so the
//! process-global `PATH` change cannot leak into another test binary.

use std::path::{Path, PathBuf};
use std::time::Duration;

use swapdock::apply::ApplyPaths;
use swapdock::registry::{self, App, ImageRegistry, Kind, Strategy};
use swapdock::{DeployContext, Run, RunMode};

/// Restores `PATH` when the test ends, even on panic.
struct PathGuard {
    original: Option<std::ffi::OsString>,
}

impl PathGuard {
    fn prepend(dir: &Path) -> Self {
        let original = std::env::var_os("PATH");
        let mut value = dir.as_os_str().to_os_string();
        if let Some(old) = &original {
            value.push(":");
            value.push(old);
        }
        // SAFETY: this test binary has exactly one test and runs it on the main
        // test thread; no other thread reads PATH concurrently.
        unsafe { std::env::set_var("PATH", value) };
        Self { original }
    }
}

impl Drop for PathGuard {
    fn drop(&mut self) {
        // SAFETY: same as above.
        match &self.original {
            Some(path) => unsafe { std::env::set_var("PATH", path) },
            None => unsafe { std::env::remove_var("PATH") },
        }
    }
}

fn write_executable(path: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A Docker CLI whose `ps` always finds a container and whose `stop` always
/// fails. `image inspect` succeeds so the local-image check and the tag probe
/// do not fail earlier.
const FAKE_DOCKER: &str = r#"#!/bin/sh
cmd="$1"
case "$cmd" in
  ps)
    echo "stale-container"
    exit 0
    ;;
  image)
    echo "sha256:111111111111"
    exit 0
    ;;
  inspect)
    echo "sha256:111111111111"
    exit 0
    ;;
  tag)
    exit 0
    ;;
  stop)
    echo "cannot stop stale-container" >&2
    exit 1
    ;;
  rm)
    exit 0
    ;;
  *)
    exit 0
    ;;
esac
"#;

#[test]
fn a_failed_stop_is_fatal_and_never_commits_the_release() {
    let dir = tempfile::TempDir::new().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    write_executable(&bin.join("docker"), FAKE_DOCKER);
    let _path = PathGuard::prepend(&bin);

    let compose_dir = dir.path().join("compose");
    std::fs::create_dir_all(&compose_dir).unwrap();
    std::fs::write(compose_dir.join("compose.yaml"), "services: {}\n").unwrap();

    let registry_dir = dir.path().join("apps");
    std::fs::create_dir_all(&registry_dir).unwrap();
    let mut app = App {
        name: "failapp".into(),
        kind: Kind::Container,
        strategy: Strategy::Replace,
        hostnames: vec![],
        listen: vec![],
        front_port: 8001,
        slot: 100,
        live_port: Some(9200),
        old_port: None,
        writes_state: true,
        image_repo: Some("e2e-failapp".into()),
        registry: Some(ImageRegistry::Local),
        release: Some("aaaa111".into()),
        old_release: None,
        build_host: Some("local".into()),
        root: None,
        spa: false,
        health_url: Some("http://127.0.0.1/".into()),
        compose_dir: compose_dir.clone(),
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
    registry::save_app(&registry_dir, &app).unwrap();

    let fake_nginx = dir.path().join("nginx");
    write_executable(&fake_nginx, "#!/bin/sh\nexit 0\n");
    let main_config = dir.path().join("nginx.conf");
    std::fs::write(&main_config, "# main\n").unwrap();
    let target = dir.path().join("front-door.conf");

    let ctx = DeployContext {
        registry_dir: registry_dir.clone(),
        state_dir: dir.path().join("green"),
        lock_dir: dir.path().join("locks"),
        apply_paths: ApplyPaths {
            nginx_bin: fake_nginx,
            main_config,
            target: target.clone(),
            pid_file: None,
        },
        drain_secs: 1,
        lock_timeout: Duration::from_secs(5),
    };

    let trace: PathBuf = dir.path().join("swapdock.jsonl");
    let log = swapdock::TraceLog::open(&trace).unwrap();
    let mut run = Run::start(
        log,
        RunMode::Live,
        Some(app.name.clone()),
        &["swapdock".to_string()],
        swapdock::Redactor::new(),
    )
    .unwrap();

    let err = swapdock::run_replace(&mut run, &ctx, &mut app, "bbbb222").unwrap_err();
    run.finish(swapdock::RunStatus::Failed).unwrap();

    assert!(
        matches!(err, swapdock::DeployError::StopFailed { .. }),
        "a failed docker stop must be fatal, got {err:?}"
    );
    assert!(
        err.to_string().contains("stale-container"),
        "the error must name the container: {err}"
    );

    // The registry was not advanced: the app still points at the old release.
    let loaded = registry::load_dir(&registry_dir).unwrap();
    let saved = loaded
        .sorted()
        .into_iter()
        .find(|a| a.name == "failapp")
        .unwrap();
    assert_eq!(saved.release.as_deref(), Some("aaaa111"));
    assert_eq!(saved.live_port, Some(9200));

    // The failed stop is recorded as an error, and no start of the new release
    // was attempted after it.
    let read = swapdock::TraceLog::read(&trace).unwrap();
    let mut saw_stop_error = false;
    for event in &read.events {
        if let swapdock::trace::TraceEvent::Step { step, status, .. } = event {
            if step == "docker-stop" && *status == swapdock::trace::StepStatus::Error {
                saw_stop_error = true;
            }
            assert!(
                !(step == "compose-up" && *status == swapdock::trace::StepStatus::Ok),
                "nothing may start after the failed stop: {event:?}"
            );
        }
    }
    assert!(saw_stop_error, "the failed stop must be in the trace");
}
