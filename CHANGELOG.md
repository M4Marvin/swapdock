# Changelog

All notable changes to swapdock. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

Security and correctness hardening; this is the pre-launch work tracked in
[LAUNCH.md](LAUNCH.md), not a released version.

### Security
- The HTTP API requires a bearer token (`SWAPDOCK_API_TOKEN`, 16+ chars,
  URL-safe) unless `--allow-unauthenticated` is passed for loopback
  development. Routes are mounted under `/api` only; `GET /health`,
  `GET /api/health`, and `POST /api/login` are the only unauthenticated routes. CORS is opt-in via
  `SWAPDOCK_ALLOWED_ORIGINS`;
  `CorsLayer::permissive()` is gone.
- The console has an access-token gate; it exchanges the token at
  `POST /api/login` for an in-memory HttpOnly, SameSite=Strict session cookie
  and a per-session CSRF token. The token is not stored after login; the CSRF
  token lives in `sessionStorage` and is sent on mutations. Bearer remains for
  CLI/automation, and the old token flow is the fallback for older APIs.
- `POST /probe` is restricted to the app's hostnames/`public_url` (or an
  operator allowlist), resolves once, rejects loopback/RFC1918/link-local/
  metadata addresses and pins curl to that public IP with `--resolve`, `/verify` only reads regular files under
  `/var/log/nginx` (bounded to 64 MiB), API mutations are capped at eight
  concurrent jobs, and the run log is `0640`.
- `root` rejects `/`, system directories and `..` traversal; `listen` rejects
  wildcard/public addresses. `[backup]` uses a positive program allowlist and
  rejects known command-execution options (`tar --checkpoint-action`,
  `rsync -e`, `sqlite3 .shell`, ...); the HTTP-reachable `allow_unsafe`
  escape hatch was removed.
- Generated front-door servers now send HSTS and `X-Content-Type-Options:
  nosniff`; a static app behind real nginx was verified over a public Cloudflare
  Tunnel URL.
- The console New App dialog has an **Import Compose** mode that calls
  `POST /api/import` with a Compose directory, hostname and optional name, then
  opens the imported app.
- The console is now packaged as `swapdock-dist/ui` and installed to
  `/srv/swapdock/ui`; `swapdock-ui.conf` serves it same-origin on
  `127.0.0.1:8080` with an `/api` proxy, HSTS and a 2 MiB body cap.
- Optional API TLS: `serve --tls-cert PATH --tls-key PATH` (rustls/ring).
  Certificates are not obtained or renewed automatically. The one-user v1 TLS
  path is a Cloudflare Tunnel in front of same-origin nginx; HTTPS 200,
  HTTP→HTTPS 301 and HSTS were exercised on a public test hostname.
- SSH/SCP transfer and build destinations are validated as `[user@]host` and
  placed after `--`, closing option injection (`-oProxyCommand=...`).
- `backup`/`restore` argv is rechecked at execution time, not only at registry
  validation; existing 0664 logs are tightened to 0640 when opened.
- `apply` renders only after taking the estate lock; registry PUT takes the
  same lock across read-modify-write; `adopt` writes a pending journal record.
- Opt-in `--non-root` install: the API runs as the deploy user and a hardened
  root `swapdock-broker` performs only validated nginx apply/reload plus an
  allowlisted Docker argv over a token-authenticated Unix socket. The API is
  not in the `docker` group. Tested under systemd in a disposable container.
- The example API/backup units get a systemd hardening baseline
  (`NoNewPrivileges`, `PrivateTmp`, `ProtectSystem`, restricted kernel paths,
  runtime directory, umask).
- Cookie sessions require `X-CSRF-Token` on POST/PUT/PATCH/DELETE; `/api/login`
  is the only new public route. Login failures are throttled per client key.
- A static-flip failpoint test proves a crash after the symlink flip leaves the
  pending journal for `reconcile`; `serve` reconciles any pending journal
  records before opening the API, so a
  restart cannot serve a registry that disagrees with the host.
- `run_start` records the process pid and kernel boot id. `runs` and `/api/runs`
  now report `Running` only while that process is alive on the same boot and
  `Interrupted` for a run whose process died without `run_end`, instead of
  leaving it "live" forever.
- `run_start` records an actor (`api` or `USER`/`SWAPDOCK_ACTOR`), exposed in
  run summaries. HTTP jobs record `api:<forwarded-IP>` when a trusted proxy
  supplies `X-Forwarded-For`/`X-Real-IP`. There is still one admin identity, not
  per-user accounts.
- `SWAPDOCK_ALLOWED_HOSTS` enforces a Host allowlist; same-origin and explicit
  Origin allowlists are checked for console sessions. Request bodies are capped
  at 2 MiB, and login failures are throttled per client key (5 failures / 5
  minutes), plus a 240/min per-client limiter and a 30s request timeout (SSE exempt). Sessions expire after 12 hours and are lost on
  restart; there is no user directory, OIDC or token rotation yet. Host and
  Origin allowlists are enforced.
- EventSource streams are capped (64; 8 in tests) and trace reads move off the
  async workers.
- Structured step `detail` is redacted; release validators are aligned to
  7–64 hex characters.

- Reconcile probes the public front door and stops a stale container still
  publishing the old port. Per-release green Compose projects are torn down
  after cutover or rollback, so they no longer leak Docker networks.
