//! Durable in-flight deploy records.
//!
//! The window that loses traffic records is between the nginx flip and the
//! registry commit. A process killed there leaves the registry describing the
//! old release while nginx serves the new one. A pending record, written and
//! fsynced before the flip and removed only after the commit, gives the
//! operator (and a later `status`) a truthful "this run was in flight" signal
//! instead of a confident lie.
//!
//! This is deliberately small: one JSON document per app under
//! `<state_dir>/pending/<app>.json`. It is not a general transaction log and it
//! does not replace the run log.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::time::Timestamp;

/// How far the deploy got before it stopped writing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Green is healthy; the registry/nginx is about to move.
    Flipping,
    /// Nginx has been asked to load the new config.
    Flipped,
    /// The old container is being stopped; the commit is next.
    Stopping,
}

/// A deploy that has started affecting the host but has not committed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    pub app: String,
    pub release: String,
    pub from_port: Option<u16>,
    pub to_port: u16,
    pub phase: Phase,
    pub run_id: String,
    pub started: String,
}

impl Pending {
    /// Creates a record for a swap/replace in progress.
    pub fn new(
        app: impl Into<String>,
        release: impl Into<String>,
        from_port: Option<u16>,
        to_port: u16,
        run_id: impl Into<String>,
    ) -> Self {
        Self {
            app: app.into(),
            release: release.into(),
            from_port,
            to_port,
            phase: Phase::Flipping,
            run_id: run_id.into(),
            started: Timestamp::now().to_rfc3339(),
        }
    }

    /// Advances the phase; the caller writes it again.
    pub fn at(mut self, phase: Phase) -> Self {
        self.phase = phase;
        self
    }
}

/// Directory holding pending records.
pub fn dir(state_dir: &Path) -> PathBuf {
    state_dir.join("pending")
}

/// Path for one app's pending record.
pub fn path(state_dir: &Path, app: &str) -> PathBuf {
    dir(state_dir).join(format!("{app}.json"))
}

/// Writes and fsyncs a pending record (file and directory).
pub fn write(state_dir: &Path, pending: &Pending) -> std::io::Result<()> {
    use std::io::Write;

    let dir = dir(state_dir);
    std::fs::create_dir_all(&dir)?;
    let target = path(state_dir, &pending.app);
    let tmp = dir.join(format!(".{}.tmp.{}", pending.app, std::process::id()));
    let mut file = std::fs::File::create(&tmp)?;
    let body = serde_json::to_vec_pretty(pending)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    file.write_all(&body)?;
    file.sync_all()?;
    std::fs::rename(&tmp, &target)?;
    // Persist the directory entry, or the rename can vanish on power loss.
    if let Ok(dir) = std::fs::File::open(&dir) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Reads a pending record, if one exists.
pub fn read(state_dir: &Path, app: &str) -> std::io::Result<Option<Pending>> {
    let target = path(state_dir, app);
    match std::fs::read(&target) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Removes a completed/interrupted record. Missing is success.
pub fn clear(state_dir: &Path, app: &str) -> std::io::Result<()> {
    match std::fs::remove_file(path(state_dir, app)) {
        Ok(()) => {
            if let Ok(dir) = std::fs::File::open(dir(state_dir)) {
                let _ = dir.sync_all();
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_reads_and_clears() {
        let dir = tempfile::TempDir::new().unwrap();
        let pending = Pending::new("chat", "abc1234", Some(9000), 9001, "run1");
        write(dir.path(), &pending).unwrap();
        let record = read(dir.path(), "chat").unwrap().unwrap();
        assert_eq!(record, pending);
        assert_eq!(record.phase, Phase::Flipping);

        write(dir.path(), &pending.clone().at(Phase::Flipped)).unwrap();
        assert_eq!(
            read(dir.path(), "chat").unwrap().unwrap().phase,
            Phase::Flipped
        );

        clear(dir.path(), "chat").unwrap();
        assert!(read(dir.path(), "chat").unwrap().is_none());
        // Clearing twice is harmless.
        clear(dir.path(), "chat").unwrap();
    }

    #[test]
    fn missing_record_is_none_not_an_error() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(read(dir.path(), "nothing").unwrap().is_none());
    }

    #[test]
    fn malformed_record_is_an_error_not_a_silent_none() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("pending")).unwrap();
        std::fs::write(dir.path().join("pending/chat.json"), "{").unwrap();
        assert!(read(dir.path(), "chat").is_err());
    }

    #[test]
    fn phase_serializes_snake_case() {
        let json = serde_json::to_value(Phase::Stopping).unwrap();
        assert_eq!(json, "stopping");
    }
}
