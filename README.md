# swapdock

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
- All app ports on loopback (`127.0.0.1:PORT`); nothing exposed directly
- Root for `apply`/`up` (writing nginx config, reloading, managing containers);
  everything else runs unprivileged
- Optional: a Cloudflare tunnel (or any stable front door) pointing at the
  front ports

## Install

```bash
cargo install --locked swapdock
```

Then scaffold your first app and check it:

```bash
mkdir -p ~/swapdock-apps && cd ~/swapdock-apps
swapdock init blog
# edit blog.toml: front_port, slot, hostnames
swapdock --registry . validate
```

Or build the static binary and run the installer, which sets up
`/srv/swapdock`, the nginx snippet files, log rotation, and passwordless sudo
for the binary:

```bash
cargo build --release
sudo bash install/install-swapdock.sh ./examples
```

Then check the fictional estate that ships in `examples/`:

```bash
swapdock --registry examples/apps --tunnel-config examples/cloudflared.yaml validate
swapdock --registry examples/apps render
```

## Commands

| Command | Changes the host | What it does |
|---|---|---|
| `validate` | no | Per-app rules, cross-app clashes, tunnel routes. Exits non-zero on error. |
| `render [--app]` | no | Print the derived nginx config. |
| `apply` | **yes** | Render, refuse on error, stage, `nginx -t`, atomic commit, reload. |
| `up <app> [--release] [--force]` | **yes** | Full deploy through the app's strategy. Refuses a release that is already live; `--force` overrides. |
| `rollback <app>` | **yes** | Redeploy `old_release` through the same health gate. |
| `sync <app>` | repo only | Fetch and fast-forward the app source. Refuses on dirty checkouts. |
| `build <app> [--release]` | build host | Build the release image. Never touches the registry. |
| `verify <app> --since` | no | Access-log verdict: failures, upstreams, the flip instant. |
| `status <app>` | no | What's live right now: recorded release, previous release, live port, health verdict. |
| `inspect <app>` | no | The registry entry, live verdict and container mounts as JSON. |
| `doctor [--app] [--public]` | no | Check compose paths, bind mounts, ports, images, healthchecks and public URLs. |
| `adopt <app> --release <sha>` | **yes** | Turn a legacy container that publishes the front port into a swapdock-managed app. |
| `backup <app>` | files | Run the app's `[backup]` command once. |
| `probe <app>` | no | Probe `public_url` (or `--url`) and report the HTTP status. |
| `prune [--app] [--keep N]` | images | Remove old release images, keeping current/old and recent rollback tags. |
| `init <name>` | no | Write a commented registry template. Refuses to overwrite. |
| `selftest` | no | Exercise every subprocess guarantee. |
| `runs` / `show` / `resume` | no | Inspect the append-only run log. |

Every mutating command supports `--dry-run`, which records the plan and spawns
nothing. Every step — including filesystem renames — lands in the JSONL run log
with argv, exit code and duration, and secrets are redacted before anything is
written.

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

Static apps need `root` instead of ports and images; the flip is an atomic
symlink rename and rollback is instant. Set `spa = true` when the app is a
single-page app: extensionless paths fall back to `index.html`, while a
missing asset keeps its 404.

## What your compose files need

Three small conventions, so the tool injects per-deploy values without editing
files:

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
and commits the registry. `--dry-run` records the plan only.

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
cargo build --release          # ~1.6 MB static binary with the musl target
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
- The run log is one timeline that root and the deploy user both append to, so
  `/var/log/swapdock/swapdock.jsonl` and its directory belong to the deploy
  user's group. `install/logrotate-swapdock` carries the group and
  `install-swapdock.sh` substitutes it: a rotation that recreated the log
  root-only broke every unprivileged command until the next install.
- `swapdock sync <app>` owns the app checkout; it is the only thing that should
  move it. A hand-run `git pull` there races the tool and hides what is really
  deployed.
- `install-swapdock.sh` also adds the deploy user to `adm`, which `verify`
  needs to read `/var/log/nginx/front-door.access.log`.

## License

MIT — see [LICENSE-MIT](LICENSE-MIT).
