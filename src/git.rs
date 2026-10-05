//! `git` argument builders.
//!
//! The tool needs exactly four git operations: fetch, resolve a branch to a
//! commit, fast-forward the checkout, and check cleanliness. Shelling out keeps
//! the server's git version irrelevant after the build moves off the box, and
//! costs nothing at four commands.

use std::path::Path;

use crate::exec::StepSpec;

/// Fetches one branch from origin into the checkout at `dir`.
pub fn fetch(dir: &Path, branch: &str) -> StepSpec {
    StepSpec::new("git-fetch", "git")
        .arg("-C")
        .arg(dir.display().to_string())
        .args(["fetch", "origin", branch])
}

/// Resolves `origin/<branch>` to a commit name, without touching the checkout.
pub fn rev_parse(dir: &Path, branch: &str) -> StepSpec {
    StepSpec::new("git-rev-parse", "git")
        .arg("-C")
        .arg(dir.display().to_string())
        .args(["rev-parse", &format!("origin/{branch}")])
}

/// Fast-forwards the checkout to `origin/<branch>`. Refuses when that is not a
/// fast-forward, which is the correct behaviour for a swapdock source: history on
/// the server must never diverge.
pub fn merge_ff_only(dir: &Path, branch: &str) -> StepSpec {
    StepSpec::new("git-merge", "git")
        .arg("-C")
        .arg(dir.display().to_string())
        .args(["merge", "--ff-only", &format!("origin/{branch}")])
}

/// Lists uncommitted changes, empty when clean.
pub fn status_porcelain(dir: &Path) -> StepSpec {
    StepSpec::new("git-status", "git")
        .arg("-C")
        .arg(dir.display().to_string())
        .args(["status", "--porcelain"])
}

/// Clones one branch into `dir`. Only used when the checkout does not exist yet.
pub fn clone_branch(url: &str, branch: &str, dir: &Path) -> StepSpec {
    StepSpec::new("git-clone", "git")
        .args(["clone", "--depth", "1", "--branch", branch, url])
        .arg(dir.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dir() -> PathBuf {
        PathBuf::from("/home/marv/apps/main-site")
    }

    #[test]
    fn fetch_targets_one_branch() {
        assert_eq!(
            fetch(&dir(), "master").full_argv(),
            [
                "git",
                "-C",
                "/home/marv/apps/main-site",
                "fetch",
                "origin",
                "master"
            ]
        );
    }

    #[test]
    fn rev_parse_resolves_the_remote_branch() {
        let argv = rev_parse(&dir(), "main").full_argv();
        assert!(
            argv.ends_with(&["rev-parse".to_string(), "origin/main".to_string()]),
            "{argv:?}"
        );
    }

    #[test]
    fn merge_is_fast_forward_only() {
        let argv = merge_ff_only(&dir(), "main").full_argv();
        assert!(argv.contains(&"--ff-only".to_string()), "{argv:?}");
        assert!(!argv.contains(&"--no-ff".to_string()), "{argv:?}");
    }

    #[test]
    fn clone_is_shallow_and_single_branch() {
        let argv = clone_branch(
            "https://github.com/M4Marvin/main-site.git",
            "master",
            &PathBuf::from("/srv/src/main-site"),
        )
        .full_argv();
        assert_eq!(
            argv,
            [
                "git",
                "clone",
                "--depth",
                "1",
                "--branch",
                "master",
                "https://github.com/M4Marvin/main-site.git",
                "/srv/src/main-site"
            ]
        );
    }

    #[test]
    fn step_names_are_stable() {
        assert_eq!(fetch(&dir(), "b").name, "git-fetch");
        assert_eq!(rev_parse(&dir(), "b").name, "git-rev-parse");
        assert_eq!(merge_ff_only(&dir(), "b").name, "git-merge");
        assert_eq!(status_porcelain(&dir()).name, "git-status");
        assert_eq!(clone_branch("u", "b", &dir()).name, "git-clone");
    }
}
