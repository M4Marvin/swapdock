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
//! * [`verify`] — the access-log verdict: failures, upstreams, the flip instant.
//! * [`builder`] — local, SSH and no-op builders behind one resolution order.

pub mod apply;
pub mod builder;
pub mod cli;
pub mod compose;
pub mod deploy;
pub mod docker;
pub mod exec;
pub mod git;
pub mod health;
pub mod id;
pub mod lock;
pub mod ports;
pub mod redact;
pub mod registry;
pub mod render;
pub mod time;
pub mod trace;
pub mod tunnel;
pub mod validator;
pub mod verify;

pub use deploy::{Ctx, DeployError, rollback, run_replace, run_swap};
pub use id::RunId;
pub use redact::Redactor;
pub use trace::{Run, RunMode, RunStatus, StepStatus, TraceLog};
