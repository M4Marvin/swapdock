//! `deploy` — a deploy tool for a multi-app Docker host.
//!
//! The crate is split so that the one component which can affect every service
//! at once — the nginx config renderer — stays a pure function that needs no
//! Docker, no nginx and no root to test. See [`render`] once it lands.
//!
//! Milestone 1 delivers the three foundations everything else assumes:
//!
//! * [`exec`] — the subprocess chokepoint. The only place in the crate that may
//!   create a process. Enforced by `tests/single_spawn_point.rs`.
//! * [`trace`] — the append-only JSONL run log: traceable, resumable, observable.
//! * [`redact`] — secret masking, applied inside the chokepoint before anything
//!   can reach a file.

pub mod cli;
pub mod exec;
pub mod id;
pub mod redact;
pub mod time;
pub mod trace;

pub use id::RunId;
pub use redact::Redactor;
pub use trace::{Run, RunMode, RunStatus, StepStatus, TraceLog};
