# Install

There is **no published release yet**. This document describes the only tested
path: build a dist tree from source, then run the installer on a supported
host. A one-line installer, checksum-verified downloads and a crates.io crate
are planned (G1.1/G1.7, DIST-01) but do not exist today.

## Prerequisites

| Requirement | Notes |
|---|---|
| Supported Linux host | Ubuntu 24.04/26.04 LTS, Debian 12/13; x86_64 or aarch64. See [supported-platforms.md](supported-platforms.md). |
| Docker Engine 27+ | With the Compose v2 plugin. |
| nginx 1.24+ | Installed and running; the installer runs `nginx -t`, not a reload. |
| git | Needed for source-built apps. |
| Rust 1.98+ | Only for building the binary; not needed at runtime. |
| sudo | The installer must run via `sudo` so it can identify your user. |

Build tools for a static binary (only needed when building locally):

```bash
# Debian/Ubuntu
sudo apt-get update
sudo apt-get install -y build-essential pkg-config musl-tools
rustup target add x86_64-unknown-linux-musl
```

`scripts/package-dist.sh` builds for the host by default. Use `--target` for a
musl build, exactly as the release workflow does:

```bash
bash scripts/package-dist.sh --target x86_64-unknown-linux-musl
```

The release workflow also builds `aarch64-unknown-linux-musl` with `cross`; see
[release.md](release.md).

## What the installer does

`install/install-swapdock.sh` is in the dist tree as
`swapdock-dist/install-swapdock.sh`.

| Path | Change |
|---|---|
| `/usr/local/bin/swapdock` | Installs the binary. A previous binary is kept as `swapdock.bak` (once). |
| `/srv/swapdock/apps` | Registry, owned by the sudo user. `apps/*.toml` from the dist are copied with `cp -n`; an existing file is never overwritten. |
| `/srv/swapdock/overrides` | Created for adopt/generated compose overrides. |
| `/srv/swapdock/backups` | Created for `[backup]` output. |
| `/srv/swapdock/green` | Created root-owned; deploy state. |
| `/var/log/swapdock` | Created `2770 root:<deploy group>`; the run log is `0640`, owned by the deploy user with the deploy group, so unprivileged commands can still append. |
| `/etc/nginx/conf.d/00-swapdock-http.conf` | Hand-written HTTP-level config. Previous file kept as `.bak` (once). |
| `/etc/nginx/snippets/proxy-common.conf` | Hand-written shared proxy config. `.bak` kept. |
| `/etc/nginx/snippets/spa-fallback.conf` | Hand-written SPA fallback. `.bak` kept. |
| `/etc/sudoers.d/swapdock` | `NOPASSWD` for `/usr/local/bin/swapdock` for the sudo user. Validated with `visudo -c` before it is moved into place. |
| `/etc/logrotate.d/swapdock` | Weekly rotation of `/var/log/swapdock/*.jsonl`, with the deploy user's group substituted. |

It also adds the sudo user to the `adm` group so `verify` can read
`/var/log/nginx/front-door.access.log` (a re-login is required). It does **not**
reload nginx, start the API, start a systemd unit, move traffic, or restart an
app container.

## Dry run

Dry run prints the plan, changes nothing, and does not require root:

```bash
bash swapdock-dist/install-swapdock.sh --dry-run swapdock-dist
```

The dry run still reads the dist tree and executes `swapdock --version`. It
warns if nginx, Docker or `visudo` is missing; a real run stops on those.

## Install

```bash
sudo bash swapdock-dist/install-swapdock.sh swapdock-dist
```

The default `DIST_DIR` (when omitted) is the sudo user's
`~/swapdock-dist`. Run it via `sudo`, not from a bare root shell: the installer
refuses a direct root login because it needs `SUDO_USER` to know whom to give
ownership to.

If you built with `--target`, the binary is included in `swapdock-dist/`
already; the installer does not need to know the target.

## Post-install checks

```bash
swapdock --version
nginx -t
swapdock --registry /srv/swapdock/apps validate
```

The installer already ran `nginx -t`. Do not reload nginx until you have
reviewed the generated config and the security warnings in
[security.md](security.md).

## Optional systemd unit

`--with-service` installs `install/systemd/swapdock.service` to
`/etc/systemd/system/swapdock.service`, generates `/etc/swapdock/api.env` with a
random `SWAPDOCK_API_TOKEN` (mode `0600`, only if the file does not exist), and
runs `systemctl daemon-reload`. It does **not** enable or start the unit.

