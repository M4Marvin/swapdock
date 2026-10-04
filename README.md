# deploy

A deploy tool for a multi-app Docker host: one registry as the source of truth,
an nginx front door, blue/green releases with a health gate, and a build host
that is a config value rather than a code path.

Design notes and the full plan live alongside the video essay in
`~/codes/deploy-explainer/`.

## Status: milestone 1 complete

The three foundations everything else assumes. No Docker, no nginx and no root
are required for any of it, which is why it could be built and tested first.

| Module | Purpose |
|---|---|
| `src/exec.rs` | The subprocess chokepoint. The only place in the crate that may create a process. |
| `src/trace.rs` | Append-only JSONL run log: traceable, resumable, observable. |
| `src/redact.rs` | Secret masking, applied inside the chokepoint before anything reaches a file. |
| `src/time.rs` | RFC 3339 timestamps, no dependency. |
| `src/id.rs` | Sortable, unique run identifiers. |
| `src/cli.rs` | `selftest`, `runs`, `show`, `resume`. |

Later milestones: `registry` + `validator` + `render` (pure functions),
`apply` with atomic writes, the two deploy strategies, the `Builder` trait.

## The one rule

`exec.rs` is the only module that may spawn a process. That single fact is what
makes logging, dry-run, redaction and timeouts possible in one place instead of
four scattered call sites.

The rule is enforced, not documented:

```
$ cargo test --test single_spawn_point
```

That test greps `src/` for `process::Command`, `Command::new` and `Stdio` and
fails with the file and line if any module other than `exec.rs` touches them. It
is verified to fail when the rule is broken.

## Try it

```bash
cargo run -- selftest          # exercise every chokepoint guarantee
cargo run -- runs              # recent runs, newest first
cargo run -- show <run-id>     # every step of one run, in order
cargo run -- resume <run-id>   # where a run stopped, and whether it ended
cargo run -- --dry-run selftest
```

`selftest` proves, and the table it prints says what each case proves:

| Case | Guarantee |
|---|---|
| success | exit 0, stdout captured |
| nonzero-exit | a non-zero exit is a result, not a tool error |
| stderr | kept separate from stdout |
| no-shell | no globbing, splitting or expansion — there is no shell |
| large-output | ~1.2 MB drained without deadlock, tail retained |
| timeout | killed at the deadline instead of hanging |
| missing-program | spawn failure reported, not panicked |
| redaction | a token in argv, in env and in the child's own output never reaches the log |

## Design decisions worth knowing

**A non-zero exit is not an error.** `nginx -t` failing is fatal; some other
command returning 1 may not be. The chokepoint reports what happened and lets the
caller decide. Only a spawn failure, a timeout or a broken log is a tool error.

**Both pipes are drained on threads.** Polling `try_wait()` while nobody reads
the pipes deadlocks the moment a child writes past one pipe buffer (64 KiB).
`docker compose up` does that easily. Covered by `large_output_does_not_deadlock`.

**The output cap is a real guarantee.** `from_utf8_lossy` expands each invalid
byte to a three-byte replacement character, so a byte cap alone is not enough.
The decoded string is capped again and trimmed to a char boundary, because it
goes into JSON.

**Torn writes are tolerated.** A power cut mid-append leaves a partial final
line. The reader skips unparseable lines and reports a count instead of refusing
to start, because refusing to start turns a crash into an outage.

**Redaction keeps no secret characters.** Registered secrets are labelled with an
FNV-1a fingerprint, so a log still tells you *which* credential was in use.
`Redactor` has a hand-written `Debug` for the same reason: a derived one would
print the secret. Display paths go through `stdout_safe`/`stderr_safe`, because
a token in a terminal scrollback is a leak even when the log is clean.

**`-p` is not a password.** In `docker run -p 8001:80` it is a port. Only
`--password`, `--token`, `--secret`, `--api-key`, `--auth-token` and
`--client-secret` mask the following argument.

## Build

```bash
cargo build --release          # ~1.6 MB, stripped
cargo test                     # 82 tests
cargo clippy --all-targets     # zero warnings
cargo fmt
```

Rust 1.98+ (edition 2024). Dependencies are kept to `clap`, `serde`,
`serde_json`, `anyhow`, `thiserror`, `tracing`, `tracing-subscriber`; timestamps
and run ids are implemented in-crate to avoid two more.

### mr-boxington

Rust builds are slow enough to want a shared cache, so the toolchain uses
[mr-boxington](https://mr-boxington.jdx.dev/) (installed via
`mise use --tool-option mr_boxington=true mr-boxington`).

```bash
mbx doctor         # all checks
mbx explain --last # why the last build hit or missed
mbx cache trace <session>
mbx stats --json
mbx tui            # live hit/miss, store capacity, time saved
```

The Cargo shim lives in `~/.local/share/mbx/bin` and must precede mise's shims
on `PATH`. It has been added to `~/.config/fish/config.fish`; `mbx setup` does not
edit startup files itself. Without it, plain `cargo` bypasses the cache and
`mbx <cargo-command>` still works.

## Next

Milestone 2 is `registry` + `validator` + `render`: the registry types, the
cross-app checks, and the nginx config renderer as a **pure function** with no
Docker, no nginx and no root. That is the component which can affect all 14
services at once, so it gets tested in milliseconds before anything touches
production.
