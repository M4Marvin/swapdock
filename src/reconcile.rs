//! Reconciling a crash between the nginx flip and the registry commit.
//!
//! A deploy writes a pending journal record before it changes host state. If the
//! process dies after nginx starts serving the new release but before the
//! registry commit, the registry describes the old release while traffic is on
//! the new one. `reconcile` reads the pending record, proves what the host is
//! actually serving, and advances the registry only when the evidence agrees.
//!
//! It never guesses. If nginx does not point at the pending release, or the
//! release does not answer its health path, the record is left in place and the
//! operator gets a specific refusal.

use std::path::Path;

use serde_json::json;
use thiserror::Error;

use crate::deploy::DeployContext;
use crate::health;
use crate::journal;
use crate::registry::{self, App, Kind};
use crate::trace::{Run, StepRecord, StepStatus};

/// What `reconcile` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The registry now matches the host.
    Committed {
        release: String,
        live_port: Option<u16>,
    },
    /// The record says the flip did not happen; the registry was not touched.
    NotFlipped { release: String, expected_port: u16 },
}

/// Why reconciliation was refused or failed.
#[derive(Debug, Error)]
pub enum ReconcileError {
    #[error("{0} has no pending deploy record; nothing to reconcile")]
    NoPending(String),

    #[error("could not read the pending record: {0}")]
    Journal(String),

    #[error("could not read the installed nginx config {path}: {reason}")]
    Config { path: String, reason: String },

    #[error(
        "nginx does not point at the pending release {release} (expected upstream {port}); \
         the flip did not happen. The registry was not changed."
    )]
    NotFlipped { release: String, port: u16 },

    #[error(
        "the static symlink for {app} does not point at the pending release {release}; \
         the flip did not happen. The registry was not changed."
    )]
    StaticNotFlipped { app: String, release: String },

    #[error("could not probe the pending release: {0}")]
    Probe(String),

    #[error(
        "the container on port {port} is not the pending image {expected} (found {actual}); refusing to commit"
    )]
    WrongImage {
        port: u16,
        expected: String,
        actual: String,
    },

    #[error("could not inspect the running container: {0}")]
    Docker(String),

    #[error("the front door for {host} is not healthy (HTTP {detail}); refusing to commit")]
    FrontUnhealthy { host: String, detail: String },

    #[error(
        "nginx points at the pending release {release}, but it is not healthy on port {port} \
         (last answer: {detail}); reconcile refused. Roll back or fix the release first."
    )]
    Unhealthy {
        release: String,
        port: u16,
        detail: String,
    },

    #[error("could not save the registry: {0}")]
    Save(String),
}

/// True when an nginx config has a `proxy_pass` for this loopback port.
///
/// The generated front door uses exactly `proxy_pass http://127.0.0.1:<port>;`.
/// Matching a whole line avoids a substring hit in a comment or another app's
/// port (for example 9001 vs 90011).
pub fn nginx_points_to(config: &str, port: u16) -> bool {
    let expected = format!("proxy_pass http://127.0.0.1:{port};");
    config.lines().any(|line| line.trim() == expected)
}

/// True when a static root symlink targets `releases/<release>`.
pub fn static_link_points_to(root: &Path, release: &str) -> bool {
    std::fs::read_link(root)
        .ok()
        .and_then(|target| target.file_name().map(|name| name == release))
        .unwrap_or(false)
}

