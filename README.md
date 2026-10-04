# deploy

A deploy tool for a multi-app Docker host: one registry as the source of truth,
an nginx front door, blue/green releases with a health gate, and a build host
that is a config value rather than a code path.

Design notes and the full plan live alongside the video essay in
`~/codes/deploy-explainer/`.

## Status: milestones 1 and 2 complete

295 tests, zero clippy warnings, 1.6 MB stripped binary. Nothing here needs
Docker, nginx or root — which is why the safety-critical parts could be finished
and tested before touching the host.

| Module | Milestone | Purpose |
|---|---|---|
| `src/exec.rs` | 1 | The subprocess chokepoint. The only place in the crate that may create a process. |
| `src/trace.rs` | 1 | Append-only JSONL run log: traceable, resumable, observable. |
| `src/redact.rs` | 1 | Secret masking, applied inside the chokepoint before anything reaches a file. |
| `src/time.rs` | 1 | RFC 3339 timestamps, no dependency. |
| `src/id.rs` | 1 | Sortable, unique run identifiers. |
| `src/registry.rs` | 2 | One file per app: the single source of truth. Per-app validation. |
| `src/ports.rs` | 2 | Slot-derived back-end ports. Collision-free by construction. |
| `src/render.rs` | 2 | The nginx config renderer. A pure function. |
| `src/validator.rs` | 2 | Cross-app checks: duplicate hostnames, ports, slots, tunnel routes. |
| `src/tunnel.rs` | 2 | Reads cloudflared's ingress rules. Degrades to a warning, never a false pass. |
| `src/apply.rs` | 3 | Stage, test, atomic commit, reload with restore on failure. |
| `src/deploy.rs` | 4 | Swap and replace as explicit step sequences. Rollback via the same gate. |
| `src/compose.rs` | 4 | `docker compose` argv builders. Pure. |
| `src/docker.rs` | 4 | `docker` argv builders. Pure. |
| `src/git.rs` | 4 | `git` argv builders. Pure. |
| `src/lock.rs` | 4 | Per-app and global nginx locks via flock. |
| `src/health.rs` | 4 | Container health gate plus a std-only HTTP probe. |
| `src/verify.rs` | 5 | Access-log verdict: failures, upstreams, the flip instant. |
| `src/builder.rs` | 6 | Local, SSH and no-op builders behind one resolution order. |
| `src/cli.rs` | 1–6 | `selftest`, `runs`, `show`, `resume`, `render`, `validate`, `apply`, `up`, `rollback`, `sync`, `build`, `verify`. |

Milestone 3 is `apply`: atomic write, `nginx -t`, restore on failure, reload.

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

# The registry commands, against the real apps in examples/
cargo run -- --registry examples/apps \
              --tunnel-config examples/cloudflared.yaml validate
cargo run -- --registry examples/apps render
cargo run -- --registry examples/apps render --app portfolio

# The commands that change the host. Refuse on any registry error.
sudo deploy --registry /srv/deploy/apps apply
sudo deploy --registry /srv/deploy/apps up portfolio --release 9c1f2ab
sudo deploy --registry /srv/deploy/apps rollback portfolio
sudo deploy --registry /srv/deploy/apps verify portfolio --since <run-id>
```

`validate` reads only and exits non-zero on any error, so it can gate a deploy.
`render` writes the config to stdout and changes nothing.

## The registry

One file per app under `/srv/deploy/apps`:

```toml
name = "portfolio"
kind = "container"            # or "static"
strategy = "swap"             # or "replace"
hostnames = ["m4marvin.com"]
listen = ["127.0.0.1"]        # empty means loopback; a second address is for tailnet
front_port = 8001             # what the tunnel points at. nginx owns it permanently.
slot = 0                      # owns back-end ports 9000 and 9001
writes_state = false          # true means a database file or other shared local state
image_repo = "apps-portfolio"
health_url = "http://127.0.0.1/"
compose_dir = "/home/marv/apps"
compose_svc = "portfolio"
env_name = "PORTFOLIO_PORT"
```

`examples/apps` holds all 13 apps currently running on the deploy host, and
`examples/cloudflared.yaml` is a copy of the tunnel config in use. Both are
exercised by `tests/example_registry.rs`, so the real data cannot drift silently.

### Design decisions worth knowing

**Unknown fields are an error.** `deny_unknown_fields` is deliberate: a typo like
`front_prot = 8001` would otherwise be ignored and the app would fall back to a
default port — on the component that rewrites nginx for every service, that is an
outage with a confusing cause.

**Back-end ports come from a slot, not a scan.** `pair(slot) = (9000 + 2*slot,
9000 + 2*slot + 1)`. Scanning races between concurrent deploys and strands ports
when a deploy dies. Deriving makes collision impossible, and the port in a log
line tells you which app it was.

**`writes_state` makes the two-writers rule machine-checkable.** `chat` and
`chats` both write `/app/data/local.db`; `forgejo`, `vaultwarden` and `kuma` use
sqlite defaults on a volume. A blue/green swap would run two containers against
one file. Rather than trusting everyone to remember, `strategy = "swap"` plus
`writes_state = true` is a validation **error**. Today that leaves only
`portfolio` and `morphotech` eligible for a swap — which is the honest answer.

**Rendering is deterministic.** Apps sorted by name, no timestamp, fixed
indentation and LF endings. That is what makes
`diff <(deploy render) /etc/nginx/conf.d/front-door.conf` a meaningful question.

**An unmigrated app gets no server block.** Every container on the host still
publishes its own front port, so nginx cannot bind any of them. A container app
only earns a `server` block once it has a `live_port`. Rendering blocks for
unmigrated apps would produce a config that fails to load.

**Values interpolated into nginx are allow-listed, not escaped.** `root` with a
space, a semicolon, a quote or a `$` is rejected rather than escaped, because an
escaping rule that misses a case is worse than a refusal.

**A parse failure in one file does not hide the other twelve.** `load_dir`
returns whatever loaded plus a problem for what did not, and `validate` reports
both.

**The tunnel parser is not a YAML parser.** It reads the one documented shape.
If the `ingress:` key is missing or nothing parses, it warns and the tunnel
checks are *skipped* — never silently passed.

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
cargo test                     # 295 tests (6 against real Docker)
cargo clippy --all-targets     # zero warnings
cargo fmt
```

