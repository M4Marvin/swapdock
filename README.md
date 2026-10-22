# swapdock

[![CI](https://github.com/M4Marvin/swapdock/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/M4Marvin/swapdock/actions/workflows/ci.yml)

> **Project status: pre-launch (v0.2).** Not yet recommended for public production. The launch gates, scope, decisions, and definition of ready are in **[LAUNCH.md](LAUNCH.md)**; the evidence behind them is in **[swapdock-pre-launch-audit.md](swapdock-pre-launch-audit.md)**.

**Docs:** [Quickstart](docs/quickstart.md) · [Import an existing app](docs/import.md) ·
[Install](docs/install.md) ·
[Upgrade](docs/upgrade.md) · [Uninstall](docs/uninstall.md) ·
[Backup and restore](docs/backup-restore.md) · [Security](docs/security.md) ·
[Troubleshooting](docs/troubleshooting.md) ·
[Supported platforms](docs/supported-platforms.md) · [Releases](docs/release.md)

Registry-driven blue/green deploys for a multi-app Docker host, behind an
nginx front door. One TOML file per app is the single source of truth; the
nginx config is derived from it, never hand-edited.

## How it works

Every app gets a **front port** that never changes, and a pair of **back ports**
that alternate on every deploy:

```text
internet -> tunnel -> nginx:8001 -> 127.0.0.1:9001 -> container (live)
                                              9000    (previous, kept for rollback)
```

A deploy starts the new release on the free port of the pair, waits for its
healthcheck, flips the generated nginx config to it, reloads nginx, and only
then stops the old container. Cloudflare, Tailscale, or plain DNS in front
never changes: only loopback ports move.

Apps with a file-backed database (sqlite on a volume) use `replace` instead:
stop, start on the same port, gate on health. Two containers never hold one
sqlite file open — the registry refuses `swap` for stateful apps, so this is
enforced, not remembered.

## Requirements

- Linux with Docker Engine (Compose v2), nginx, and git
- Rust 1.98+ only when you build from source; the installer needs no Rust
- All app ports on loopback (`127.0.0.1:PORT`); nothing exposed directly
- Root for `apply`/`up` (writing nginx config, reloading, managing containers);
  everything else runs unprivileged
- Optional: a Cloudflare tunnel (or any stable front door) pointing at the
  front ports

## Install

**There is no published release yet.** The crate is not on crates.io and there
are no release binaries (`cargo install --locked swapdock` fails with a 404
until [DIST-01](LAUNCH.md) lands). Use the source path below; it is the only
install path this tree documents today.

Build the dist tree from a checkout (Rust 1.98+):

```bash
git clone https://github.com/M4Marvin/swapdock.git
cd swapdock
bash scripts/package-dist.sh
sudo bash swapdock-dist/install-swapdock.sh swapdock-dist
```

`scripts/package-dist.sh` runs `cargo build --release --locked` and assembles
`swapdock-dist/` with the binary, the app-template placeholder, the nginx
files, logrotate, and the installer/uninstaller. The installer creates
`/srv/swapdock`, installs the nginx snippet files, adds logrotate and a
passwordless sudo entry for the binary, and runs `nginx -t`. It does **not**
reload nginx or restart anything. Print the plan first if you want:

```bash
bash swapdock-dist/install-swapdock.sh --dry-run swapdock-dist
```

For a non-root API with the root apply broker:

```bash
sudo bash swapdock-dist/install-swapdock.sh --with-service --non-root swapdock-dist
```

See [docs/install.md](docs/install.md) and [docs/non-root.md](docs/non-root.md)
for prerequisites, service modes, the uninstaller, and what the installer
deliberately leaves alone.

Then scaffold your first app and check it:

```bash
mkdir -p ~/swapdock-apps && cd ~/swapdock-apps
swapdock init blog
# edit blog.toml: front_port, slot, hostnames, compose_dir, health_url
swapdock --registry . validate
swapdock --registry . doctor
```

`validate` only checks registry rules; `doctor` checks the host (compose paths,
images, ports, healthchecks). Neither makes a pre-launch build deploy-ready;
read [docs/quickstart.md](docs/quickstart.md) before pointing it at a real app.

## Import an existing Compose app

If the app already runs from a Compose file, let `import` write the registry
entry instead of hand-editing one:

```bash
swapdock import \
  --compose-dir ~/apps/shop \
  --service web \
  --name shop \
  --domain shop.example.com \
  --registry ./apps \
  --state-dir ./state/green \
  --dry-run
```

It resolves `compose.yaml`/`compose.yml`/`docker-compose.yml`, reads the merged
model with `docker compose config`, detects the image, container port and
healthcheck, allocates a front port and slot from the existing registry, and
writes `<name>.toml` plus a generated Compose override. It never edits the
Compose file, starts a container or deploys. `--dry-run` still runs the
read-only Compose probe (that is where the plan comes from) but writes nothing;
without it, the two files are written and the command prints the remaining
steps. See [docs/import.md](docs/import.md) for detection rules, named volumes,
and a worked example.

## Examples (validation only)

`examples/` is a fictional estate used by the validator tests. It is **not** an
installer template and **not** a quickstart: it contains no binary and no
`nginx/` directory, and its `/srv/example/...` paths do not exist on your host.
Use it to exercise validation and rendering:

```bash
swapdock --registry examples/apps --tunnel-config examples/cloudflared.yaml validate
swapdock --registry examples/apps render
```

`examples/apps/docs.toml` is a static app. The working tree has a static deploy
path (verify `<root-parent>/releases/<release>/index.html`, flip the `root`
symlink, probe, restore on failure) with unit and dry-run tests, but DEP-04/G2.4
still needs release-level end-to-end evidence. Do not point a static app at a
production host until [LAUNCH.md](LAUNCH.md) marks that gate green; treat
validation/rendering as all this example proves today.

## Supported platforms

The v1 target is narrow, and it matches [LAUNCH.md](LAUNCH.md) §8:

- **OS:** Ubuntu 24.04/26.04 LTS, Debian 12/13. No Windows or macOS hosts.
- **Architecture:** x86_64 and aarch64.
- **Host tools:** Docker Engine 27+ with Compose v2, nginx 1.24+, git.
- **Network:** a domain and DNS control; Cloudflare Tunnel is optional.
- **Not supported in v1:** rootless Docker, multi-host, Kubernetes, and
  Kubernetes-style orchestration.

The release workflow in [.github/workflows/release.yml](.github/workflows/release.yml)
builds static musl binaries for both architectures when a tag is pushed, but
**no release has been published yet** and no artifact is claimed to work on a
host other than the one that built it. See
[docs/supported-platforms.md](docs/supported-platforms.md) and
[docs/release.md](docs/release.md).

## Security

Read [SECURITY.md](SECURITY.md) to report a vulnerability and
[docs/security.md](docs/security.md) for the honest current posture. The short
version, as of this pre-launch tree:

- **The HTTP API requires a token by default.** `swapdock serve` refuses to
  start unless `SWAPDOCK_API_TOKEN` is set (at least 16 characters). The
  `--allow-unauthenticated` escape hatch is for loopback development only and
  must never be used on a shared host. The token is a single shared secret, not
  a multi-user account system.
- **The console exchanges the token for an HttpOnly session cookie.** Stateful
  requests require a per-session `X-CSRF-Token`; bearer clients remain for the
  CLI and automation. Sessions are in memory, expire after 12 hours, and are
  lost on restart. There is no OIDC, user directory or token rotation.
- **TLS is optional and ACME is not implemented.** `serve --tls-cert --tls-key`
  terminates TLS on the API listener. The one-user v1 path is a Cloudflare
  Tunnel in front of same-origin nginx: public HTTPS, HTTP→HTTPS redirect and
  HSTS were exercised on a disposable hostname. The generated front door is
  HTTP and the example unit binds `127.0.0.1` only; see
  [docs/tls.md](docs/tls.md).
- **Login throttling exists; general request rate limiting does not.** Host and
  Origin are checked when `SWAPDOCK_ALLOWED_HOSTS`/`SWAPDOCK_ALLOWED_ORIGINS`
  are set; CORS is an optional allowlist and is not a substitute for
  authentication. See Gate 0 (G0.2/G0.3/G0.9).
- **Do not expose the API to the internet or to an untrusted network.** If you
  run `--with-service`, keep the unit on loopback. The proxy rule is defence in
  depth, not real user authorization.
- **The run log is `0640`**, owned by the deploy user and readable by its group.
  Secret redaction is incomplete (G0.8); treat the log and `/srv/swapdock` as
  sensitive.
- Auth hardening, TLS, non-root service and redaction are Gate 0 work (SEC-01
  through SEC-06). The non-root API + apply broker is opt-in and tested in a
  disposable container. The API is not in the docker group and non-root mode
  installs no passwordless sudo entry; Docker commands go through the broker allowlist. The broker itself is still a high-value root
  process. Do not deploy this build as secure without reviewing the residual risk.

## Screenshots

<!-- TODO: add non-personal screenshots/GIFs once the console is installable and
     served same-origin. Never commit the author's private-fleet .shots images. -->

_No screenshots yet._ The console is pre-launch and not distributable; a
non-personal screenshot/GIF set is part of Gate 4 (G4.1/G4.7). The images under
`.shots/` are private-fleet verification captures and are deliberately not
committed.

## Commands

| Command | Changes the host | What it does |
|---|---|---|
| `validate` | no | Per-app rules, cross-app clashes, tunnel routes. Exits non-zero on error. |
| `render [--app]` | no | Print the derived nginx config. |
| `apply` | **yes** | Render, refuse on error, stage, `nginx -t`, atomic commit, reload. |
| `up <app> [--release] [--force]` | **yes** | Full deploy through the app's strategy. Refuses a release that is already live; `--force` overrides. Refuses a local release with no built image. |
| `rollback <app>` | **yes** | Redeploy `old_release` through the same health gate. |
| `sync <app>` | repo only | Fetch and fast-forward the app source. Refuses on dirty checkouts. |
| `build <app> [--release]` | build host | Build the release image. Never touches the registry. |
| `verify <app> --since` | no | Access-log verdict: failures, upstreams, the flip instant. |
| `status <app>` | no | What's live right now: recorded release, previous release, live port, health verdict. |
| `inspect <app>` | no | The registry entry, live verdict and container mounts as JSON. |
| `doctor [--app] [--public]` | no | Check compose paths, bind mounts, ports, images, healthchecks and public URLs. |
| `adopt <app> --release <sha>` | **yes** | Turn a legacy container that publishes the front port into a swapdock-managed app. |
| `import [--compose-dir DIR] [--service NAME] ...` | registry + override | Detect an existing Compose service, allocate a front port/slot, and write the registry entry plus a generated override. Never edits Compose or deploys. |
| `restore <app> --file PATH` | app data | Run the app's `[backup] restore_command` against a snapshot. Refuses a missing file or unconfigured restore path. |
| `reconcile <app>` | registry | After a crash between the nginx flip and the registry commit, prove what is live and advance the registry. Refuses if nginx is not on the pending release or it is unhealthy. |
| `backup <app>` / `backup --all` | files | Run configured `[backup]` commands once, or all of them for the systemd timer. |
| `probe <app>` | no | Probe `public_url` (or `--url`) and report the HTTP status. |
| `prune [--app] [--keep N]` | images | Remove old release images, keeping current/old and recent rollback tags. |
| `init <name>` | no | Write a commented registry template. Refuses to overwrite. |
| `selftest` | no | Exercise every subprocess guarantee. |
| `runs` / `show` / `resume` | no | Inspect the append-only run log. |

A `--dry-run` flag is wired through the CLI, and the current tree has tests that
assert it spawns nothing, reaches `registry-commit`, and prints `would ...`
steps (including `apply --dry-run`). G2.8/DEP-08 still needs release evidence
before the gate is green; treat dry-run as a plan, not a safety proof, and check
the run log for what it actually did. Every live step — including filesystem
renames — lands in the JSONL run log with argv, exit code and duration; secret
redaction is improved (structured `detail`, URL-safe tokens, `0640` logs) but
still not a complete secret store (G0.8).

## A deploy, step by step

```
swapdock up shop --release 9c1f2ab
  lock                 per-app flock; the kernel releases it if the tool dies
  resolve-release      flag, then recorded release, then an error
  pull                 skipped for local-only images
  green-start          same compose file, generated override for the name,
                       PORT and IMAGE from the environment, own project
  health-wait          Health.Status until healthy (a missing healthcheck fails),
                       then GET the candidate port directly
  render-validate      the post-commit registry, validated as it will be
  apply                stage, test, atomic commit, reload
  probe-front          every hostname on the front port with its Host header
  drain-wait           15 s for old keepalive workers to finish
  retain-old-image     tag the running image as old_release before removing it
  stop-old             by published port, never by assumed name
  registry-commit      release chain advances atomically
```

## The registry

One file per app in `/srv/swapdock/apps/<name>.toml`:

```toml
name = "shop"
kind = "container"            # or "static"
strategy = "replace"          # or "swap"; refused with writes_state
hostnames = ["shop.example.com", "www.shop.example.com"]
listen = ["127.0.0.1"]        # empty means loopback; more addresses bind more
front_port = 8002             # the tunnel points here; nginx owns it permanently
slot = 1                      # owns back-end ports 9002 and 9003, for life
writes_state = true           # a database file or other shared local state
image_repo = "example-shop"   # release tag appended: example-shop:9c1f2ab
build_host = "local"          # or an SSH destination; a config value, not code
release = "9c1f2ab"           # running commit; never `latest`
health_url = "http://127.0.0.1:3000/api/health"  # path is used; port is the candidate's
compose_dir = "/srv/example/compose"
compose_svc = "shop"
env_name = "SHOP_PORT"        # compose reads 127.0.0.1:${SHOP_PORT}:80
git_remote = "example-org/shop"
branch = "main"
repo = "/srv/example/shop"
# Optional operations hooks:
# compose_override = "/srv/swapdock/overrides/shop.yml"  # generated by `adopt`
# public_url = "https://shop.example.com/health"
# [backup]
# command = ["sqlite3", "/srv/example/shop/data/shop.db", ".backup {file}"]
# dir = "/srv/swapdock/backups/shop"
# before_deploy = true
```

Static apps use `root` instead of ports and images. The working tree now has a
static deploy path: it expects `releases/<release>/index.html` beside the root,
flips the root symlink atomically, probes the front door, and restores the
previous link on failure. `spa = true` adds the history fallback (extensionless
paths serve `index.html`; a missing asset stays a 404). DEP-04/G2.4 still needs
release-level end-to-end evidence; do not point a static app at a real host
until [LAUNCH.md](LAUNCH.md) marks it green.

## What your compose files need

Three small conventions, so the tool injects per-deploy values without editing
files. The deploy path resolves `compose.yaml`, `compose.yml`, then
`docker-compose.yml`; `doctor` in this tree still looks only for
`docker-compose.yml`, so it can report a false `compose-missing` for the new
names until that check is updated. Compose is always invoked with `--no-build`:
a missing image is an error, never an excuse to build from whatever checkout
happens to be on the host.

```yaml
services:
  shop:
    image: ${IMAGE:-example-shop}       # default keeps `compose up` working by hand
    ports:
      - "127.0.0.1:${SHOP_PORT:-8002}:80"
    healthcheck:                         # the gate refuses without one
      test: ["CMD", "wget", "-q", "-O", "/dev/null", "http://127.0.0.1:3000/api/health || exit 1"]
```

## Adopting a legacy container

An app that already publishes its front port directly cannot self-adopt: a
`replace` app needs a recorded `live_port`, and nginx cannot bind the front port
while the old container holds it. `swapdock adopt` performs the one-time
cutover:

```bash
swapdock adopt shop --release 9c1f2ab   # the release it is already running
swapdock up shop --release d34db33      # the next normal deploy
```

It finds the container by the front port, tags its image as
`image_repo:<release>`, writes a persistent compose override
(`/srv/swapdock/overrides/<app>.yml`) for the image, the `${ENV_NAME}` port and
a healthcheck when needed, validates the merged compose model, stops the legacy
container, applies nginx, starts the adopted release on the back port, gates it
and commits the registry.

> **Gate 2 is still open.** [LAUNCH.md](LAUNCH.md) tracks adopt safety
> (DEP-06/G2.6) and the rest of deploy correctness. This tree contains fixes
> and tests for several of those paths, but a gate is not green until the
> evidence is recorded. Treat adopt as unsafe on a production app until
> LAUNCH.md says otherwise, keep a backup, and test on a disposable host.
> `--dry-run` is best-effort, not a safety proof.

If the compose service has no healthcheck, pass
`--healthcheck "<command>"`; the override installs it. `--container-port` is
only needed when Docker's port map cannot be read.

## Tunnel setup (optional)

Point each hostname at its front port and never touch it again. With
cloudflared, one rule per hostname (`service: http://localhost:<front_port>`)
is enough — swapdock never edits the tunnel, so deploys can never cause a
tunnel reconnect. `validate --tunnel-config` cross-checks the registry against
those routes.

## Development

```bash
cargo build --release          # native release build; the release workflow uses musl
cargo test                     # unit, e2e (real Docker), installer and chokepoint tests
cargo clippy --all-targets     # zero warnings
cargo fmt --check
```

The test suite includes fault injection (broken configs restore byte-identical),
end-to-end strategies against real containers, and a test that fails the build
if any module besides `exec.rs` spawns a process. The `examples/` estate is
covered by tests too, so the documentation cannot drift from the validator.

## Ops notes

- The deploy loop retains the old image as `image_repo:<old_release>` before
  stopping it, so rollback never points at a tag that was pruned. `swapdock
  prune` removes older release tags while keeping the current release,
  `old_release`, `latest`, unknown tags and the newest `--keep` rollback tags.
- The registry files are owned by the deploy user. Swapdock rewrites them
  in place as root but preserves their mode and owner.
- The run log is one timeline that root and the deploy user both append to.
  `/var/log/swapdock/swapdock.jsonl` is `0640`, owned by the deploy user with
  the deploy user's group; the directory is `2770 root:<group>`. `install/logrotate-swapdock`
  carries `@LOG_USER@`/`@LOG_GROUP@` and `install-swapdock.sh` substitutes
  both: a rotation that recreated the log root-only broke every unprivileged
  command until the next install.
- `swapdock sync <app>` owns the app checkout; it is the only thing that should
  move it. A hand-run `git pull` there races the tool and hides what is really
  deployed.
- `install-swapdock.sh` also adds the deploy user to `adm`, which `verify`
  needs to read `/var/log/nginx/front-door.access.log`.

## Web UI

`web/` is the operator console: fleet health and attention, an app's blue/green
topology, the build → transfer → deploy pipeline with live step output,
operations (doctor, probe, backup, prune, inspect, adopt, generated nginx), the
registry editor, the run ledger and estate-wide doctor findings.

```bash
cd web
pnpm install
pnpm dev        # Vite dev server; check vite.config.ts for the API proxy
pnpm build      # static bundle; not an installer
```

> **Not distributable yet** (DIST-03/G1.8, G0.2). The console has an access-token
> gate; it exchanges the token for an HttpOnly session cookie and sends a CSRF
> header on mutations. `web/public/config.json` defaults to same-origin `/api`.
> There is one shared admin credential, no user accounts or rotation, and the
> installer does not install the UI. `web/deploy.sh` is a private deploy helper,
> not an installer. Do not serve the console on a public host without an
> authenticated TLS reverse proxy; a same-origin `/api` proxy is required for
> the default config.

The UI follows system light/dark by default, has a command palette on
`⌘K`/`Ctrl K`, and talks to the API through `web/public/config.json`. Deploys
are version-first for source-built apps, image-based for prebuilt apps, and
take over legacy containers before deploying them. A static deploy path exists
but its release gate is still open. Design notes, routes and data-flow
details live in [web/README.md](web/README.md).

## Documentation

| Doc | What it covers |
|---|---|
| [Quickstart](docs/quickstart.md) | Source checkout to a validated registry, with the known gaps called out. |
| [Import](docs/import.md) | Turn an existing Compose service into a registry entry without editing Compose. |
| [Install](docs/install.md) | Prerequisites, `package-dist.sh`, installer and `--with-service`, dry-run. |
| [Upgrade](docs/upgrade.md) | Binary replacement, backups, and the not-yet-built migration path. |
| [Uninstall](docs/uninstall.md) | What the uninstaller removes/keeps, `--purge`, leftovers. |
| [Backup and restore](docs/backup-restore.md) | Registry and app-data backup, restore, and what is still planned. |
| [TLS and the console proxy](docs/tls.md) | Required reverse-proxy shape for the console and API; not automated yet. |
| [Non-root API and broker](docs/non-root.md) | Opt-in non-root API with a root broker for nginx and allowlisted Docker commands. |
| [Failure notifications](docs/notifications.md) | Failure webhook, app/TLS/disk monitor checks, payload and what is still missing. |
| [Security](docs/security.md) | Current posture, threat model, exposure rules, Gate 0 work. |
| [Troubleshooting](docs/troubleshooting.md) | The failures a pre-launch install actually hits. |
| [Supported platforms](docs/supported-platforms.md) | v1 matrix and unsupported-host behavior. |
| [Releases](docs/release.md) | Release workflow, keyless Sigstore signing, SBOM, verification. |
| [Contributing](CONTRIBUTING.md) | Build/test commands, single-spawn-point rule, PR norms. |
| [Security policy](SECURITY.md) | Reporting, 48-hour acknowledgement, scope, no bounty. |

## License

MIT — see [LICENSE-MIT](LICENSE-MIT).