/// Reconciles the pending record for one app.
pub fn reconcile(
    run: &mut Run,
    ctx: &DeployContext,
    app: &mut App,
) -> Result<Outcome, ReconcileError> {
    let pending = journal::read(&ctx.state_dir, &app.name)
        .map_err(|e| ReconcileError::Journal(e.to_string()))?
        .ok_or_else(|| ReconcileError::NoPending(app.name.clone()))?;

    let health_url = app.health_url.as_deref().unwrap_or("http://127.0.0.1/");
    let path =
        health::checked_probe_path(health_url).map_err(|e| ReconcileError::Probe(e.to_string()))?;
    let host = app
        .hostnames
        .first()
        .map(String::as_str)
        .unwrap_or("127.0.0.1");
    match app.kind {
        Kind::Static => {
            let root = app
                .root
                .as_deref()
                .ok_or_else(|| ReconcileError::Probe("static app has no root".into()))?;
            if !static_link_points_to(Path::new(root), &pending.release) {
                return Err(ReconcileError::StaticNotFlipped {
                    app: app.name.clone(),
                    release: pending.release,
                });
            }
            match health::http_status("127.0.0.1", app.front_port, &path) {
                Ok(code) if (200..300).contains(&code) => {}
                Ok(code) => {
                    return Err(ReconcileError::Unhealthy {
                        release: pending.release,
                        port: app.front_port,
                        detail: format!("HTTP {code}"),
                    });
                }
                Err(detail) => {
                    return Err(ReconcileError::Unhealthy {
                        release: pending.release,
                        port: app.front_port,
                        detail,
                    });
                }
            }
            if app.release.as_deref() == Some(pending.release.as_str()) {
                app.live_port = Some(app.front_port);
            } else {
                app.advance(pending.release.clone(), app.front_port);
            }
        }
        Kind::Container => {
            let config = std::fs::read_to_string(&ctx.apply_paths.target).map_err(|e| {
                ReconcileError::Config {
                    path: ctx.apply_paths.target.display().to_string(),
                    reason: e.to_string(),
                }
            })?;
            if !nginx_points_to(&config, pending.to_port) {
                return Err(ReconcileError::NotFlipped {
                    release: pending.release,
                    port: pending.to_port,
                });
            }
            match health::http_status_with_host("127.0.0.1", pending.to_port, &path, host) {
                Ok(code) if (200..300).contains(&code) => {}
                Ok(code) => {
                    return Err(ReconcileError::Unhealthy {
                        release: pending.release,
                        port: pending.to_port,
                        detail: format!("HTTP {code}"),
                    });
                }
                Err(detail) => {
                    return Err(ReconcileError::Unhealthy {
                        release: pending.release,
                        port: pending.to_port,
                        detail,
                    });
                }
            }
            let expected =
                app.image_ref(&pending.release)
                    .ok_or_else(|| ReconcileError::WrongImage {
                        port: pending.to_port,
                        expected: "<no image_repo>".into(),
                        actual: "<none>".into(),
                    })?;
            let ps = run
                .exec(&crate::docker::ps_publishing(pending.to_port))
                .map_err(|e| ReconcileError::Docker(e.to_string()))?;
            let container = ps.stdout_trimmed().lines().next().unwrap_or("");
            if container.is_empty() {
                return Err(ReconcileError::WrongImage {
                    port: pending.to_port,
                    expected,
                    actual: "<no container>".into(),
                });
            }
            let actual = run
                .exec(&crate::docker::inspect_image_ref(container))
                .map_err(|e| ReconcileError::Docker(e.to_string()))?;
            let actual = actual.stdout_trimmed().to_string();
            if actual != expected {
                return Err(ReconcileError::WrongImage {
                    port: pending.to_port,
                    expected,
                    actual,
                });
            }
            if app.release.as_deref() == Some(pending.release.as_str()) {
                app.live_port = Some(pending.to_port);
            } else {
                app.advance(pending.release.clone(), pending.to_port);
            }
        }
    }

    prove_front_door(app)?;

    registry::save_app(&ctx.registry_dir, app)
        .map_err(|e| ReconcileError::Save(format!("save {}: {e}", app.name)))?;

    // A crash after the flip but before stop-old leaves the previous release
    // publishing the alternate port. The registry now names the new release;
    // retire the stale container so the next swap/rollback can reuse that port.
    if let Some(old_port) = pending.from_port.filter(|port| *port != pending.to_port) {
        stop_old_on_port(run, old_port)?;
    }

    let _ = journal::clear(&ctx.state_dir, &app.name);

    let release = pending.release;
    let live_port = app.live_port;
    let seq = run.next_seq();
    run.record_step(
        StepRecord::new(
            seq,
            "reconcile",
            StepStatus::Ok,
            &["registry-commit".to_string(), release.clone()],
        )
        .with_detail(json!({"release": release, "live_port": live_port})),
    )
    .map_err(|e| ReconcileError::Journal(e.to_string()))?;

    Ok(Outcome::Committed { release, live_port })
}

fn prove_front_door(app: &App) -> Result<(), ReconcileError> {
    let Some(host) = app.hostnames.first() else {
        return Ok(());
    };
    match health::http_status_with_host("127.0.0.1", app.front_port, "/", host) {
        Ok(code) if (200..400).contains(&code) => Ok(()),
        Ok(code) => Err(ReconcileError::FrontUnhealthy {
            host: host.clone(),
            detail: code.to_string(),
        }),
        Err(detail) => Err(ReconcileError::FrontUnhealthy {
            host: host.clone(),
            detail,
        }),
    }
}

