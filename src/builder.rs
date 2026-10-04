//! Building releases on a pluggable build host.
//!
//! A release is `(image_repo, sha)`, independent of any machine. That is what
//! makes the build host a config value rather than a code path: the workstation
//! builds tonight, a VPS builds next month, the server builds in an emergency,
//! and all three produce the same artifact because the name is the commit.
//!
//! Three builders implement one trait:
//!
//! ```text
//! local  clone to a temp dir, docker build, push when the registry is remote
//! ssh    one ssh invocation running the same steps in a remote temp dir
//! none   the release already exists; verify it is there and stop
//! ```
//!
//! Resolution order for which one runs: `--build-host`, then
//! `DEPLOY_BUILD_HOST`, then the app's `build_host`, then local. The string
//! `local` and an empty value both mean this machine; anything else is an SSH
//! destination.
//!
//! `deploy build` prints the image reference and the release. It does not touch
//! the registry: building is not deploying, and conflating them is how a bad
//! build becomes a bad release without anyone deciding.

use std::path::{Path, PathBuf};
use std::time::Duration;

use thiserror::Error;

use crate::exec::StepSpec;
use crate::registry::App;
use crate::trace::Run;

/// How long a build may take. Compiling on a small box is slow; the timeout
/// guards against a wedged daemon, not a slow compiler.
pub const BUILD_TIMEOUT: Duration = Duration::from_secs(1800);

/// How long a push may take.
pub const PUSH_TIMEOUT: Duration = Duration::from_secs(600);

/// What building produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Built {
    pub image_ref: String,
    pub release: String,
}

/// Why a build did not finish.
#[derive(Debug, Error)]
pub enum BuildError {
    #[error("no release to build: pass --release, or sync the source first")]
    NoRelease,

    #[error("{0} has no repo recorded; set repo to the source checkout")]
    NoRepo(String),

    #[error("build failed: {0}")]
    Failed(String),

    #[error("push failed: {0}")]
    PushFailed(String),

    #[error("release {release} is not in the daemon as {image_ref}")]
    Missing { release: String, image_ref: String },

