# Changelog

All notable changes to swapdock. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.2.0] — 2026-10-19

- The run log stays readable and appendable by the deploy user across
  rotations: `install/logrotate-swapdock` recreates it `0664` with the deploy
  user's group instead of `root:root`, so unprivileged commands (`sync`,
  `build`, `runs`, `show`) no longer fail before doing any work. The installer
  substitutes the group, keeps the log's directory group-writable, and the
  directive carries `su` so logrotate will rotate inside that directory.
- `install-swapdock.sh` adds the deploy user to `adm`, which `verify` needs to
  read the nginx access log.
- `up` refuses to deploy the release that is already live unless `--force`
  (API: `"force": true`); a bare `up` no longer restarts a healthy app.
- `status <app>` prints the recorded release, previous release, live port and
  the live health verdict. `GET /apps/{name}/status` now includes `release` and
  `old_release` alongside the existing `up`, `status` and `live_port`.
- Run-log io errors name the file instead of a bare `Permission denied`.
- Static apps: `spa = true` includes the shared SPA history fallback, so
  extensionless deep links resolve to `index.html` while a missing asset keeps
  its 404. Ships `install/nginx/spa-fallback.conf`.
- `adopt <app> --release <sha>` turns a legacy container that publishes its
  front port directly into a swapdock-managed app: it finds the container by
  port, tags its image as the release, generates a persistent compose override
  (`/srv/swapdock/overrides/<app>.yml`) for the image, port and optional
  healthcheck, validates the merged compose model, stops the legacy container,
  applies nginx, starts the adopted release on the back port and commits the
  registry. `--dry-run` records the plan.
- Deploys retain the outgoing image as `image_repo:<old_release>` before
  stopping it, so a rollback target cannot disappear just because the tag was
  never created.
- `doctor` checks the host: home-relative compose paths (`~/` becomes `/root`
  under sudo), missing bind sources, containers mounting `/root`, legacy
  containers publishing a front port, missing release/rollback images,
  missing healthchecks and optional public URLs.
- `backup <app>` and a registry `[backup]` block run an argv-only snapshot
  command before a deploy, with `{file}`, `{app}`, `{release}` and
  `{timestamp}` placeholders.
- `prune [--app] [--keep N]` removes old release tags while keeping the current
  release, `old_release`, `latest`, unknown tags and recent rollback tags.
- `probe <app>` checks `public_url` (or `--url`) over HTTP(S), and `inspect
  <app>` prints the registry entry, live verdict and container mounts as JSON.
- The HTTP API mirrors the new operations: `GET /apps/{name}/inspect`,
  `GET /apps/{name}/doctor`, `POST /apps/{name}/adopt`,
  `POST /apps/{name}/backup`, `POST /apps/{name}/probe` and
  `POST /apps/{name}/prune`.
- `writes_state` apps without a `[backup]` command get a `backup-unset`
  warning; it is a warning, not a refusal.

## [0.1.0] — 2026-10-05

First public release. A complete deploy loop for one Docker host:

- Registry: one TOML file per app as the single source of truth, with
  per-app and cross-app validation that refuses typos, port collisions,
  duplicate hostnames and swaps on stateful apps.
- Renderer: the nginx front-door config as a pure, deterministic function.
- Apply: atomic file replacement gated by `nginx -t`, with restore on
  failure and a reload-vs-restart check on the master pid.
- Strategies: `swap` (blue/green on a port pair) and `replace` (stop and
  restart on one port), chosen by the registry, with rollback through the
  same health gate.
- Health gate: container health plus a direct HTTP probe; a missing
  healthcheck fails the deploy instead of passing it.
- Verify: the access-log verdict — failures, per-upstream counts and the
  exact flip instant.
- Builder: releases built on a pluggable host (local, SSH, none), named by
  commit, never `latest`.
- Observability throughout: one subprocess chokepoint, an append-only JSONL
  run log, secret redaction before anything reaches a file, and dry-run
  rendering of every mutating command.
