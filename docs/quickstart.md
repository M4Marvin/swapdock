# Quickstart (source checkout)

**Status: pre-launch.** This gets you from a checkout to a *validated* registry
on a disposable host. It does **not** promise a working deploy: the deploy
correctness P0s in [LAUNCH.md](../LAUNCH.md) are still open. Do not use a
production app for this walkthrough.

## 1. Prerequisites

- A supported Linux host (Ubuntu 24.04/26.04 LTS or Debian 12/13, x86_64 or
  aarch64). See [supported-platforms.md](supported-platforms.md).
- Docker Engine 27+ with Compose v2, nginx 1.24+, and git.
- Rust 1.98+ **only to build**; the installer itself needs no Rust.
- sudo access for the installer. The commands below marked "unprivileged" do
  not need root.

There is no published binary yet. `cargo install --locked swapdock` fails with a
404 until the crate is published.

## 2. Build the dist tree

```bash
git clone https://github.com/M4Marvin/swapdock.git
cd swapdock
bash scripts/package-dist.sh
```

This runs `cargo build --release --locked` and writes `swapdock-dist/` with the
binary, the installer, the uninstaller, the nginx files and logrotate. Inspect
it:

```bash
ls -R swapdock-dist
```

## 3. Preview the install

The installer supports a side-effect-free plan and does **not** need root for it:

```bash
bash swapdock-dist/install-swapdock.sh --dry-run swapdock-dist
```

Read the "would:" lines. If nginx or Docker is missing, the dry run warns and a
real install would stop.

## 4. Install

```bash
sudo bash swapdock-dist/install-swapdock.sh swapdock-dist
```

The installer:

- backs up `/etc/nginx` to `/root/nginx-pre-swapdock-<timestamp>.tgz`;
- installs `/usr/local/bin/swapdock` (a previous binary is kept as `.bak`);
- creates `/srv/swapdock/{apps,green,overrides,backups}` and
  `/var/log/swapdock`;
- installs the hand-written nginx files, sudoers entry and logrotate config;
- runs `nginx -t` but **does not reload nginx or start anything**.

It is idempotent and never overwrites a registry file. It exits non-zero if
`nginx -t` fails.

## 5. Create and check a registry entry

Use a scratch directory first so you cannot damage a live registry:

```bash
mkdir -p ~/swapdock-apps && cd ~/swapdock-apps
swapdock init blog --dir .
# edit blog.toml: front_port, slot, hostnames, compose_dir, compose_svc,
# health_url, env_name, and (for source deploys) repo/git_remote/branch.
swapdock --registry . --trace ./swapdock.jsonl validate
swapdock --registry . --trace ./swapdock.jsonl doctor
swapdock --registry . --trace ./swapdock.jsonl render
```

`validate` checks registry rules only. It can print `0 error(s)` while
`doctor` still fails on a missing compose directory or image — that is the
real readiness check. `render` prints the nginx config and changes nothing.

When the registry looks right, copy the TOML into `/srv/swapdock/apps/` (the
installer makes it writable by your user) and run the checks again without
`--registry .`:

```bash
cp blog.toml /srv/swapdock/apps/
swapdock validate
swapdock doctor
```

## 6. Stop before the first deploy

The first `apply` reloads nginx; `up` starts containers and moves traffic.
Both are host-mutating, and Gate 2 (deploy correctness) is still listed as red
in [LAUNCH.md](../LAUNCH.md). The current tree contains fixes and tests for
several of those paths, but do not treat a deploy as production-safe until the
gate is green; use a disposable host and a backup. If you only want to inspect
the generated front door:

```bash
swapdock render | less
```

## What this quickstart deliberately does not cover

| Not covered | Why |
|---|---|
| A real deploy | Gate 2 deploy correctness is still red in LAUNCH.md. |
| TLS/domain setup | No ACME or assisted tunnel flow exists yet (SEC-02 planned). |
| The web console | Not distributable, has a token-entry screen and a single-admin session, but no multi-user login; config defaults to same-origin `/api` (DIST-03 planned). |
| Static apps | The tree has a symlink-flip static path and tests, but DEP-04/G2.4 still needs release evidence; do not use production until the gate is green. |
| Uninstall/upgrade | See [uninstall.md](uninstall.md) and [upgrade.md](upgrade.md); upgrade is not supported until releases exist. |
