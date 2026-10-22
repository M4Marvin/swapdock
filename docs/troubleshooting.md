# Troubleshooting

These are the failures a pre-launch install actually hits, in the order people
hit them. For security issues, do not open a public issue; see
[SECURITY.md](../SECURITY.md).

## Install and build

### `cargo install --locked swapdock` fails with a 404

The crate is **not published**. That command is not a supported install path
until DIST-01/G1.7 lands. Build from source instead:

```bash
git clone https://github.com/M4Marvin/swapdock.git
cd swapdock
bash scripts/package-dist.sh
```

### `install-swapdock.sh` says `swapdock missing or not executable`

You passed the wrong directory. The installer needs a dist tree built by
`scripts/package-dist.sh`; `examples/` has no binary and no `nginx/` directory.
Run:

```bash
bash scripts/package-dist.sh
sudo bash swapdock-dist/install-swapdock.sh swapdock-dist
```

### The installer refuses to run

It must be started with `sudo`, not from a bare root shell or as a normal user.
It needs `SUDO_USER` to know which user owns the registry and gets the sudo
entry.

```bash
sudo bash swapdock-dist/install-swapdock.sh swapdock-dist
```

`--dry-run` is the exception: it changes nothing and works as your normal user.

### The dry run warns that nginx/docker/visudo is missing

A real install requires nginx, Docker and `visudo`. Install them first. The
installer stops before writing anything if a prerequisite is missing.

### `nginx -t` fails at the end of the installer

The installer runs `nginx -t` after writing the hand-written files and stops on
failure. The previous files are at `.bak`. Restore them and inspect:

```bash
sudo diff -u /etc/nginx/conf.d/00-swapdock-http.conf.bak \
  /etc/nginx/conf.d/00-swapdock-http.conf
sudo mv /etc/nginx/conf.d/00-swapdock-http.conf.bak \
  /etc/nginx/conf.d/00-swapdock-http.conf
sudo nginx -t
```

Do not reload nginx until `nginx -t` passes.

## Registry and checks

### `validate` says 0 errors but `doctor` fails

`validate` checks registry rules only. `doctor` checks the host: compose files,
images, bind mounts, ports, healthchecks and public URLs. Trust `doctor` for
readiness. A missing `compose_dir` or image is the usual cause.

### `doctor` says the compose file is missing, but it exists as `compose.yaml`

The deploy path resolves `compose.yaml`, `compose.yml` and `docker-compose.yml`.
`doctor` in this tree still hardcodes `docker-compose.yml`, so it can report a
false `compose-missing` for a `compose.yaml` app. Workaround: add a
`docker-compose.yml` symlink next to it, or wait for the doctor check to be
updated. Do not rename a live app's compose file just to satisfy `doctor`.

### `swapdock init` refuses to overwrite

It is intentionally non-destructive. Choose another name/directory or move the
old file. `init` writes a commented template; it does not validate the result.

### `up --dry-run` did something unexpected

The current tree has tests asserting dry-run spawns no process and records
`would ...` steps, but launch gate G2.8/DEP-08 is not marked green with release
evidence. Treat dry-run as a best-effort plan, not a safety proof. Capture the
run id and `swapdock show <run-id>` output; if a process or file write happened,
stop and report it as a bug. Use `render` and `doctor` to inspect first.

### A deploy was killed and the app looks half-switched

A swap/replace/static deploy writes a pending record under
`/srv/swapdock/green/pending/<app>.json` before the nginx flip and clears it
after the registry commit. If the process was killed in that window, the file
remains and the API's app status reports the pending record (release, ports,
phase, run id).

Use `swapdock reconcile <app>` first. It reads the pending record, proves nginx
is pointing at the pending upstream (or the static symlink names the pending
release), probes the release's health path, and only then advances the registry
and clears the record. It refuses and leaves the record if the flip did not
happen or the release is unhealthy.

If reconcile refuses, recover deliberately:

1. Save the pending JSON and the run id; do not delete them first.
2. Compare `swapdock status <app>`, the registry TOML, the running container
   (`docker ps`) and `/etc/nginx/conf.d/front-door.conf`.
3. Decide whether to finish the new release or restore the previous one. Take a
   backup of `/srv/swapdock` before changing anything.
4. Only then run `swapdock apply`/`up` again, and clear the pending record only
   after you have confirmed the intended release is live.

`reconcile` does not handle an adopted app whose cutover failed before the
registry was saved; that path restarts the legacy container and leaves the
record for an operator. Crash recovery is still Gate 2 work; see
[LAUNCH.md](../LAUNCH.md).

### A static app fails to deploy

