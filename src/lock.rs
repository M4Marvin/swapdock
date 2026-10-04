//! Advisory file locks.
//!
//! Two deploys must never interleave: one rewriting the nginx config while
//! another renames a registry file is how state corrupts across apps. `flock`
//! is the right primitive because the kernel releases it when the holder dies,
//! so a killed deploy cannot wedge every later one.
//!
//! Two lock domains, matching the two kinds of shared state:
//!
//! * one file per app (`deploy-<app>.lock`) around build, start and health gate;
//! * one global file (`deploy-nginx.lock`) around generate, test and reload.
//!
//! Two apps build and health-gate in parallel and serialize only for the few
//! hundred milliseconds of generate-and-reload. That is the minimum contention
//! this design can get away with.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fs2::FileExt;

/// How long to wait for a lock before giving up.
pub const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(300);

/// How long to sleep between acquisition attempts.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Why a lock could not be taken.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LockError {
    #[error("could not create lock file {path}: {reason}")]
    Create { path: String, reason: String },

    #[error("timed out after {timeout_ms} ms waiting for {path}; another deploy holds it")]
    Timeout { path: String, timeout_ms: u64 },
}

/// An exclusive lock, released when dropped.
#[derive(Debug)]
pub struct Lock {
    path: PathBuf,
    // Held open for the life of the guard: closing releases the flock.
    _file: File,
}

impl Lock {
    /// Acquires an exclusive lock on `dir/<name>.lock`, creating the directory
    /// and the file as needed. Blocks up to `timeout`.
    pub fn acquire(dir: &Path, name: &str, timeout: Duration) -> Result<Self, LockError> {
        std::fs::create_dir_all(dir).map_err(|e| LockError::Create {
            path: dir.display().to_string(),
            reason: e.to_string(),
        })?;

        let path = dir.join(format!("{name}.lock"));
        // No truncate: the file's only purpose is to be locked, and its
        // contents are never read.
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| LockError::Create {
                path: path.display().to_string(),
                reason: e.to_string(),
            })?;

        let started = Instant::now();
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(Self { path, _file: file }),
                Err(_) => {
                    if started.elapsed() >= timeout {
                        return Err(LockError::Timeout {
                            path: path.display().to_string(),
                            timeout_ms: timeout.as_millis() as u64,
                        });
                    }
                    std::thread::sleep(POLL_INTERVAL);
                }
            }
        }
    }

    /// What file this lock holds.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Lock file name for one app's deploy.
pub fn app_lock_name(app: &str) -> String {
    format!("deploy-{app}")
}

/// Lock file name for the nginx critical section.
pub fn nginx_lock_name() -> &'static str {
    "deploy-nginx"
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn acquire_and_release() {
        let dir = TempDir::new().unwrap();
        let lock = Lock::acquire(dir.path(), "deploy-test", Duration::from_secs(5)).unwrap();
        assert_eq!(lock.path(), dir.path().join("deploy-test.lock"));
        assert!(lock.path().exists());
        drop(lock);

        // Re-acquirable immediately after the guard drops.
        Lock::acquire(dir.path(), "deploy-test", Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn a_second_holder_times_out() {
        let dir = TempDir::new().unwrap();
        let _first = Lock::acquire(dir.path(), "deploy-test", Duration::from_secs(5)).unwrap();

        let started = Instant::now();
        let err = Lock::acquire(dir.path(), "deploy-test", Duration::from_millis(300)).unwrap_err();
        assert!(
            matches!(err, LockError::Timeout { .. }),
            "expected a timeout, got {err:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "must give up at the deadline, took {:?}",
            started.elapsed()
        );
        assert!(err.to_string().contains("another deploy holds it"), "{err}");
    }

    #[test]
    fn different_names_do_not_contend() {
        let dir = TempDir::new().unwrap();
        let _a = Lock::acquire(dir.path(), "deploy-a", Duration::from_secs(5)).unwrap();
        let _b = Lock::acquire(dir.path(), "deploy-b", Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn a_missing_directory_is_created() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("a").join("b");
        Lock::acquire(&nested, "deploy-test", Duration::from_secs(5)).unwrap();
        assert!(nested.join("deploy-test.lock").exists());
    }

    #[test]
    fn an_unwritable_directory_is_an_error_not_a_hang() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let ro = dir.path().join("ro");
        std::fs::create_dir(&ro).unwrap();
        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o555)).unwrap();

        // Root can write anywhere regardless of mode bits, so probe first.
        let writable = std::fs::write(ro.join(".probe"), b"x").is_ok();
        let _ = std::fs::remove_file(ro.join(".probe"));
        if !writable {
            let err = Lock::acquire(&ro, "deploy-test", Duration::from_secs(5)).unwrap_err();
            assert!(matches!(err, LockError::Create { .. }), "{err:?}");
        }

        std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn lock_names_follow_the_convention() {
        assert_eq!(app_lock_name("portfolio"), "deploy-portfolio");
        assert_eq!(nginx_lock_name(), "deploy-nginx");
    }
}
