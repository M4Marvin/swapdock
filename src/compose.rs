//! `docker compose` argument builders.
//!
//! Pure functions: compose file, project and service in, argv out. No process is
//! started here — the caller hands the argv to the exec chokepoint, which is what
//! makes every invocation logged, timed out and dry-runnable.
//!
//! Two conventions these builders assume, both documented where they matter:
//!
//! * The compose file reads its port from an environment variable
//!   (`ports: ["127.0.0.1:${PORTFOLIO_PORT}:80"]`). The env var name comes from
//!   the registry, so the tool never edits a compose file.
//! * The image reads from `$IMAGE` with a local default
//!   (`image: ${PORTFOLIO_IMAGE:-apps-portfolio}`). Same reason.

use std::path::{Path, PathBuf};

use serde_json::Value;
use thiserror::Error;

use crate::exec::StepSpec;

/// Filenames Compose v2 accepts, in the order they are tried.
///
/// `docker compose` itself prefers `compose.yaml`; matching that order means a
/// directory holding both files deploys the same one the operator sees when
/// running compose by hand.
pub const COMPOSE_FILENAMES: [&str; 3] = ["compose.yaml", "compose.yml", "docker-compose.yml"];

/// Why a compose file could not be found.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ComposeFileError {
    #[error(
        "no compose file in {dir}: expected one of {candidates}; \
         create it or point compose_dir at the directory that holds it"
    )]
    Missing { dir: String, candidates: String },
}

/// Finds the compose file in `dir`, preferring Compose v2's names.
///
/// Every Compose invocation must use the same file. Resolving it once, here,
/// keeps the deploy path, adoption and `doctor` from disagreeing when a project
/// ships `compose.yaml` instead of the legacy `docker-compose.yml`.
pub fn resolve_compose_file(dir: &Path) -> Result<PathBuf, ComposeFileError> {
    for name in COMPOSE_FILENAMES {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(ComposeFileError::Missing {
        dir: dir.display().to_string(),
        candidates: COMPOSE_FILENAMES.join(", "),
    })
}

/// Reasons a `swap` must be refused for this service, from the merged model.
///
/// The green container runs in a *different* Compose project, so Compose
/// namespaces its named volumes and default network differently: a service with
/// `data:/var/lib/app` would mount an empty `project-green_data`, and a service
/// with `depends_on: [db]` would have no `db` in its project. Both fail silently
/// or reset data, so they are refused instead of guessed at.
///
/// Bind mounts are deliberately allowed: they point at the same host path in
/// both projects. An anonymous volume (type `volume` with no source) is also
/// fine to leave alone; it is fresh in either project and holds no named data.
pub fn swap_blockers(config: &Value, service: &str) -> Vec<String> {
    let Some(svc) = config.get("services").and_then(|s| s.get(service)) else {
        return vec![format!(
            "service {service:?} is missing from the merged compose model"
        )];
    };

    let mut out = Vec::new();

    for volume in svc
        .get("volumes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let volume_type = volume.get("type").and_then(Value::as_str);
        let source = volume
            .get("source")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty());
        if volume_type == Some("volume")
            && let Some(source) = source
        {
            let target = volume
                .get("target")
                .and_then(Value::as_str)
                .unwrap_or("<unknown target>");
            out.push(format!(
                "service {service:?} mounts named volume {source:?} at {target:?}; \
                 the green project would mount a new, empty {source:?}"
            ));
        }
    }

    let depends_on = service_references(svc.get("depends_on"));
    if !depends_on.is_empty() {
        out.push(format!(
            "service {service:?} depends on {}; those services would not exist in the green project",
            depends_on.join(", ")
        ));
    }

    let links = service_references(svc.get("links"));
    if !links.is_empty() {
        out.push(format!(
            "service {service:?} links to {}; those links would not exist in the green project",
            links.join(", ")
        ));
    }

    let volumes_from = service_references(svc.get("volumes_from"));
    if !volumes_from.is_empty() {
        out.push(format!(
            "service {service:?} takes volumes from {}; those containers would not exist in the green project",
            volumes_from.join(", ")
        ));
    }

    out
}