fn stop_old_on_port(run: &mut Run, port: u16) -> Result<(), ReconcileError> {
    let found = run
        .exec(&crate::docker::ps_publishing(port))
        .map_err(|e| ReconcileError::Docker(e.to_string()))?;
    if !found.success() {
        return Err(ReconcileError::Docker(found.stderr_trimmed().to_string()));
    }
    for container in found
        .stdout_trimmed()
        .lines()
        .filter(|line| !line.is_empty())
    {
        let stop = run
            .exec(&crate::docker::stop(container))
            .map_err(|e| ReconcileError::Docker(e.to_string()))?;
        if !stop.success() {
            return Err(ReconcileError::Docker(stop.stderr_trimmed().to_string()));
        }
        let _ = run.exec(&crate::docker::rm(container));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nginx_match_is_exact_per_port() {
        let config = r#"
            location / {
                proxy_pass http://127.0.0.1:9001;
            }
        "#;
        assert!(nginx_points_to(config, 9001));
        assert!(!nginx_points_to(config, 900));
        assert!(!nginx_points_to(
            "# proxy_pass http://127.0.0.1:9001;",
            9001
        ));
    }

    #[test]
    fn startup_reconciles_a_pending_record_that_was_already_committed() {
        let dir = tempfile::TempDir::new().unwrap();
        let registry_dir = dir.path().join("apps");
        let state_dir = dir.path().join("green");
        std::fs::create_dir_all(&registry_dir).unwrap();
        std::fs::create_dir_all(crate::journal::dir(&state_dir)).unwrap();
        let text = r#"
name = "docs"
kind = "static"
strategy = "swap"
hostnames = ["docs.test"]
front_port = 8012
slot = 0
writes_state = false
root = "/srv/www/docs/current"
spa = false
compose_dir = "/tmp"
compose_svc = "docs"
env_name = "DOCS_PORT"
release = "abc1234"
live_port = 8012
"#;
        let mut app: App = toml::from_str(text).unwrap();
        let root = dir.path().join("www/current");
        std::fs::create_dir_all(dir.path().join("www/releases/abc1234")).unwrap();
        std::os::unix::fs::symlink("releases/abc1234", &root).unwrap();
        app.root = Some(root.display().to_string());
        let probe = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        app.front_port = port;
        registry::save_app(&registry_dir, &app).unwrap();
        let server = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for _ in 0..2 {
                if let Ok((mut stream, _)) = server.accept() {
                    let mut request = [0u8; 1024];
                    let _ = stream.read(&mut request);
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    );
                }
            }
        });
        crate::journal::write(
            &state_dir,
            &crate::journal::Pending::new("docs", "abc1234", None, port, "run-1"),
        )
        .unwrap();
        let ctx = DeployContext {
            registry_dir: registry_dir.clone(),
            state_dir: state_dir.clone(),
            lock_dir: dir.path().join("locks"),
            apply_paths: crate::apply::ApplyPaths {
                nginx_bin: "nginx".into(),
                main_config: dir.path().join("nginx.conf"),
                target: dir.path().join("front-door.conf"),
                pid_file: None,
            },
            drain_secs: 0,
            lock_timeout: std::time::Duration::from_secs(5),
        };
        let outcomes = reconcile_pending(&ctx, &dir.path().join("trace.jsonl"));
        assert_eq!(outcomes.len(), 1, "{outcomes:?}");
        assert!(matches!(outcomes[0].result, Ok(Outcome::Committed { .. })));
        assert!(
            crate::journal::read(&state_dir, "docs").unwrap().is_none(),
            "committed pending record must be cleared"
        );
    }

    #[test]
    fn static_link_must_name_the_release() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("current");
        std::fs::create_dir_all(dir.path().join("releases/abc1234")).unwrap();
        std::os::unix::fs::symlink("releases/abc1234", &root).unwrap();
        assert!(static_link_points_to(&root, "abc1234"));
        assert!(!static_link_points_to(&root, "def5678"));
    }
}

/// Reconciles every pending journal record found at startup.
///
/// A crash between the nginx flip and the registry commit leaves a pending
/// record. Running this before the API starts means a restart does not present
/// a registry that disagrees with the host. Each attempt gets its own run in the
/// shared log; a refusal leaves the journal in place for the operator.
#[derive(Debug)]
pub struct StartupOutcome {
    pub app: String,
    pub result: Result<Outcome, ReconcileError>,
}

pub fn reconcile_pending(ctx: &DeployContext, trace: &Path) -> Vec<StartupOutcome> {
    let dir = journal::dir(&ctx.state_dir);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let Ok(loaded) = crate::registry::load_dir(&ctx.registry_dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(app_name) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(mut app) = loaded.apps.iter().find(|app| app.name == app_name).cloned() else {
            continue;
        };
        let log = match crate::trace::TraceLog::open(trace) {
            Ok(log) => log,
            Err(e) => {
                out.push(StartupOutcome {
                    app: app_name.to_string(),
                    result: Err(ReconcileError::Journal(e.to_string())),
                });
                continue;
            }
        };
        let mut run = match Run::start_as(
            log,
            crate::trace::RunMode::Live,
            Some(app.name.clone()),
            &["swapdock".to_string(), "startup-reconcile".to_string()],
            crate::redact::Redactor::new(),
            Some("startup".to_string()),
        ) {
            Ok(run) => run,
            Err(e) => {
                out.push(StartupOutcome {
                    app: app_name.to_string(),
                    result: Err(ReconcileError::Journal(e.to_string())),
                });
                continue;
            }
        };
        let result = reconcile(&mut run, ctx, &mut app);
        let _ = run.finish(if result.is_ok() {
            crate::trace::RunStatus::Ok
        } else {
            crate::trace::RunStatus::Failed
        });
        out.push(StartupOutcome {
            app: app_name.to_string(),
            result,
        });
    }
    out
}
