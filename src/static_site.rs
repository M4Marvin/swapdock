//! Static-app releases: the symlink side of a static deployment.
//!
//! A static app's registry `root` is a stable path (for example
//! `/srv/www/docs/current`) that nginx serves. Releases live beside it:
//!
//! ```text
//! /srv/www/docs/releases/<release>/index.html
//! /srv/www/docs/current -> releases/<release>
//! ```
//!
//! Flipping `current` is one `rename(2)` of a temporary symlink, so a request
//! sees either the old release or the new one, never a mix. The release
//! directory is created by the build/rsync step; this module only verifies it,
//! flips the link, and can restore the previous link if the post-flip probe
//! fails.

use std::path::{Path, PathBuf};

/// Why a static release could not be prepared or flipped.
#[derive(Debug, thiserror::Error)]
pub enum StaticError {
    #[error("static app has no root configured")]
    NoRoot,

    #[error("static root {root} is not a symlink; create it as a symlink to releases/<release>")]
    RootNotSymlink { root: String },

    #[error(
        "static release {release} is missing; expected a directory at {path} with an index.html"
    )]
    ReleaseMissing { release: String, path: String },

    #[error("could not read the current symlink {root}: {reason}")]
    ReadLink { root: String, reason: String },

    #[error("could not create the release symlink {link}: {reason}")]
    Symlink { link: String, reason: String },

    #[error("could not atomically flip {root}: {reason}")]
    Flip { root: String, reason: String },

    #[error("could not restore the previous symlink {root}: {reason}")]
    Restore { root: String, reason: String },
}

/// The directory a release must occupy for a given static app.
pub fn release_dir(root: &Path, release: &str) -> Result<PathBuf, StaticError> {
    if !is_valid_release_component(release) {
        return Err(StaticError::ReleaseMissing {
            release: release.to_string(),
            path: root.display().to_string(),
        });
    }
    let parent = root.parent().ok_or(StaticError::NoRoot)?;
    Ok(parent.join("releases").join(release))
}

/// Verifies the release exists and has an index. A static release without an
/// index would flip to a directory nginx cannot serve.
pub fn ensure_release(root: &Path, release: &str) -> Result<PathBuf, StaticError> {
    let dir = release_dir(root, release)?;
    if !dir.is_dir() || !dir.join("index.html").is_file() {
        return Err(StaticError::ReleaseMissing {
            release: release.to_string(),
            path: dir.display().to_string(),
        });
    }
    Ok(dir)
}

/// Flips `root` to `releases/<release>` and returns the previous link target
/// (if any) so a failed post-flip probe can restore it.
///
/// The root may be absent on the first deploy; it is created. If it exists as
/// anything other than a symlink, the deploy refuses rather than replacing a
/// directory with a link.
pub fn flip(root: &Path, release: &str) -> Result<Option<PathBuf>, StaticError> {
    ensure_release(root, release)?;
    let parent = root.parent().ok_or(StaticError::NoRoot)?;
    std::fs::create_dir_all(parent).map_err(|e| StaticError::Flip {
        root: root.display().to_string(),
        reason: e.to_string(),
    })?;

    let previous = match std::fs::symlink_metadata(root) {
        Ok(meta) if meta.file_type().is_symlink() => {
            Some(std::fs::read_link(root).map_err(|e| StaticError::ReadLink {
                root: root.display().to_string(),
                reason: e.to_string(),
            })?)
        }
        Ok(_) => {
            return Err(StaticError::RootNotSymlink {
                root: root.display().to_string(),
            });
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            return Err(StaticError::ReadLink {
                root: root.display().to_string(),
                reason: e.to_string(),
            });
        }
    };

    let target = Path::new("releases").join(release);
    // The temporary link lives in the same directory, so rename(2) is atomic.
    let stamp = std::process::id();
    let tmp = parent.join(format!(
        ".{}.tmp.{stamp}",
        root.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("current")
    ));
    let _ = std::fs::remove_file(&tmp);
    std::os::unix::fs::symlink(&target, &tmp).map_err(|e| StaticError::Symlink {
        link: tmp.display().to_string(),
        reason: e.to_string(),
    })?;
    if let Err(e) = std::fs::rename(&tmp, root) {
        let _ = std::fs::remove_file(&tmp);
        return Err(StaticError::Flip {
            root: root.display().to_string(),
            reason: e.to_string(),
        });
    }
    std::fs::File::open(parent)
        .map_err(|e| StaticError::Flip {
            root: root.display().to_string(),
            reason: format!("fsync parent: {e}"),
        })?
        .sync_all()
        .map_err(|e| StaticError::Flip {
            root: root.display().to_string(),
            reason: format!("fsync parent: {e}"),
        })?;
    Ok(previous)
}

