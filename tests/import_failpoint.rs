//! A swap crash after the nginx flip is recovered by `reconcile`.
//!
//! Debug-only failpoint: the deploy engine leaves the new green container and
//! the flipped nginx config in place, exactly as a killed process would. The
//! test then proves the startup path commits from image+health+front-door
//! evidence and retires the old container.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use swapdock::apply::ApplyPaths;
use swapdock::deploy::{self, DeployContext};
use swapdock::import::{self, Options};
use swapdock::registry;
use swapdock::trace::{Run, RunMode, RunStatus, TraceLog};
use tempfile::TempDir;

fn docker(argv: &[&str]) -> std::process::Output {
    Command::new("docker").args(argv).output().expect("docker")
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn a_swap_crash_after_the_flip_reconciles_to_the_new_release() {
    if !docker(&["info"]).status.success() {
        eprintln!("SKIP import_failpoint: no Docker daemon");
        return;
    }
    let dir = TempDir::new().unwrap();
    let project = format!("failpoint{}", std::process::id());
    let compose_dir = dir.path().join(&project);
    std::fs::create_dir_all(&compose_dir).unwrap();
    std::fs::write(
        compose_dir.join("compose.yaml"),
        "services:\n  web:\n    image: nginx:alpine\n    ports:\n      - \"127.0.0.1:3000:80\"\n    healthcheck:\n      test: [\"CMD\", \"wget\", \"-q\", \"-O\", \"/dev/null\", \"http://127.0.0.1/\"]\n      interval: 2s\n      timeout: 2s\n      retries: 3\n      start_period: 2s\n",
    )
    .unwrap();

    let release1 = format!("{:07x}", std::process::id());
    let release2 = format!("{:07x}", std::process::id() as u64 + 1);
    for release in [&release1, &release2] {
        assert!(
            docker(&["tag", "nginx:alpine", &format!("nginx:{release}")])
                .status
                .success()
        );
    }

    let registry_dir = dir.path().join("apps");
    let state_dir = dir.path().join("green");
    let front = free_port();
    let opts = Options {
        compose_dir,
        name: Some("failpoint".into()),
        front_port: Some(front),
        slot: Some(30),
        domain: Some("failpoint.test".into()),
        state_dir: state_dir.clone(),
        ..Options::default()
    };
    import::run(&opts, &registry_dir, None).expect("import");

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

    let listener = TcpListener::bind(("127.0.0.1", front)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let server_stop = Arc::clone(&stop);
    let server = std::thread::spawn(move || {
        while !server_stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut request = [0u8; 1024];
                    let _ = stream.read(&mut request);
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    );
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20));
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
        lock_timeout: Duration::from_secs(60),
    };

    let mut app = registry::load_dir(&registry_dir)
        .unwrap()
        .apps
        .into_iter()
        .find(|app| app.name == "failpoint")
        .unwrap();
    let mut run = Run::start(
        TraceLog::open(&trace).unwrap(),
        RunMode::Live,
        Some(app.name.clone()),
        &["swapdock".to_string()],
        swapdock::Redactor::new(),
    )
    .unwrap();
    deploy::deploy(&mut run, &ctx, &mut app, &release1).expect("first deploy");
    run.finish(RunStatus::Ok).unwrap();

    // Crash after the flip: the new container and nginx config are live, but
    // the registry is still on release1 and the journal is pending.
    unsafe { std::env::set_var("SWAPDOCK_FAILPOINT", "swap-flipped") };
    let mut run = Run::start(
        TraceLog::open(&trace).unwrap(),
        RunMode::Live,
        Some(app.name.clone()),
        &["swapdock".to_string()],
        swapdock::Redactor::new(),
    )
    .unwrap();
    let crashed = deploy::deploy(&mut run, &ctx, &mut app, &release2);
    unsafe { std::env::remove_var("SWAPDOCK_FAILPOINT") };
    let _ = run.finish(RunStatus::Failed);
    assert!(crashed.is_err(), "failpoint must abort the swap");
    let pending = swapdock::journal::read(&state_dir, "failpoint").unwrap();
    assert_eq!(pending.map(|p| p.release), Some(release2.clone()));

    // Restart/reconcile path.
    let mut app = registry::load_dir(&registry_dir)
        .unwrap()
        .apps
        .into_iter()
        .find(|app| app.name == "failpoint")
        .unwrap();
    let mut run = Run::start(
        TraceLog::open(&trace).unwrap(),
        RunMode::Live,
        Some(app.name.clone()),
        &["swapdock".to_string(), "reconcile".to_string()],
        swapdock::Redactor::new(),
    )
    .unwrap();
    let outcome = swapdock::reconcile::reconcile(&mut run, &ctx, &mut app).unwrap();
    run.finish(RunStatus::Ok).unwrap();
    assert!(matches!(
        outcome,
        swapdock::reconcile::Outcome::Committed { .. }
    ));
    assert_eq!(app.release.as_deref(), Some(release2.as_str()));
    assert!(
        swapdock::journal::read(&state_dir, "failpoint")
            .unwrap()
            .is_none()
    );
    assert!(
        !docker(&["inspect", &format!("web-green-{release1}")])
            .status
            .success(),
        "the old container must be retired"
    );

    stop.store(true, Ordering::Relaxed);
    let _ = server.join();
    for release in [&release1, &release2] {
        let _ = docker(&["rm", "-f", &format!("web-green-{release}")]);
        let _ = docker(&["rm", "-f", &format!("web-blue-{release}")]);
        let _ = docker(&["rmi", &format!("nginx:{release}")]);
    }
    let prefix = format!("{project}-green-");
    if let Ok(list) = Command::new("docker")
        .args(["network", "ls", "--format", "{{.Name}}"])
        .output()
    {
        for name in String::from_utf8_lossy(&list.stdout).lines() {
            if name.starts_with(&prefix) {
                let _ = docker(&["network", "rm", name]);
            }
        }
    }
    let _ = Path::new(&target);
}