/// Names in a `depends_on`/`links`/`volumes_from` value.
///
/// `docker compose config --format json` normally renders `depends_on` as a map
/// of service names to conditions and the other two as arrays, but accepting both
/// shapes keeps this helper honest against older daemons.
fn service_references(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Object(map)) => map.keys().cloned().collect(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// Standard prelude every compose invocation carries.
fn base(name: &'static str, compose_file: &Path, project: &str) -> StepSpec {
    StepSpec::new(name, "docker")
        .args(["compose", "-p", project, "-f"])
        .arg(compose_file.display().to_string())
}

/// Pulls one service's image. Fails when the image only exists locally, so the
/// caller skips this for local-only registries.
pub fn pull(compose_file: &Path, project: &str, service: &str) -> StepSpec {
    base("compose-pull", compose_file, project)
        .arg("pull")
        .arg(service)
}

/// Starts one service detached, without touching its dependencies.
///
/// `--no-deps` matters: without it compose may recreate dependencies of the
/// service, which turns a one-app swapdock into a multi-app event. `--wait` is a
/// backstop; the explicit health gate in `health.rs` is what reports clearly.
pub fn up_detached(
    compose_file: &Path,
    project: &str,
    extra_files: &[&Path],
    service: &str,
) -> StepSpec {
    let mut spec = base("compose-up", compose_file, project);
    for file in extra_files {
        spec = spec.arg("-f").arg(file.display().to_string());
    }
    spec.args(["up", "-d", "--no-deps", "--no-build", "--wait", service])
}

/// Removes a per-release green project and its network after the cutover.
pub fn down(compose_file: &Path, project: &str, extra_files: &[&Path]) -> StepSpec {
    let mut spec = base("compose-down", compose_file, project);
    for file in extra_files {
        spec = spec.arg("-f").arg(file.display().to_string());
    }
    spec.args(["down", "--volumes", "--remove-orphans"])
}

/// Stops one service.
pub fn stop(compose_file: &Path, project: &str, service: &str) -> StepSpec {
    base("compose-stop", compose_file, project)
        .arg("stop")
        .arg(service)
}

/// Removes a stopped service's container.
pub fn rm(compose_file: &Path, project: &str, service: &str) -> StepSpec {
    base("compose-rm", compose_file, project).args(["rm", "-f", service])
}

/// Renders the fully merged compose model as JSON.
///
/// This is how adoption and `doctor` ask compose what it will actually do,
/// without parsing YAML themselves. Later files win, matching `up_detached`.
pub fn config_json(
    compose_file: &Path,
    project: &str,
    extra_files: &[&Path],
    service: &str,
) -> StepSpec {
    let mut spec = base("compose-config", compose_file, project);
    for file in extra_files {
        spec = spec.arg("-f").arg(file.display().to_string());
    }
    // Restrict to one service: the full model can exceed the chokepoint's
    // retained output tail, and a truncated JSON document does not parse.
    spec.args(["config", "--format", "json", service])
}

/// Validates the merged compose model without starting anything.
pub fn config_quiet(compose_file: &Path, project: &str, extra_files: &[&Path]) -> StepSpec {
    let mut spec = base("compose-config", compose_file, project);
    for file in extra_files {
        spec = spec.arg("-f").arg(file.display().to_string());
    }
    spec.args(["config", "--quiet"])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn file() -> PathBuf {
        PathBuf::from("/home/marv/apps/docker-compose.yml")
    }

    #[test]
    fn pull_targets_one_service() {
        let spec = pull(&file(), "apps", "portfolio");
        assert_eq!(
            spec.full_argv(),
            [
                "docker",
                "compose",
                "-p",
                "apps",
                "-f",
                "/home/marv/apps/docker-compose.yml",
                "pull",
                "portfolio"
            ]
        );
    }

    #[test]
    fn up_is_detached_without_dependencies_and_waits() {
        let spec = up_detached(&file(), "apps", &[], "portfolio");
        let argv = spec.full_argv();
        assert!(argv.contains(&"-d".to_string()), "{argv:?}");
        assert!(argv.contains(&"--no-deps".to_string()), "{argv:?}");
        assert!(argv.contains(&"--wait".to_string()), "{argv:?}");
        assert!(
            !argv.contains(&"--build".to_string()),
            "never build here: {argv:?}"
        );
        assert!(
            argv.contains(&"--no-build".to_string()),
            "compose must not auto-build a missing image: {argv:?}"
        );
    }

    #[test]
    fn extra_files_come_before_the_subcommand() {
        let extra = PathBuf::from("/srv/swapdock/green/portfolio.yml");
        let spec = up_detached(&file(), "apps-green", &[extra.as_path()], "portfolio");
        let argv = spec.full_argv();
        let up_pos = argv.iter().position(|a| a == "up").unwrap();
        let extra_pos = argv
            .iter()
            .position(|a| a == "/srv/swapdock/green/portfolio.yml")
            .unwrap();
        assert!(
            extra_pos < up_pos,
            "-f must precede the subcommand: {argv:?}"
        );
        assert!(argv.contains(&"apps-green".to_string()), "{argv:?}");
    }

    #[test]
    fn config_json_is_scoped_to_one_service() {
        let spec = config_json(&file(), "apps", &[], "chats");
        assert!(
            spec.full_argv().ends_with(&[
                "config".to_string(),
                "--format".to_string(),
                "json".to_string(),
                "chats".to_string()
            ]),
            "{:?}",
            spec.full_argv()
        );
    }

    #[test]
    fn stop_and_rm_target_one_service() {
        assert!(
            stop(&file(), "apps", "portfolio")
                .full_argv()
                .ends_with(&["stop".to_string(), "portfolio".to_string()])
        );
        let rm_argv = rm(&file(), "apps", "portfolio").full_argv();
        assert!(rm_argv.contains(&"-f".to_string()), "{rm_argv:?}");
    }

    #[test]
    fn step_names_are_stable() {
        // The run log groups by step name; renaming one breaks history queries.
        assert_eq!(pull(&file(), "p", "s").name, "compose-pull");
        assert_eq!(up_detached(&file(), "p", &[], "s").name, "compose-up");
        assert_eq!(stop(&file(), "p", "s").name, "compose-stop");
        assert_eq!(rm(&file(), "p", "s").name, "compose-rm");
    }

    // ---- compose file discovery ----

    fn temp_dir() -> tempfile::TempDir {
        tempfile::TempDir::new().unwrap()
    }

    #[test]
    fn every_supported_compose_filename_is_found() {
        for name in COMPOSE_FILENAMES {
            let dir = temp_dir();
            let path = dir.path().join(name);
            std::fs::write(&path, "services: {}\n").unwrap();
            assert_eq!(
                resolve_compose_file(dir.path()).unwrap(),
                path,
                "{name} must be supported"
            );
        }
    }

    #[test]
    fn compose_yaml_wins_when_several_candidates_exist() {
        // Same precedence as `docker compose` itself, so the file this tool
        // deploys is the file the operator sees compose use.
        let dir = temp_dir();
        for name in COMPOSE_FILENAMES {
            std::fs::write(dir.path().join(name), "services: {}\n").unwrap();
        }
        assert_eq!(
            resolve_compose_file(dir.path()).unwrap(),
            dir.path().join("compose.yaml")
        );

        std::fs::remove_file(dir.path().join("compose.yaml")).unwrap();
        assert_eq!(
            resolve_compose_file(dir.path()).unwrap(),
            dir.path().join("compose.yml")
        );
    }

    #[test]
    fn a_missing_compose_file_names_every_candidate() {
        let dir = temp_dir();
        let err = resolve_compose_file(dir.path()).unwrap_err();
        let message = err.to_string();
        for name in COMPOSE_FILENAMES {
            assert!(message.contains(name), "{message}");
        }
        assert!(
            message.contains(&dir.path().display().to_string()),
            "{message}"
        );
        assert!(message.contains("compose_dir"), "{message}");
    }

    #[test]
    fn a_directory_is_not_a_compose_file() {
        let dir = temp_dir();
        std::fs::create_dir(dir.path().join("compose.yaml")).unwrap();
        assert!(resolve_compose_file(dir.path()).is_err());
    }

    // ---- swap preflight ----

    fn merged(service_json: serde_json::Value) -> Value {
        serde_json::json!({"services": {"web": service_json}})
    }

    #[test]
    fn swap_blockers_names_named_volumes_and_dependencies() {
        let config = merged(serde_json::json!({
            "image": "app:abc1234",
            "volumes": [
                {"type": "volume", "source": "uploads", "target": "/data/uploads"},
                {"type": "bind", "source": "/srv/uploads", "target": "/data/uploads"}
            ],
            "depends_on": {"db": {"condition": "service_healthy"}},
            "links": ["redis:cache"]
        }));

        let blockers = swap_blockers(&config, "web");
        assert_eq!(blockers.len(), 3, "{blockers:?}");
        assert!(
            blockers.iter().any(|b| b.contains("\"uploads\"")),
            "{blockers:?}"
        );
        assert!(
            blockers.iter().any(|b| b.contains("depends on db")),
            "{blockers:?}"
        );
        assert!(
            blockers.iter().any(|b| b.contains("redis:cache")),
            "{blockers:?}"
        );
    }

    #[test]
    fn swap_allows_bind_mounts_and_anonymous_volumes() {
        let config = merged(serde_json::json!({
            "volumes": [
                {"type": "bind", "source": "/srv/app/data", "target": "/data"},
                {"type": "volume", "target": "/tmp/cache"}
            ]
        }));
        assert!(
            swap_blockers(&config, "web").is_empty(),
            "bind mounts point at the same host path in both projects"
        );
    }

    #[test]
    fn swap_blockers_accepts_array_shaped_dependencies() {
        let config = merged(serde_json::json!({"depends_on": ["db", "cache"]}));
        let blockers = swap_blockers(&config, "web");
        assert_eq!(blockers.len(), 1, "{blockers:?}");
        assert!(blockers[0].contains("db, cache"), "{blockers:?}");
    }

    #[test]
    fn swap_blockers_refuses_a_missing_service() {
        let config = merged(serde_json::json!({}));
        let blockers = swap_blockers(&config, "not-there");
        assert_eq!(blockers.len(), 1, "{blockers:?}");
        assert!(blockers[0].contains("not-there"), "{blockers:?}");
    }

    #[test]
    fn swap_blockers_ignores_host_ipc_and_pid_modes() {
        // Only the cases that silently change data or service discovery are
        // refused; the rest of the compose surface is not this check's job.
        let config = merged(serde_json::json!({"network_mode": "host", "ipc": "host"}));
        assert!(swap_blockers(&config, "web").is_empty());
    }
}