/// Restores a previous link target after a failed post-flip probe.
pub fn restore(root: &Path, previous: Option<&Path>) -> Result<(), StaticError> {
    let Some(previous) = previous else {
        // There was no previous link: remove the one we created, so a failed
        // first deploy does not pretend to serve a release.
        match std::fs::remove_file(root) {
            Ok(()) => {
                if let Some(parent) = root.parent() {
                    let _ = std::fs::File::open(parent).and_then(|f| f.sync_all());
                }
                return Ok(());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                return Err(StaticError::Restore {
                    root: root.display().to_string(),
                    reason: e.to_string(),
                });
            }
        }
    };
    let parent = root.parent().ok_or(StaticError::NoRoot)?;
    let stamp = std::process::id();
    let tmp = parent.join(format!(
        ".{}.restore.{stamp}",
        root.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("current")
    ));
    let _ = std::fs::remove_file(&tmp);
    std::os::unix::fs::symlink(previous, &tmp).map_err(|e| StaticError::Symlink {
        link: tmp.display().to_string(),
        reason: e.to_string(),
    })?;
    std::fs::rename(&tmp, root).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        StaticError::Restore {
            root: root.display().to_string(),
            reason: e.to_string(),
        }
    })?;
    std::fs::File::open(parent)
        .map_err(|e| StaticError::Restore {
            root: root.display().to_string(),
            reason: format!("fsync parent: {e}"),
        })?
        .sync_all()
        .map_err(|e| StaticError::Restore {
            root: root.display().to_string(),
            reason: format!("fsync parent: {e}"),
        })?;
    Ok(())
}

/// A release becomes a path component, so it must not contain separators,
/// traversal or shell/nginx metacharacters. The registry already validates
/// releases; this is the second gate at the filesystem boundary.
fn is_valid_release_component(release: &str) -> bool {
    !release.is_empty() && release.len() <= 64 && release.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs as unix_fs;

    fn estate() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().join("current");
        std::fs::create_dir_all(dir.path().join("releases").join("abc1234")).unwrap();
        std::fs::write(dir.path().join("releases/abc1234/index.html"), "first").unwrap();
        std::fs::create_dir_all(dir.path().join("releases").join("def5678")).unwrap();
        std::fs::write(dir.path().join("releases/def5678/index.html"), "second").unwrap();
        (dir, root)
    }

    #[test]
    fn flips_and_restores_a_static_release() {
        let (dir, root) = estate();
        let previous = flip(&root, "abc1234").unwrap();
        assert!(previous.is_none());
        assert_eq!(
            std::fs::read_link(&root).unwrap(),
            PathBuf::from("releases/abc1234")
        );
        assert_eq!(
            std::fs::read_to_string(root.join("index.html")).unwrap(),
            "first"
        );

        let previous = flip(&root, "def5678").unwrap();
        assert_eq!(previous, Some(PathBuf::from("releases/abc1234")));
        assert_eq!(
            std::fs::read_to_string(root.join("index.html")).unwrap(),
            "second"
        );

        restore(&root, previous.as_deref()).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("index.html")).unwrap(),
            "first"
        );
        drop(dir);
    }

    #[test]
    fn refuses_a_missing_release() {
        let (dir, root) = estate();
        let err = flip(&root, "fffffff").unwrap_err();
        assert!(matches!(err, StaticError::ReleaseMissing { .. }), "{err}");
        drop(dir);
    }

    #[test]
    fn refuses_a_root_that_is_a_real_directory() {
        let (dir, root) = estate();
        std::fs::create_dir_all(&root).unwrap();
        let err = flip(&root, "abc1234").unwrap_err();
        assert!(matches!(err, StaticError::RootNotSymlink { .. }), "{err}");
        drop(dir);
    }

    #[test]
    fn first_deploy_without_a_root_creates_it_and_can_remove_it_on_failure() {
        let (dir, root) = estate();
        let previous = flip(&root, "abc1234").unwrap();
        assert!(previous.is_none());
        restore(&root, None).unwrap();
        assert!(!root.exists());
        drop(dir);
    }

    #[test]
    fn rejects_path_traversal_in_a_release() {
        let (dir, root) = estate();
        assert!(release_dir(&root, "../etc").is_err());
        assert!(release_dir(&root, "abc1234").is_ok());
        drop(dir);
    }

    #[test]
    fn a_symlink_to_a_directory_still_reads_the_link_target() {
        let (dir, _root) = estate();
        let other = dir.path().join("link");
        unix_fs::symlink("releases/abc1234", &other).unwrap();
        assert_eq!(
            std::fs::read_link(&other).unwrap(),
            PathBuf::from("releases/abc1234")
        );
        drop(dir);
    }
}
