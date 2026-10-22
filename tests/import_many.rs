//! Five real off-the-shelf Compose services imported and deployed in turn.
//!
//! The fixture images are public (nginx, httpd, caddy, python, busybox), not
//! mocks. Docker is required; the test skips cleanly if any image is missing.
//! It is serialized because the deploy engine allocates host ports.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use swapdock::apply::ApplyPaths;
use swapdock::deploy::{self, DeployContext};
use swapdock::import::{self, Options};
use swapdock::registry;
use swapdock::trace::{Run, RunMode, RunStatus, TraceLog};
use tempfile::TempDir;

static SERIAL: Mutex<()> = Mutex::new(());

fn docker(argv: &[&str]) -> std::process::Output {
    Command::new("docker").args(argv).output().expect("docker")
}

fn daemon() -> bool {
    if !docker(&["info"]).status.success() {
        eprintln!("SKIP import_many: no Docker daemon");
        return false;
    }
    true
}

fn free_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct Fixture {
    name: &'static str,
    image: &'static str,
    container_port: u16,
    command: Option<&'static str>,
}

const FIXTURES: &[Fixture] = &[
    Fixture {
        name: "nginxapp",
        image: "nginx:alpine",
        container_port: 80,
        command: None,
    },
    Fixture {
        name: "httpdapp",
        image: "httpd:alpine",
        container_port: 80,
        command: None,
    },
    Fixture {
        name: "caddyapp",
        image: "caddy:alpine",
        container_port: 80,
        command: None,
    },
    Fixture {
        name: "pythonapp",
        image: "python:3.12-alpine",
        container_port: 80,
        command: Some("[\"python\", \"-m\", \"http.server\", \"80\"]"),
    },
    Fixture {
        name: "busyboxapp",
        image: "busybox:latest",
        container_port: 8080,
        command: Some(
            "[\"sh\", \"-c\", \"mkdir -p /www && echo ok > /www/index.html && exec httpd -f -p 8080 -h /www\"]",
        ),
    },
];

fn run_one(dir: &Path, fixture: &Fixture, index: u8) -> Result<(), String> {
    let project = format!("{}{}", fixture.name, std::process::id());
    let compose_dir = dir.join(&project);
    std::fs::create_dir_all(&compose_dir).map_err(|e| e.to_string())?;
    let command = fixture
        .command
        .map(|value| format!("    command: {value}\n"))
        .unwrap_or_default();
    let compose = format!(
        "services:\n  web:\n    image: {}\n{}    ports:\n      - \"127.0.0.1:3000:{}\"\n    healthcheck:\n      test: [\"CMD-SHELL\", \"wget -q -O /dev/null http://127.0.0.1:{}/ || exit 1\"]\n      interval: 2s\n      timeout: 2s\n      retries: 3\n      start_period: 2s\n",
        fixture.image, command, fixture.container_port, fixture.container_port
    );
    std::fs::write(compose_dir.join("compose.yaml"), compose).map_err(|e| e.to_string())?;

    let registry_dir = dir.join("apps");
    let state_dir = dir.join("green");
    let front = free_port();
    let opts = Options {
        compose_dir,
        name: Some(fixture.name.into()),
        front_port: Some(front),
        slot: Some(index + 10),
        domain: Some(format!("{}.test", fixture.name)),
        state_dir: state_dir.clone(),
        ..Options::default()
    };
    import::run(&opts, &registry_dir, None).map_err(|e| e.to_string())?;
    let loaded = registry::load_dir(&registry_dir).map_err(|e| e.to_string())?;
    let mut app = loaded
        .apps
        .into_iter()
        .find(|app| app.name == fixture.name)
        .ok_or("imported app missing")?;

    let nginx = dir.join(format!("fake-nginx-{index}"));
    std::fs::write(
        &nginx,
        "#!/bin/sh\ncase \"$1\" in -t|-s) exit 0;; esac\nexit 0\n",
    )
    .map_err(|e| e.to_string())?;
    std::fs::set_permissions(&nginx, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| e.to_string())?;
    let main_config = dir.join(format!("nginx-{index}.conf"));
    std::fs::write(&main_config, "events {}\nhttp {}\n").map_err(|e| e.to_string())?;
    let target = dir.join(format!("front-{index}.conf"));
    let trace = dir.join(format!("trace-{index}.jsonl"));

    let listener = TcpListener::bind(("127.0.0.1", front)).map_err(|e| e.to_string())?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
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

    let release = format!("{:07x}", std::process::id() as u64 + index as u64);
    let tagged = format!(
        "{}:{release}",
        fixture.image.split(':').next().unwrap_or("image")
    );
    let _ = docker(&["tag", fixture.image, &tagged]);

    let ctx = DeployContext {
        registry_dir: registry_dir.clone(),
        state_dir: state_dir.clone(),
        lock_dir: dir.join(format!("locks-{index}")),
        apply_paths: ApplyPaths {
            nginx_bin: nginx,
            main_config,
            target: target.clone(),
            pid_file: None,
        },
        drain_secs: 0,
        lock_timeout: Duration::from_secs(60),
    };
    let mut run = Run::start(
        TraceLog::open(&trace).map_err(|e| e.to_string())?,
        RunMode::Live,
        Some(app.name.clone()),
        &["swapdock".to_string()],
        swapdock::Redactor::new(),
    )
    .map_err(|e| e.to_string())?;
    let result = deploy::deploy(&mut run, &ctx, &mut app, &release);
    let _ = run.finish(if result.is_ok() {
        RunStatus::Ok
    } else {
        RunStatus::Failed
    });
    stop.store(true, Ordering::Relaxed);
    let _ = server.join();
    let _ = docker(&["rm", "-f", &format!("web-green-{release}")]);
    let _ = docker(&["rm", "-f", &format!("web-blue-{release}")]);
    let _ = docker(&["rmi", &tagged]);

    // Green projects are per release; remove the test network so repeated runs
    // cannot exhaust Docker's address pools.
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

    result.map_err(|e| format!("{}: {e}", fixture.name))?;
    if app.release.as_deref() != Some(release.as_str()) {
        return Err(format!("{}: release not committed", fixture.name));
    }
    if !target.is_file() {
        return Err(format!("{}: nginx target missing", fixture.name));
    }
    Ok(())
}

#[test]
fn five_real_compose_services_import_and_deploy() {
    if !daemon() {
        return;
    }
    let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for fixture in FIXTURES {
        if !docker(&["image", "inspect", fixture.image])
            .status
            .success()
        {
            eprintln!("SKIP import_many: {} is not available", fixture.image);
            return;
        }
    }
    let dir = TempDir::new().unwrap();
    for (index, fixture) in FIXTURES.iter().enumerate() {
        run_one(dir.path(), fixture, index as u8).unwrap_or_else(|e| panic!("{e}"));
    }
}
