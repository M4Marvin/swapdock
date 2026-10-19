//! Host and compose checks that the pure registry validator cannot see.
//!
//! `validate` answers "is this registry internally consistent?". `doctor`
//! answers "will the host actually do what the registry says?" — the questions
//! that caused real outages:
//!
//! * a `~/` bind mount that becomes `/root/...` when swapdock runs compose as
//!   root;
//! * a bind source that does not exist, so Docker silently creates an empty
//!   directory;
//! * a container still publishing the front port directly, which means the app
//!   was never adopted;
//! * a release or rollback image that is no longer in the daemon;
//! * a compose service with no healthcheck, which the deploy gate will refuse.

use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use crate::compose;
use crate::docker;
use crate::exec::StepSpec;
use crate::registry::{App, ImageRegistry, Kind, Problem};
use crate::trace::{Run, RunMode};

/// Runs every check for `apps`, optionally narrowed to one app.
pub fn check(run: &mut Run, apps: &[App], only: Option<&str>, public: bool) -> Vec<Problem> {
    let mut out = Vec::new();
    for app in apps {
        if only.is_some_and(|name| name != app.name) {
            continue;
        }
        check_compose_paths(run, app, &mut out);
        check_backup(run, app, &mut out);
        check_docker_state(run, app, &mut out);
        if public {
            check_public_url(run, app, &mut out);
        }
    }
    out.sort_by(|a, b| {
        a.app
            .cmp(&b.app)
            .then(a.code.cmp(b.code))
            .then(a.message.cmp(&b.message))
    });
    out
}

/// Finds `~/` in a compose file, ignoring comments.
///
/// This is a text check on purpose: parsing YAML here would duplicate compose's
/// own expansion rules, and the failure we care about is exactly the expansion
/// compose performs under sudo.
pub fn compose_home_paths(text: &str) -> Vec<(usize, String)> {
    text.lines()
        .enumerate()
        .filter_map(|(i, line)| {
            let trimmed = line.trim_start();
            if trimmed.starts_with('#') || !line.contains("~/") {
                return None;
            }
            Some((i + 1, line.trim().to_string()))
        })
        .collect()
}

