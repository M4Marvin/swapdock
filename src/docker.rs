//! `docker` argument builders.
//!
//! Same contract as `compose.rs`: pure argv construction, execution left to the
//! chokepoint. These cover what compose cannot express: finding a container by
//! the port it publishes (so the tool never assumes a container name), pulling
//! a release by digest-free tag, and reading health state.

use crate::exec::StepSpec;

/// Names of containers publishing `port` on the host.
///
/// The tool discovers containers by port rather than by name because the registry
/// does not store container names and compose's `container_name` pins are a
/// host-side detail. `--format {{.Names}}` keeps the output parseable.
pub fn ps_publishing(port: u16) -> StepSpec {
    StepSpec::new("docker-ps", "docker")
        .args(["ps", "--format", "{{.Names}}", "--filter"])
        .arg(format!("publish={port}"))
}

/// Pulls an image reference.
pub fn pull(image_ref: &str) -> StepSpec {
    StepSpec::new("docker-pull", "docker")
        .arg("pull")
        .arg(image_ref)
}

/// Reads `Health.Status` and `State.Status` for one container, space-separated.
///
/// When the image has no healthcheck, the first field renders as `<no value>`,
/// which the health gate treats as a hard failure rather than a pass.
pub fn inspect_health(container: &str) -> StepSpec {
    StepSpec::new("docker-inspect", "docker")
        .args([
            "inspect",
            "--format",
            "{{.State.Health.Status}} {{.State.Status}}",
        ])
        .arg(container)
}

/// Stops one container by name or id.
pub fn stop(container: &str) -> StepSpec {
    StepSpec::new("docker-stop", "docker")
        .arg("stop")
        .arg(container)
}

/// Removes one container by name or id.
pub fn rm(container: &str) -> StepSpec {
    StepSpec::new("docker-rm", "docker").args(["rm", container])
}

/// Tags a local image, used by tests to fabricate a release.
pub fn tag(source: &str, target: &str) -> StepSpec {
    StepSpec::new("docker-tag", "docker").args(["tag", source, target])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ps_filters_by_published_port() {
        let argv = ps_publishing(9004).full_argv();
        assert_eq!(
            argv,
            [
                "docker",
                "ps",
                "--format",
                "{{.Names}}",
                "--filter",
                "publish=9004"
            ]
        );
    }

    #[test]
    fn inspect_reads_both_health_and_state() {
        let argv = inspect_health("portfolio-green").full_argv();
        assert!(
            argv.contains(&"{{.State.Health.Status}} {{.State.Status}}".to_string()),
            "{argv:?}"
        );
    }

    #[test]
    fn pull_tags_and_removals_carry_the_reference() {
        assert!(
            pull("ghcr.io/example-org/portfolio:9c1f2ab")
                .full_argv()
                .ends_with(&["ghcr.io/example-org/portfolio:9c1f2ab".to_string()])
        );
        assert!(
            stop("portfolio-green")
                .full_argv()
                .ends_with(&["portfolio-green".to_string()])
        );
        assert!(
            rm("portfolio-green")
                .full_argv()
                .ends_with(&["portfolio-green".to_string()])
        );
        assert_eq!(
            tag("nginx:alpine", "test:abc1234").full_argv(),
            ["docker", "tag", "nginx:alpine", "test:abc1234"]
        );
    }

    #[test]
    fn step_names_are_stable() {
        assert_eq!(ps_publishing(1).name, "docker-ps");
        assert_eq!(pull("x").name, "docker-pull");
        assert_eq!(inspect_health("x").name, "docker-inspect");
        assert_eq!(stop("x").name, "docker-stop");
        assert_eq!(rm("x").name, "docker-rm");
    }
}