Rust 1.98+ (edition 2024). Dependencies: `clap`, `serde`, `serde_json`, `toml`,
`anyhow`, `thiserror`, `tracing`, `tracing-subscriber`. Timestamps and run ids are
implemented in-crate to avoid two more.

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

## How apply keeps its promises

```
stage   write <target>.staging.<pid>, fsync file and directory, mode 0644
backup  copy the live file to <target>.bak (kept: it is the rollback source)
commit  rename(2) staging over target — readers see old or new, never a mix
test    nginx -t -c <main config> against the real full config
reload  nginx -s reload -c <main config>, only after the test passed
verify  the master pid is unchanged, proving reload and not restart
```

Every step is in the run log, including the filesystem mutations (recorded as
`write` / `copy` / `rename` steps, since those do not go through the
subprocess chokepoint).

| Failure | File left behind | Reload issued |
|---|---|---|
| `nginx -t` rejects | previous file, re-tested to prove it | never |
| `nginx -s reload` fails | previous file, re-tested to prove it | once, failed |
| master pid changed | **new file, deliberately** — the new config is already loaded | once |

A first-time apply has no backup; a failed test then removes the target rather
than leave an untested file. `--dry-run` records the plan and touches nothing.

Two details the real binary forced: paths are absolutized before invoking nginx,
because a relative `-c` resolves against nginx's compiled prefix and would test
the wrong file; and the reload carries `-c`, because without it nginx reads the
default config's pid file and signals the wrong master.

`proxy_pass` is emitted inside `location /`, not at server level — real `nginx
-t` rejected the first version of the renderer, which is why the milestone 2
golden test was wrong and the code was right to change.

## How a deploy runs

```
deploy up portfolio --release 9c1f2ab
  lock                 per-app flock; the kernel releases it if the tool dies
  resolve-release      flag, then recorded release, then an error
  pull                 skipped for local-only images
  green-start          same compose file, generated override for the name,
                       PORT and IMAGE from the environment, own project
  health-wait          Health.Status until healthy (missing healthcheck fails),
                       then GET the candidate port directly
  render-validate      the post-commit registry, validated as it will be
  apply                stage, test, atomic commit, reload (milestone 3)
  probe-front          every hostname on the front port with its Host header
  drain-wait           15 s for old keepalive workers to finish
  stop-old             by published port, never by assumed name
  registry-commit      release chain advances atomically
```

`replace` keeps one port: render, apply only when the text changed, stop, start
in place, gate, probe, commit. `rollback` deploys `old_release` through the same
strategy, so the way back has the same gate as the way forward.

`deploy build` runs on whichever host the resolution order picks (`--build-host`,
`DEPLOY_BUILD_HOST`, the registry, local) and prints the image ref. It never
touches the registry: building is not deploying.

`deploy verify` reads the access log back and reports failures, per-upstream
counts and the exact flip instant. It exits non-zero on any 5xx, so it can gate
automation — and `--since` takes a run id, so the window starts when the deploy
did.

## What is left

These are operations on the host, not code:

1. `apt install nginx` plus `proxy-common.conf`, the `deploy` log_format and the
   `front-door.conf` include — the three prerequisites the test harness fakes.
2. The 6 healthchecks in the two compose files. Without them the health gate
   refuses every deploy, which is correct but means nothing can ship.
3. Ports in compose as `${ENV_NAME}` with an `${IMAGE:-default}` image, so the
   tool can inject per-deploy values without editing files.
4. `~/apps` under version control, so compose files have history and rollback.
5. The first real migration: `charts` on its pair, measured with `verify`.