Static deploys have a symlink-flip path and unit/dry-run tests, but DEP-04/G2.4
is still open; there is no release
evidence yet. Do not rely on the symlink flip until [LAUNCH.md](../LAUNCH.md)
marks it green; `kind = "static"` validates and renders. Use a container app if
you need a deploy now.

### `rollback` or `adopt` behaved badly

Gate 2 deploy correctness is still red in [LAUNCH.md](../LAUNCH.md); the gate
tracks rollback image integrity, concurrency, the crash journal and adopt
safety. The current tree includes fixes and tests for several of those paths,
but do not rely on them as release guarantees until the gate is green. If a
rollback/adopt goes wrong:

1. Stop and do not run another deploy.
2. Record `docker ps`, the image IDs (`docker inspect`), and the run id.
3. Restore the app from your backup or the previous Compose setup.
4. Report the run id and exact commands; do not include secrets.

## API and console

### `swapdock serve` exits with `SWAPDOCK_API_TOKEN is not set`

Authentication is required by default. Set a token (16+ characters) or use the
development escape hatch on loopback only:

```bash
export SWAPDOCK_API_TOKEN="$(openssl rand -hex 32)"
swapdock serve --bind 127.0.0.1:8088
```

For the systemd unit, the token lives in `/etc/swapdock/api.env` (mode `0600`,
`SWAPDOCK_API_TOKEN=...`). `--with-service` creates it if missing. Never pass
the token as a command-line argument.

### `SWAPDOCK_API_TOKEN must be at least 16 characters`

Generate a real token: `openssl rand -hex 32`. Do not weaken the check.

### The API returns 401

The token is missing or wrong. Send `Authorization: Bearer <token>` or, for
`EventSource`, `?token=<token>`. The health endpoints (`/health`,
`/api/health`) are unauthenticated; every other `/api` route is not. Rotate the
token if it may have leaked.

### The console shows the wrong API or cannot connect

`web/public/config.json` defaults to same-origin `/api`, so the UI must be
served from the same origin as the API (or you must edit the config). The
console is not installed by the installer. It has a token-entry screen: enter
the token from `/etc/swapdock/api.env` (or your `SWAPDOCK_API_TOKEN`). There is
still no multi-user login or OIDC, so do not expose the console; use the CLI or a
loopback/TLS setup until DIST-03/G0.2 land.

### I exposed the API by mistake

Stop the service, remove the public route, rotate
`/etc/swapdock/api.env`, and follow
[security.md](security.md#if-you-exposed-it-by-mistake). Then report it via
[SECURITY.md](../SECURITY.md).

## Logs and permissions

### `Permission denied` on `/var/log/swapdock/swapdock.jsonl`

The log is `0640`, owned by the deploy user with the deploy user's group. If you
run a command as a different user, either use `--trace` to a path that user can
write, or run the command as the deploy user. The directory is
`2770 root:<group>`; the user must be in the group.

### `verify` cannot read the nginx access log

`verify` needs `/var/log/nginx/front-door.access.log`, typically
`www-data:adm 0640`. The installer adds the deploy user to `adm`; **log out and
back in** (or use `newgrp adm`) for the group to take effect. Check with:

```bash
id -nG
sudo ls -l /var/log/nginx/front-door.access.log
```

### Log rotation recreated the log with the wrong owner

Re-run the installer. It substitutes `@LOG_USER@` and `@LOG_GROUP@` into
`/etc/logrotate.d/swapdock` and recreates the log `0640` with the right owner.
Check the installed file:

```bash
sudo grep -A3 '^/var/log/swapdock' /etc/logrotate.d/swapdock
```

### The run log is empty or missing

Commands default to `/var/log/swapdock/swapdock.jsonl`. Before installation,
pass `--trace ./swapdock.jsonl`. A command that never opened the log (for
example, a clap usage error) writes nothing.

## Uninstall

### `uninstall-swapdock.sh` requires root

Run it with `sudo`, or use `--dry-run` without root. See
[uninstall.md](uninstall.md).

### nginx fails after uninstall

The generated `/etc/nginx/conf.d/front-door.conf` is not removed by the
uninstaller. If it references snippets that were removed, `nginx -t` fails, but
running nginx keeps its loaded configuration. Back up and remove or replace the
generated file before reloading. See
[uninstall.md](uninstall.md#the-generated-front-door-config).

## Getting help

- Reproduce on a disposable host and include `swapdock --version`, the command,
  and the run id from `swapdock runs`.
- Include `swapdock doctor` output (redact hostnames if needed).
- Never include tokens, customer data, or unredacted registry files.
