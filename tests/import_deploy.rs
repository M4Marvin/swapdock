//! Import a Compose service, then deploy it through the real swap machinery.
//!
//! This is the first-deploy half of G3.1: `swapdock import` writes the registry
//! and override, and `deploy::deploy` starts the imported service, gates its
//! health, applies the fake nginx config, probes the front door and commits.
//! Docker is required; the test skips cleanly without it.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use swapdock::apply::ApplyPaths;
use swapdock::deploy::{self, DeployContext};
use swapdock::import::{self, Options};
use swapdock::registry;
use swapdock::trace::{Run, RunMode, RunStatus, TraceLog};
use tempfile::TempDir;

static SERIAL: Mutex<()> = Mutex::new(());

fn docker(argv: &[&str]) -> std::process::Output {
    Command::new("docker")
        .args(argv)
        .output()
        .expect("docker must run")
}

fn daemon() -> bool {
    let ok = docker(&["info"]).status.success();
    if !ok {
        eprintln!("SKIP import_deploy: no Docker daemon");
    }
    ok
}

fn free_port() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.local_addr().unwrap().port()
}

#[test]
fn an_imported_compose_app_deploys_and_commits() {
    if !daemon() {
        return;
    }
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new().unwrap();
    let project = format!("importdeploy{}", std::process::id());
    let compose_dir = dir.path().join(&project);
    std::fs::create_dir_all(&compose_dir).unwrap();
    std::fs::write(
        compose_dir.join("compose.yaml"),
        "services:\n  web:\n    image: nginx:alpine\n    ports:\n      - \"127.0.0.1:3000:80\"\n    healthcheck:\n      test: [\"CMD\", \"wget\", \"-q\", \"-O\", \"/dev/null\", \"http://127.0.0.1/\"]\n      interval: 2s\n      timeout: 2s\n      retries: 3\n      start_period: 2s\n",
    )
    .unwrap();

    let release = format!("{:07x}", std::process::id());
    assert!(
        docker(&["tag", "nginx:alpine", &format!("nginx:{release}")])
            .status
            .success()
    );

    let registry_dir = dir.path().join("apps");
    let state_dir = dir.path().join("green");
    let opts = Options {
        compose_dir: compose_dir.clone(),
        name: Some("importdeploy".into()),
        front_port: Some(free_port()),
        slot: Some(0),
        domain: Some("importdeploy.test".into()),
        state_dir: state_dir.clone(),
        ..Options::default()
    };
    let report = import::run(&opts, &registry_dir, None).expect("import");
    assert!(report.app.name == "importdeploy");

    let nginx = dir.path().join("fake-nginx");
    std::fs::write(
        &nginx,
        "#!/bin/sh\ncase \"$1\" in -t|-s) exit 0;; esac\nexit 0\n",
    )
    .unwrap();
    std::fs::set_permissions(&nginx, std::fs::Permissions::from_mode(0o755)).unwrap();
    let main_config = dir.path().join("nginx.conf");
    std::fs::write(&main_config, "events {}\nhttp {}\n").unwrap();
    let target = dir.path().join("front-door.conf");
    let trace = dir.path().join("swapdock.jsonl");

    // The imported front port gets a tiny HTTP server for probe_front.
    let listener = TcpListener::bind(("127.0.0.1", opts.front_port.unwrap())).unwrap();
    listener.set_nonblocking(true).unwrap();
    let stop_server = Arc::new(AtomicBool::new(false));
    let server_stop = Arc::clone(&stop_server);
    let fail_front = Arc::new(AtomicBool::new(false));
    let server_fail = Arc::clone(&fail_front);
    let server = std::thread::spawn(move || {
        while !server_stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut request = [0u8; 1024];
                    let n = stream.read(&mut request).unwrap_or(0);
                    let _path = String::from_utf8_lossy(&request[..n]);
                    let response = if server_fail.load(Ordering::Relaxed) {
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .as_slice()
                    } else {
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                            .as_slice()
                    };
                    let _ = stream.write_all(response);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }
    });

    let ctx = DeployContext {
        registry_dir: registry_dir.clone(),
        state_dir: state_dir.clone(),
        lock_dir: dir.path().join("locks"),
        apply_paths: ApplyPaths {
            nginx_bin: nginx,
            main_config,
            target: target.clone(),
            pid_file: None,
        },
        drain_secs: 0,
        lock_timeout: std::time::Duration::from_secs(60),
    };
    let loaded = registry::load_dir(&registry_dir).unwrap();
    let mut app = loaded
        .apps
        .into_iter()
        .find(|app| app.name == "importdeploy")
        .unwrap();
    eprintln!(
        "app hostnames={:?} front={} live={:?}",
        app.hostnames, app.front_port, app.live_port
    );
    let mut run = Run::start(
        TraceLog::open(&trace).unwrap(),
        RunMode::Live,
        Some(app.name.clone()),
        &["swapdock".to_string()],
        swapdock::Redactor::new(),
    )
    .unwrap();

    let result = deploy::deploy(&mut run, &ctx, &mut app, &release);
    let _ = run.finish(if result.is_ok() {
        RunStatus::Ok
    } else {
        RunStatus::Failed
    });

    // A second release exercises the rolling swap path: the old container stays
    // until the new one is healthy, the front door is applied and probed, then
    // the old one is stopped and the registry commits the new release.
    let release2 = format!("{:07x}", std::process::id() as u64 + 1);
    assert!(
        docker(&["tag", "nginx:alpine", &format!("nginx:{release2}")])
            .status
            .success()
    );
    let mut run2 = Run::start(
        TraceLog::open(&trace).unwrap(),
        RunMode::Live,
        Some(app.name.clone()),
        &["swapdock".to_string()],
        swapdock::Redactor::new(),
    )
    .unwrap();
    let result2 = deploy::deploy(&mut run2, &ctx, &mut app, &release2);
    let _ = run2.finish(if result2.is_ok() {
        RunStatus::Ok
    } else {
        RunStatus::Failed
    });

    // A failed cutover must leave the previous release serving. The front door
    // starts answering 503, so the new container is healthy but the probe after
    // apply fails; the old container must stay and the registry must not commit.
    fail_front.store(true, Ordering::Relaxed);
    let release3 = format!("{:07x}", std::process::id() as u64 + 2);
    assert!(
        docker(&["tag", "nginx:alpine", &format!("nginx:{release3}")])
            .status
            .success()
    );
    let mut run3 = Run::start(
        TraceLog::open(&trace).unwrap(),
        RunMode::Live,
        Some(app.name.clone()),
        &["swapdock".to_string()],
        swapdock::Redactor::new(),
    )
    .unwrap();
    let result3 = deploy::deploy(&mut run3, &ctx, &mut app, &release3);
    let _ = run3.finish(if result3.is_ok() {
        RunStatus::Ok
    } else {
        RunStatus::Failed
    });
    fail_front.store(false, Ordering::Relaxed);
    let old_survived = docker(&["inspect", &format!("web-green-{release2}")])
        .status
        .success();

    // Best-effort cleanup before asserting, so a failed assertion does not leave
    // a container behind on the test host. The deploy names the container from
    // the service and the short release, not the compose project.
    let _ = docker(&["rm", "-f", &format!("web-green-{release}")]);
    let _ = docker(&["rm", "-f", &format!("web-blue-{release}")]);
    let _ = docker(&["rm", "-f", &format!("web-green-{release2}")]);
    let _ = docker(&["rm", "-f", &format!("web-blue-{release2}")]);
    let _ = docker(&["rm", "-f", &format!("web-green-{release3}")]);
    let _ = docker(&["rm", "-f", &format!("web-blue-{release3}")]);
    let _ = docker(&["rmi", &format!("nginx:{release}")]);
    let _ = docker(&["rmi", &format!("nginx:{release2}")]);
    let _ = docker(&["rmi", &format!("nginx:{release3}")]);
    // Per-release Compose projects leave a network behind: remove the ones this
    // test created so repeated runs cannot exhaust Docker's address pools.
    let project_prefix = format!("importdeploy{}-green-", std::process::id());
    if let Ok(list) = Command::new("docker")
        .args(["network", "ls", "--format", "{{.Name}}"])
        .output()
    {
        for name in String::from_utf8_lossy(&list.stdout).lines() {
            if name.starts_with(&project_prefix) {
                let _ = docker(&["network", "rm", name]);
            }
        }
    }
    stop_server.store(true, Ordering::Relaxed);
    let _ = server.join();

    result.expect("imported app deploy");
    result2.expect("second imported app deploy");
    assert!(result3.is_err(), "bad health gate must fail");
    assert_eq!(app.release.as_deref(), Some(release2.as_str()));
    assert!(
        old_survived,
        "old container must survive a failed front probe"
    );
    assert_eq!(app.live_port, Some(9001));
    assert!(target.is_file(), "nginx config written");
}
