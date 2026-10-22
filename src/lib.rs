//! `swapdock` — registry-driven blue/green deploys for a multi-app Docker host.
//!
//! The crate is split so that the one component which can affect every service
//! at once — the nginx config renderer ([`render`]) — stays a pure function
//! that needs no Docker, no nginx and no root to test.
//!
//! Foundations:
//!
//! * [`exec`] — the subprocess chokepoint. The only place in the crate that may
//!   create a process. Enforced by `tests/single_spawn_point.rs`.
//! * [`trace`] — the append-only JSONL run log: traceable, resumable, observable.
//! * [`redact`] — secret masking, applied inside the chokepoint before anything
//!   can reach a file.
//! * [`registry`], [`validator`], [`render`] — the source of truth, the
//!   cross-app checks, and the config renderer as a pure function.
//! * [`apply`] — atomic file replacement with `nginx -t` gating and restore.
//! * [`deploy`] — the swap and replace strategies as explicit step sequences.
//! * [`import`] — Compose onboarding: detect a service, allocate ports, and
//!   write a registry entry plus a generated override without editing Compose.
//! * [`verify`] — the access-log verdict: failures, upstreams, the flip instant.
//! * [`builder`] — local, SSH and no-op builders behind one resolution order.

pub mod api;
pub mod apply;
pub mod auth;
pub mod backup;
pub mod broker;
pub mod builder;
pub mod cli;
pub mod compose;
pub mod deploy;
pub mod docker;
pub mod doctor;
pub mod exec;
pub mod git;
pub mod health;
pub mod id;
pub mod import;
pub mod journal;
pub mod lock;
pub mod monitor;
pub mod notify;
pub mod ports;
pub mod prune;
pub mod reconcile;
pub mod redact;
pub mod registry;
pub mod render;
pub mod ssh;
pub mod static_site;
pub mod time;
pub mod trace;
pub mod transfer;
pub mod tunnel;
pub mod validator;
pub mod verify;

pub use deploy::{DeployContext, DeployError, rollback, run_replace, run_swap};
pub use id::RunId;
pub use redact::Redactor;
pub use trace::{Run, RunMode, RunStatus, StepStatus, TraceLog};

/// Process-wide environment helpers for this crate's unit tests.
///
/// Unit tests run on parallel threads in one process, and `std::env::set_var`
/// is unsafe in edition 2024 because a write races any read of the environment.
/// Every test that mutates a variable goes through [`set_var`] or
/// [`remove_var`], and every spawn takes [`reading`], so the two never overlap.
/// A test that needs its variables left alone for its whole body also takes
/// [`own`], which keeps other owners out.
#[cfg(test)]
pub(crate) mod test_env {
    use std::ffi::OsStr;
    use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard};

    /// Held for the whole body of a test that owns a group of variables.
    static OWNER: Mutex<()> = Mutex::new(());
    /// Spawns read the environment, mutations write it. Never held by a
    /// mutating test across its own spawn: only for the few instructions of
    /// the write.
    static ENV: RwLock<()> = RwLock::new(());

    /// Blocks until no other test owns the environment.
    pub(crate) fn own() -> MutexGuard<'static, ()> {
        // A panicking test must not fail the rest of the suite.
        OWNER.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Blocks while a mutation is in progress; hold it across the spawn.
    pub(crate) fn reading() -> RwLockReadGuard<'static, ()> {
        ENV.read().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn set_var(key: &str, value: impl AsRef<OsStr>) {
        let _guard = ENV.write().unwrap_or_else(|e| e.into_inner());
        // SAFETY: the write lock is held, and every environment read in this
        // crate's tests takes `reading()` first.
        unsafe { std::env::set_var(key, value) };
    }

    pub(crate) fn remove_var(key: &str) {
        let _guard = ENV.write().unwrap_or_else(|e| e.into_inner());
        // SAFETY: as in `set_var`.
        unsafe { std::env::remove_var(key) };
    }
}
