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

use std::path::Path;

use crate::exec::StepSpec;

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
}