Root mode installs a narrow `/usr/local/lib/swapdock/swapdock-root` wrapper; the
sudoers entry grants only that wrapper, not `swapdock broker`, `serve` or path
override flags. `--non-root` (implies `--with-service`) installs the API as the
deploy user and the root apply broker. It adds the deploy user to `docker`/`adm`, makes the
green/lock directories user-owned, and installs `swapdock-broker.service` plus a
non-root `swapdock.service`. The API is not added to the docker group; Docker
commands go through the broker's allowlist. See [non-root.md](non-root.md).

```bash
sudo bash swapdock-dist/install-swapdock.sh --with-service swapdock-dist
```

`--with-backup-timer` installs `swapdock-backup.service` and
`swapdock-backup.timer` and runs `systemctl daemon-reload`; it does **not**
enable the timer. Enable it only after a tested restore:

```bash
sudo systemctl enable --now swapdock-backup.timer
```

The timer runs `swapdock backup --all` daily at 03:30 with a randomized delay.
It does not enforce retention or copy offsite; see
[backup-restore.md](backup-restore.md).

`--with-monitor-timer` installs `swapdock-monitor.service/.timer` (five-minute
app/TLS/disk checks, not enabled). Set `SWAPDOCK_TLS_CERTS`,
`SWAPDOCK_DISK_THRESHOLD` and `SWAPDOCK_WEBHOOK_URL` in
`/etc/swapdock/api.env`; see [notifications.md](notifications.md).

The unit is documentation-grade, not a security boundary:

- it binds the API to `127.0.0.1:8088` (the CLI default);
- API TLS is optional: add `--tls-cert`/`--tls-key` to `ExecStart` when the
  files exist (see [tls.md](tls.md)); no ACME client is built in;
- it reads `SWAPDOCK_API_TOKEN` from `EnvironmentFile=-/etc/swapdock/api.env`;
  the token is required and must be at least 16 characters;
- set `SWAPDOCK_ALLOWED_HOSTS=console.example.com` (and
  `SWAPDOCK_ALLOWED_ORIGINS=https://console.example.com` for a split console)
  before exposing it through a proxy;
- the process runs as root because `apply`/`up` need root;
- API TLS is optional (above); the console exchanges the shared token for an
  HttpOnly session cookie and requires a CSRF header on mutations, but there
  are no user accounts, OIDC or token rotation;
- remote console access requires an authenticated TLS reverse proxy in front of
  the loopback listener until TLS and a real login land;
- it has a systemd hardening baseline (`NoNewPrivileges`, `PrivateTmp`,
  `ProtectSystem=full`, restricted kernel/control-group paths), but it still
  runs as root; that is not a substitute for the non-root broker (SEC-03).

Read [security.md](security.md) first. Read the generated token with:

```bash
sudo cat /etc/swapdock/api.env
```

Do not paste it into a URL, a shell history, a screenshot or a public issue.
Start the unit manually with `sudo systemctl start swapdock` only after you
understand the exposure.

If you install the unit manually without `--with-service`, create the token file
yourself before starting it:

```bash
sudo install -d -m 0755 /etc/swapdock
printf 'SWAPDOCK_API_TOKEN=%s\n' "$(openssl rand -hex 32)" |
  sudo tee /etc/swapdock/api.env >/dev/null
sudo chmod 0600 /etc/swapdock/api.env
```

## What the installer does not cover

- **Uninstall:** `swapdock-dist/uninstall-swapdock.sh` (see
  [uninstall.md](uninstall.md)).
- **Upgrade:** no released versions exist yet (see [upgrade.md](upgrade.md)).
- **TLS/domain:** not automated (SEC-02 planned). The supported reverse-proxy shape is in [tls.md](tls.md).
- **Console install:** not implemented; `web/public/config.json` defaults to
  same-origin `/api`, and the console exchanges the token for an HttpOnly
  session cookie, but the installer does not install the UI (DIST-03 planned)
  and there is no multi-user account system.
- **Static apps:** gated by DEP-04/G2.4; the tree has a static deploy path but
  the gate is still open. Do not rely on it on a real host until LAUNCH.md
  marks it green.

## If something fails

See [troubleshooting.md](troubleshooting.md). The most common failure is
running the installer against `examples/`; it has no `swapdock` binary and no
`nginx/` directory. Build with `scripts/package-dist.sh` and pass
`swapdock-dist`.