/// Bind mounts declared by the fully merged compose model.
pub fn bind_sources(config: &Value, service: &str) -> Vec<(String, String)> {
    let Some(volumes) = config
        .get("services")
        .and_then(|s| s.get(service))
        .and_then(|s| s.get("volumes"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    volumes
        .iter()
        .filter_map(|v| {
            if v.get("type").and_then(Value::as_str) != Some("bind") {
                return None;
            }
            let source = v.get("source").and_then(Value::as_str)?;
            let target = v.get("target").and_then(Value::as_str).unwrap_or("");
            Some((source.to_string(), target.to_string()))
        })
        .collect()
}

/// True when the service declares a healthcheck in the merged model.
pub fn healthcheck_present(config: &Value, service: &str) -> bool {
    config
        .get("services")
        .and_then(|s| s.get(service))
        .and_then(|s| s.get("healthcheck"))
        .is_some_and(|v| !v.is_null())
}

fn check_compose_paths(run: &mut Run, app: &App, out: &mut Vec<Problem>) {
    if app.kind != Kind::Container {
        return;
    }
    let path = app.compose_dir.join("docker-compose.yml");
    let Ok(text) = std::fs::read_to_string(&path) else {
        out.push(Problem::error(
            "compose-missing",
            Some(&app.name),
            format!("{} is not readable", path.display()),
        ));
        return;
    };
    for (line, text) in compose_home_paths(&text) {
        out.push(Problem::error(
            "compose-home-path",
            Some(&app.name),
            format!(
                "{}:{line} uses a home-relative path ({text}); under sudo compose expands \
                 `~` to /root, not the deploy user's home. Use an absolute path.",
                path.display()
            ),
        ));
    }
    // A compose file that is not used by this run is still worth validating:
    // the next `up` will read it.
    if run.mode() == RunMode::DryRun {
        return;
    }
    let extra: Vec<&Path> = app.compose_override.as_deref().into_iter().collect();
    let spec = compose::config_json(&path, &app.compose_project(), &extra, &app.compose_svc);
    let outcome = match run.exec(&spec) {
        Ok(o) if o.success() => o,
        Ok(o) => {
            out.push(Problem::warning(
                "compose-config-unreadable",
                Some(&app.name),
                format!(
                    "docker compose config failed for {}: {}",
                    path.display(),
                    o.stderr_trimmed()
                ),
            ));
            return;
        }
        Err(e) => {
            out.push(Problem::warning(
                "compose-config-unreadable",
                Some(&app.name),
                format!("could not run docker compose config: {e}"),
            ));
            return;
        }
    };
    let Ok(config) = serde_json::from_str::<Value>(outcome.stdout_trimmed()) else {
        out.push(Problem::warning(
            "compose-config-unreadable",
            Some(&app.name),
            "docker compose config did not return JSON; host checks skipped",
        ));
        return;
    };
    if !healthcheck_present(&config, &app.compose_svc) {
        out.push(Problem::warning(
            "no-healthcheck",
            Some(&app.name),
            format!(
                "compose service {:?} has no healthcheck; the deploy gate will refuse it",
                app.compose_svc
            ),
        ));
    }
    for (source, target) in bind_sources(&config, &app.compose_svc) {
        if !Path::new(&source).exists() {
            out.push(Problem::error(
                "mount-source-missing",
                Some(&app.name),
                format!(
                    "bind source {source} for {target} does not exist; Docker would create an \
                     empty directory and the app would start without its data"
                ),
            ));
        }
    }
}

fn check_backup(_run: &mut Run, app: &App, out: &mut Vec<Problem>) {
    if app.writes_state && app.backup.is_none() {
        out.push(Problem::warning(
            "backup-unset",
            Some(&app.name),
            "writes_state is set but no [backup] command is configured: \
             a failed deploy has no restore point",
        ));
    }
}

fn check_docker_state(run: &mut Run, app: &App, out: &mut Vec<Problem>) {
    if app.kind != Kind::Container || run.mode() == RunMode::DryRun {
        return;
    }
    let port = app.live_port.unwrap_or(app.front_port);
    let found = match run.exec(&docker::ps_publishing(port)) {
        Ok(o) if o.success() => o,
        Ok(o) => {
            out.push(Problem::warning(
                "docker-unavailable",
                Some(&app.name),
                format!("docker ps failed: {}", o.stderr_trimmed()),
            ));
            return;
        }
        Err(e) => {
            out.push(Problem::warning(
                "docker-unavailable",
                Some(&app.name),
                format!("could not run docker ps: {e}"),
            ));
            return;
        }
    };
    let names: Vec<&str> = found
        .stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();

    if app.live_port.is_none() && !names.is_empty() {
        out.push(Problem::warning(
            "legacy-front-publisher",
            Some(&app.name),
            format!(
                "container(s) {} publish front_port {} but the registry has no live_port; \
                 run `swapdock adopt {} --release <current>`",
                names.join(", "),
                app.front_port,
                app.name
            ),
        ));
    }

    for name in &names {
        let Ok(mounts) = run.exec(&docker::inspect_mounts(name)) else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<Value>(mounts.stdout_trimmed()) else {
            continue;
        };
        let Some(array) = parsed.as_array() else {
            continue;
        };
        for mount in array {
            let source = mount.get("Source").and_then(Value::as_str).unwrap_or("");
            let target = mount
                .get("Destination")
                .and_then(Value::as_str)
                .unwrap_or("");
            if source == "/root" || source.starts_with("/root/") {
                out.push(Problem::error(
                    "container-root-mount",
                    Some(&app.name),
                    format!(
                        "container {name} mounts {source} at {target}; this is the root \
                         user's home, not the deploy user's data. Fix the compose path."
                    ),
                ));
            }
        }
    }

    if app.image_registry() == ImageRegistry::Local {
        if let Some(release) = &app.release
            && let Some(image_ref) = app.image_ref(release)
        {
            match run.exec(&docker::image_exists(&image_ref)) {
                Ok(o) if !o.success() => out.push(Problem::error(
                    "release-image-missing",
                    Some(&app.name),
                    format!("{image_ref} is not in the local daemon"),
                )),
                Err(e) => out.push(Problem::warning(
                    "docker-unavailable",
                    Some(&app.name),
                    format!("could not inspect {image_ref}: {e}"),
                )),
                _ => {}
            }
        }
        if let Some(old) = &app.old_release
            && let Some(image_ref) = app.image_ref(old)
            && let Ok(o) = run.exec(&docker::image_exists(&image_ref))
            && !o.success()
        {
            out.push(Problem::warning(
                "old-release-image-missing",
                Some(&app.name),
                format!("rollback target {image_ref} is not in the local daemon"),
            ));
        }
    }
}

fn check_public_url(run: &mut Run, app: &App, out: &mut Vec<Problem>) {
    let Some(url) = app.public_url.as_deref() else {
        return;
    };
    if run.mode() == RunMode::DryRun {
        return;
    }
    let spec = StepSpec::new("doctor-public", "curl")
        .args([
            "-sS",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "--max-time",
            "10",
            url,
        ])
        .timeout(Duration::from_secs(15));
    let code = match run.exec(&spec) {
        Ok(o) => o.stdout_trimmed().to_string(),
        Err(e) => {
            out.push(Problem::warning(
                "public-url-down",
                Some(&app.name),
                format!("could not probe {url}: {e}"),
            ));
            return;
        }
    };
    let Ok(n) = code.parse::<u16>() else {
        out.push(Problem::warning(
            "public-url-down",
            Some(&app.name),
            format!("{url} returned no HTTP status"),
        ));
        return;
    };
    if !(200..400).contains(&n) {
        out.push(Problem::warning(
            "public-url-down",
            Some(&app.name),
            format!("{url} returned HTTP {n}"),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn home_relative_mounts_are_found_but_comments_are_not() {
        let text = "\
services:
  app:
    volumes:
      - ~/data/app:/app/data
      # - ~/commented:/ignored
      - /srv/data/app:/app/data
";
        let found = compose_home_paths(text);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].0, 4);
        assert!(found[0].1.contains("~/data/app"));
    }

    #[test]
    fn bind_sources_read_only_bind_volumes() {
        let config = json!({
            "services": {
                "app": {
                    "volumes": [
                        {"type": "bind", "source": "/home/marv/data/app", "target": "/app/data"},
                        {"type": "volume", "source": "appdata", "target": "/var/lib/app"},
                    ]
                }
            }
        });
        assert_eq!(
            bind_sources(&config, "app"),
            vec![("/home/marv/data/app".to_string(), "/app/data".to_string())]
        );
        assert!(bind_sources(&config, "missing").is_empty());
    }

    #[test]
    fn healthcheck_presence_is_read_from_the_merged_model() {
        let with = json!({"services": {"app": {"healthcheck": {"test": ["CMD", "true"]}}}});
        let without = json!({"services": {"app": {}}});
        assert!(healthcheck_present(&with, "app"));
        assert!(!healthcheck_present(&without, "app"));
        assert!(!healthcheck_present(&without, "missing"));
    }
}