- Registry saves and static symlink flips fsync the parent directory after the
  rename, closing a power-loss window before the journal is cleared.

### Deploy correctness
- `rollback` and `up --force` no longer overwrite the rollback image tag; a
  conflicting tag is a hard `TagConflict` before the old container stops.
- Estate lock plus reload-under-lock fixes concurrent deploys and stale
  snapshots; CLI/API `apply` take the same lock.
- `--dry-run` reaches `registry-commit` without spawning/probing.
- `adopt` stops but keeps the legacy container until the new release passes
  health and the front probe; failure restarts it.
- `replace` auto-rolls back to the previous release through the same health
  gate; stop/rm failures are fatal.
- Green candidates use a per-release Compose project. Sharing one project made
  Compose remove the previous green container before the new release passed the
  front probe, breaking the keep-old guarantee; the imported-app failed-cutover
  test now proves the old container survives.
- Compose files resolve `compose.yaml`/`compose.yml`/`docker-compose.yml`;
  swap refuses named volumes and service dependencies that a new project would
  break.
- IPv6 listeners render as `[::1]:port`; apply staging is unique and fsyncs the
  parent directory after rename.
- Static apps deploy by verifying `releases/<release>/index.html`, flipping the
  `root` symlink atomically, probing, and restoring the previous link on
  failure (release-level e2e still pending).
- A pending journal records `flipping`/`flipped`/`stopping` and is exposed in
  app status; automatic reconcile and interrupted run status are still planned.
- Static deploy now has a live test (symlink flip, nginx render, registry
  commit) in addition to the dry-run/unit tests; it uses `/srv/www` and is
  skipped locally when that root is not writable.

### Notifications
- Optional `SWAPDOCK_WEBHOOK_URL` sends a redacted JSON POST after an API job
  fails (deploy, build, sync, backup, restore, apply, transfer). CLI backup
  failures use the same hook. It runs after the run closes, has no retries, and
  does not yet cover app-down, certificate expiry or disk thresholds.
- `swapdock backup --all` and an optional daily `swapdock-backup.timer`
  (`install-swapdock.sh --with-backup-timer`). The timer is not enabled by the
  installer; retention/offsite/verification are still manual.
- `swapdock monitor` checks live app ports, certificate expiry
  (`SWAPDOCK_TLS_CERTS`, 14-day window) and root disk use
  (`SWAPDOCK_DISK_THRESHOLD`, default 90%), with a de-duplicated webhook and an
  optional five-minute `swapdock-monitor.timer` (`--with-monitor-timer`). Email
  is still missing.
- `[backup] verify_command` checks a fresh snapshot and fails the backup on a
  non-zero exit; `offsite_command` copies it off the host; `keep = N` prunes all
  but the newest `<app>-*` snapshots. A real SQLite backup→verify→offsite→
  destroy→restore test now passes; a disposable-host drill is still missing.

### Onboarding, recovery, and backups
- `swapdock import` reads an existing Compose service through `docker compose
  config`, allocates a free front port and slot, writes the registry entry and a
  generated override (pinning `${IMAGE}`/ports, optionally adding a healthcheck),
  validates the staged estate, and never edits the Compose file or deploys.
  `--dry-run` writes nothing. Docker tests now import/deploy five real
  off-the-shelf services (nginx, httpd, caddy, python http.server, busybox
  httpd), and one app follows first deploy with a rolling release and a failed
  cutover that leaves the old container serving.
- `[backup] restore_command` plus `swapdock restore <app> --file <snapshot>`;
  restore refuses a missing snapshot and uses the same positive program
  allowlist as the dump command.
- `swapdock reconcile <app>` recovers a crash between the nginx flip and the
  registry commit: it proves nginx points at the pending upstream (or the static
  symlink names the release), probes health, then advances the registry. It
  refuses and keeps the pending record otherwise.
- `adopt` now writes/clears a pending journal record around its cutover.
- TLS reverse-proxy example and docs (`docs/tls.md`); not automated.

### Console
- New version-drift card and header badge: current release/health, latest
  fetched source release, commit subject/time, commits behind, image/release
  readiness, and one action (deploy/build/check).
- Same-origin `config.json`; empty `transferTarget` no longer enables transfers;
  missing config shows a warning.
- Internal fields (front port, slot, raw release, strategy, live port) moved
  behind “Technical details”; human deploy titles; rollback disabled when there
  is no previous version; Doctor/Runs error states no longer render as clean.
- Light-theme status tokens meet AA; app metadata ≥11px; search labels and
  screen-reader status text; first-run “New app” dialog; token gate.

### Distribution and docs
- CI (`fmt`, clippy, tests, e2e, audit, web, docs), tag-driven release workflow
  with musl builds, checksums, keyless Sigstore signatures and SPDX SBOMs
  (not yet run on GitHub), `scripts/package-dist.sh`,
  installer `--dry-run`/`--with-service`, uninstaller, docs and SECURITY.md.
- No published release, crate, image, signature or SBOM yet; TLS, non-root
  service, Compose import, scheduled backups/notifications and the measured
  15-minute onboarding path remain open in [LAUNCH.md](LAUNCH.md).

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
- `up` refuses a local release whose `image_repo:<release>` tag is absent, and
  compose runs with `--no-build`. A missing image can no longer make compose
  build from the host's checkout and tag stale code as the requested release.
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