    #[error("could not run a build step: {0}")]
    Exec(#[from] crate::exec::ExecError),

    #[error("could not write to the run log: {0}")]
    Trace(#[from] anyhow::Error),

    #[error("could not prepare the build context: {0}")]
    Io(String),
}

/// Resolves which builder to use. `local` and empty both mean this machine.
pub fn resolve_host(flag: Option<&str>, env: Option<&str>, app: &App) -> String {
    flag.or(env)
        .or(app.build_host.as_deref())
        .unwrap_or("local")
        .to_string()
}

/// True when the host string means this machine.
pub fn is_local(host: &str) -> bool {
    host.trim().is_empty() || host.trim() == "local"
}

/// Dockerfile path for an app: `<repo>/Dockerfile` unless told otherwise.
pub fn dockerfile_for(app: &App) -> Option<PathBuf> {
    app.repo.as_ref().map(|repo| repo.join("Dockerfile"))
}

/// Builds `release` of `app` on this machine.
///
/// The source is cloned fresh into a temp dir at exactly the release commit, so
/// the build cannot include uncommitted work from a checkout. Pushes when the
/// registry is remote; a local-only image stops after the build.
pub fn build_local(
    run: &mut Run,
    app: &App,
    release: &str,
    work_parent: &Path,
) -> Result<Built, BuildError> {
    let repo_url = remote_url(app)?;
    let image_ref = app
        .image_ref(release)
        .ok_or_else(|| BuildError::NoRepo(app.name.clone()))?;

    let workdir = work_parent.join(format!("{}-{release}-build", app.name));
    if workdir.exists() {
        std::fs::remove_dir_all(&workdir)
            .map_err(|e| BuildError::Io(format!("clear {}: {e}", workdir.display())))?;
    }
    std::fs::create_dir_all(&workdir)
        .map_err(|e| BuildError::Io(format!("create {}: {e}", workdir.display())))?;

    // Clone exactly the release commit: `--depth 1 --branch` would take the
    // branch tip, which may have moved since the release was chosen.
    let clone = StepSpec::new("build-clone", "git")
        .args(["clone", &repo_url])
        .arg(workdir.display().to_string())
        .timeout(Duration::from_secs(300));
    let outcome = run.exec(&clone)?;
    if !outcome.success() {
        return Err(BuildError::Failed(format!(
            "clone failed: {}",
            outcome.stderr_trimmed()
        )));
    }
    let checkout = StepSpec::new("build-checkout", "git")
        .arg("-C")
        .arg(workdir.display().to_string())
        .args(["checkout", release])
        .timeout(Duration::from_secs(120));
    let outcome = run.exec(&checkout)?;
    if !outcome.success() {
        return Err(BuildError::Failed(format!(
            "checkout {release} failed: {}",
            outcome.stderr_trimmed()
        )));
    }

    let dockerfile = workdir.join("Dockerfile");
    if !dockerfile.exists() {
        return Err(BuildError::Failed(format!(
            "no Dockerfile at {}",
            dockerfile.display()
        )));
    }

    let build = StepSpec::new("docker-build", "docker")
        .args(["build", "-t", &image_ref, "."])
        .timeout(BUILD_TIMEOUT)
        .cwd(&workdir);
    let outcome = run.exec(&build)?;
    if !outcome.success() {
        return Err(BuildError::Failed(format!(
            "build failed: {}",
            outcome.stderr_trimmed()
        )));
    }

    if app.needs_pull() {
        let push = StepSpec::new("docker-push", "docker")
            .args(["push", &image_ref])
            .timeout(PUSH_TIMEOUT);
        let outcome = run.exec(&push)?;
        if !outcome.success() {
            return Err(BuildError::PushFailed(outcome.stderr_trimmed().to_string()));
        }
    }

    let _ = std::fs::remove_dir_all(&workdir);
    Ok(Built {
        image_ref,
        release: release.to_string(),
    })
}

/// The remote script `ssh` runs: clone, checkout, build, push, clean up.
///
/// Rendered here as text so it is testable without an SSH server. All work
/// happens in a fresh temp dir on the remote, so nothing about the remote's
/// layout is assumed except `git` and `docker` on `PATH`.
pub fn remote_build_script(repo_url: &str, release: &str, image_ref: &str, push: bool) -> String {
    let push_line = if push {
        format!("docker push {image_ref} || exit 1\n")
    } else {
        String::new()
    };
    format!(
        "set -e\n\
         work=$(mktemp -d)\n\
         trap 'rm -rf \"$work\"' EXIT\n\
         git clone {repo_url} \"$work\" >&2\n\
         git -C \"$work\" checkout {release} >&2\n\
         docker build -t {image_ref} \"$work\"\n\
         {push_line}"
    )
}

/// Builds `release` of `app` on `host` over SSH.
pub fn build_ssh(run: &mut Run, app: &App, release: &str, host: &str) -> Result<Built, BuildError> {
    let repo_url = remote_url(app)?;
    let image_ref = app
        .image_ref(release)
        .ok_or_else(|| BuildError::NoRepo(app.name.clone()))?;
    let script = remote_build_script(&repo_url, release, &image_ref, app.needs_pull());

    let spec = StepSpec::new("ssh-build", "ssh")
        .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=30", host, "--"])
        .arg(script)
        .timeout(BUILD_TIMEOUT);
    let outcome = run.exec(&spec)?;
    if !outcome.success() {
        return Err(BuildError::Failed(format!(
            "remote build failed: {}",
            outcome.stderr_trimmed()
        )));
    }
    Ok(Built {
        image_ref,
        release: release.to_string(),
    })
}

/// Verifies the release image is present without building anything.
pub fn build_none(run: &mut Run, app: &App, release: &str) -> Result<Built, BuildError> {
    let image_ref = app
        .image_ref(release)
        .ok_or_else(|| BuildError::NoRepo(app.name.clone()))?;
    let spec =
        StepSpec::new("docker-image-inspect", "docker").args(["image", "inspect", &image_ref]);
    let outcome = run.exec(&spec)?;
    if !outcome.success() {
        return Err(BuildError::Missing {
            release: release.to_string(),
            image_ref,
        });
    }
    Ok(Built {
        image_ref,
        release: release.to_string(),
    })
}

/// The clone URL for an app. Short `owner/repo` remotes mean GitHub.
fn remote_url(app: &App) -> Result<String, BuildError> {
    let remote = app
        .git_remote
        .as_ref()
        .ok_or_else(|| BuildError::NoRepo(app.name.clone()))?;
    if remote.contains("://") || remote.contains('@') {
        Ok(remote.clone())
    } else {
        Ok(format!("https://github.com/{remote}.git"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::tests::sample;

    #[test]
    fn host_resolution_order_is_flag_env_app_then_local() {
        let mut app = sample();
        app.build_host = Some("buildbox".into());
        assert_eq!(resolve_host(Some("flag"), Some("env"), &app), "flag");
        assert_eq!(resolve_host(None, Some("env"), &app), "env");
        assert_eq!(resolve_host(None, None, &app), "buildbox");

        app.build_host = None;
        assert_eq!(resolve_host(None, None, &app), "local");
    }

    #[test]
    fn local_and_empty_mean_this_machine() {
        assert!(is_local("local"));
        assert!(is_local(""));
        assert!(is_local("   "));
        assert!(!is_local("buildbox"));
        assert!(!is_local("192.168.1.10"));
    }

    #[test]
    fn remote_url_expands_short_remotes_to_github() {
        let app = sample();
        assert_eq!(
            remote_url(&app).unwrap(),
            "https://github.com/M4Marvin/main-site.git"
        );

        let mut full = sample();
        full.git_remote = Some("https://git.example.com/x/y.git".into());
        assert_eq!(
            remote_url(&full).unwrap(),
            "https://git.example.com/x/y.git"
        );

        let mut ssh = sample();
        ssh.git_remote = Some("git@git.example.com:x/y.git".into());
        assert_eq!(remote_url(&ssh).unwrap(), "git@git.example.com:x/y.git");

        let mut none = sample();
        none.git_remote = None;
        assert!(matches!(
            remote_url(&none).unwrap_err(),
            BuildError::NoRepo(_)
        ));
    }

    #[test]
    fn remote_build_script_clones_checks_out_builds_and_pushes() {
        let script = remote_build_script(
            "https://github.com/M4Marvin/main-site.git",
            "9c1f2ab",
            "ghcr.io/m4marvin/main-site:9c1f2ab",
            true,
        );
        assert!(script.starts_with("set -e\n"), "{script}");
        assert!(script.contains("mktemp -d"), "{script}");
        assert!(script.contains("trap "), "must clean up: {script}");
        assert!(
            script.contains("git clone https://github.com/M4Marvin/main-site.git"),
            "{script}"
        );
        assert!(script.contains("checkout 9c1f2ab"), "{script}");
        assert!(
            script.contains("docker build -t ghcr.io/m4marvin/main-site:9c1f2ab"),
            "{script}"
        );
        assert!(
            script.contains("docker push ghcr.io/m4marvin/main-site:9c1f2ab"),
            "{script}"
        );
    }

    #[test]
    fn remote_build_script_skips_push_for_local_images() {
        let script = remote_build_script("https://x/y.git", "abc1234", "apps-web:abc1234", false);
        assert!(
            script.contains("docker build -t apps-web:abc1234"),
            "{script}"
        );
        assert!(!script.contains("docker push"), "{script}");
    }

    #[test]
    fn dockerfile_defaults_to_the_repo_root() {
        let app = sample();
        assert_eq!(
            dockerfile_for(&app),
            Some(std::path::PathBuf::from(
                "/home/marv/apps/main-site/Dockerfile"
            ))
        );

        let mut none = sample();
        none.repo = None;
        assert_eq!(dockerfile_for(&none), None);
    }
}
