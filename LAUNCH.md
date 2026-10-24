# swapdock — launch source of truth

> **Status:** ACTIVE — execution plan  
> **Owner:** Marvin (maintainer)  
> **Created:** 2026-10-22  
> **Last verified:** 2026-10-24 against `/home/marv/codes/deploy` at local `0.2.0`. No git tag exists and `origin/main` is `b55b4d4`, so there is no released baseline yet.  
> **Review cadence:** weekly, and in the same PR as any change to scope, a gate, or a P0/P1 status  
> **Evidence base:** [swapdock-pre-launch-audit.md](swapdock-pre-launch-audit.md) (the point-in-time adversarial audit)

This is the canonical document for **what swapdock is, what “ready” means, what is in and out of scope, and what must be true before it is marketed publicly**. When this document and the code disagree, the code describes what *is*, this document describes what *should be*; reconcile them before the next release. When this document and a chat/issue/old README disagree, this document wins for scope and intent.

**Contents:** [How to use](#1-how-to-use-this-document) · [Product definition](#2-product-definition) · [Launch gates](#3-launch-gates) · [Scope](#4-scope) · [Decisions](#5-decisions) · [Defect register](#6-defect-register) · [Non-technical decision](#7-non-technical-decision) · [Quality/release](#8-quality-security-and-release-process) · [Operations/data](#9-operations-and-data) · [Go-to-market](#10-go-to-market-source-of-truth) · [Metrics](#11-metrics) · [Risks](#12-risks) · [Open questions](#13-open-questions) · [References](#14-repository-map-and-references) · [Change log](#15-change-log-of-this-document)

### TL;DR

- **Do not launch publicly yet.** The user narrowed v1 to a private one-user tool. Token/session auth, same-origin console, Cloudflare TLS, Compose import, static deploys, backups/restore, monitoring webhooks, version drift, an opt-in non-root API + narrow nginx/Docker broker, crash journal + reconcile, and local packaging are in place. Still open: measured onboarding evidence (no beta exists), a tagged and repeatable artifact, the replace failpoint matrix, and a power-loss test. Public beta, signed release, GitHub Actions and email notifications are explicitly out of scope.
- **First priority: close the evidence gaps, not new features.** Every amber gate row is now blocked on a test run, a measurement, or a document — not on missing code.
- **Second: decide the launch shape.** A marketing site (`site/`) and a 30-second launch film (`launch-video-v2/`) are in the tree while every GTM gate is `DEFERRED`. See Q13; until it is answered no public URL is announced.
- **Target user for v1:** the author/one technical operator with a handful of apps on one Linux box — not a public multi-user service.
- **Launch date:** private-use readiness only; no public launch date. Beta/GTM gates are deferred.
- **Positioning:** “The deploy console for the one server that runs everything. One box. Many apps. No surprises.”
- **Current next action:** close the G2 evidence gap (a `replace` failpoint, the step matrix, the power-loss test), then tag a version and install it on a fresh aarch64 host. Answer Q13 before spending more on launch assets.

---

## 1. How to use this document

1. **Start here** before planning a release, writing launch copy, or accepting a feature request.
2. **Do not delete history.** Mark items `DONE`, `WON'T DO`, or `DEFERRED` with a date and a one-line reason. The record is the point.
3. **Use the IDs** in commits, PRs, and issues: `fix(SEC-01): require a session on every route`, `test(DEP-01): distinct-image rollback`.
4. **Every P0/P1 change updates its row** and the “Last verified” date in the same PR.
5. **A gate is not green because the code exists.** It is green only when the stated evidence exists and is reproducible by someone other than the author.
6. **The audit is evidence, not a plan.** It explains how we know what is broken; this document says what we do about it.
7. **No launch date is set until Gate 3 is green.** Dates follow evidence, not the reverse.
8. **If a decision in §5 is reopened, add a new decision entry** rather than rewriting the old one.

### Status legend

| Status | Meaning |
|---|---|
| `OPEN` | Known, not started. |
| `IN PROGRESS` | Someone is actively working; link the PR. |
| `BLOCKED` | Cannot proceed; state the blocker and owner. |
| `DONE` | Acceptance evidence exists and is linked. |
| `DEFERRED` | Deliberately after v1; state the trigger to revisit. |
| `WON'T DO` | Out of scope for the foreseeable product; state why. |

### Current dashboard (2026-10-24)

| Area | Status | One-line reason |
|---|---|---|
| Security / access | 🟠 AMBER | Bearer token (env, never persisted), single-admin HttpOnly sessions + CSRF, Host/Origin allowlists, Cloudflare TLS, non-root API + nginx/Docker broker, request limits, source-IP actor and full redaction landed; no token rotation and no per-user accounts. |
| Deploys are correct | 🟠 AMBER | Rollback tags, estate lock, dry-run, adopt, compose filenames, volumes, replace auto-rollback, static path, crash journal + `reconcile` and the reload-race probe fix are in; replace-path failpoints and the full step matrix are missing. |
| Install / lifecycle | 🟠 AMBER | CI, package-dist, installer, `setup`, uninstaller, non-root and docs exist; no tag, no aarch64/signed artifact, no published upgrade path. |
| Onboarding | 🟠 AMBER | Compose import (five real services), version drift, token gate, import-from-console, same-origin config and domain/TLS landed; measured 15-minute path and a screen recording remain. |
| Console | 🟠 AMBER | Deploy dialog now plans build→transfer→deploy as one job with an honest static path; a11y/contrast evidence still missing. |
| Documentation / trust | 🟠 AMBER | README/docs/SECURITY/CONTRIBUTING/templates exist; screenshots, code of conduct, public support channel and independent review still missing. |
| Tests / CI | 🟢 GREEN (local) | 488 unit tests, 12 Docker e2e, 6 import, 1 import-deploy, 1 import-many, 1 import-failpoint, 2 backup-restore, 1 failure-injection, 14 example, 3 install and 3 spawn-point tests pass. Docker cases self-skip on a host without a daemon (see Phase 2 evidence). No GitHub workflow is used for the one-user v1. |
| Market / GTM | 🟠 AMBER | Positioning and ICP defined; a preticked marketing site (`site/`, 13 routes) and a 30 s launch film (`launch-video-v2/`) exist; no beta, no published URL, pricing untested. |
| **Overall** | **🔴 NOT LAUNCH-READY** | No tagged artifact, no measured onboarding, no replace failpoint matrix, no power-loss test, and the launch shape (Q13) is undecided. |

---

## 2. Product definition

### What swapdock is

**A deploy console for the one Linux server that runs several Docker apps.**

One TOML per app is the source of truth. swapdock generates the nginx front door from it, starts a release beside the live one, gates it on health, flips traffic, drains the old workers, keeps the previous image for rollback, and writes every step to an append-only, redacted run log. It is a single static binary with no agent, no platform database, and no control-plane service. The console is the operator surface; the CLI is first-class.

### What swapdock is not (v1)

- Not a Kubernetes or multi-host orchestrator.
- Not a managed database provider.
- Not a general Docker UI / Portainer.
- Not a PaaS marketplace with one-click templates.
- Not a hosted SaaS that holds customers' deploy credentials.
- Not a monitoring/APM product; it has host checks and deploy verification, not full observability.
- Not a Windows/macOS-host tool.
- Not a tool for someone who cannot use a terminal at all — yet. See §7.

### Positioning (locked for v1)

> **The deploy console for the one server that runs everything.**
>
> *One box. Many apps. No surprises.*

**Primary proof points**
1. **You own the files.** TOML + generated nginx; readable, diffable, recoverable without the tool.
2. **Every change leaves a receipt.** Append-only run log with argv, exit code, duration, redaction; `verify` reads the access log and shows the flip.
3. **The boring parts are enforced.** Port clashes, missing healthchecks, stateful apps on `swap`, and cross-app conflicts are refused by the registry, not remembered by a human.
4. **A console that shows the whole estate**, not one app at a time.

**ICP:** solo operator or 2–3 person agency running 5–30 small Docker apps for clients on 1–3 €6–12/mo Linux VPSes, currently on Coolify or Compose + Caddy + a runbook. They need to answer “what changed?”, roll back at 1 a.m., and not adopt another platform.

**Anti-ICP:** truly non-technical self-hosters (until §7 is true); >3 hosts/Kubernetes; teams requiring SSO/RBAC/compliance; happy Rails/Kamal shops; Windows/macOS hosts; anyone who wants a managed PaaS.

**Do not claim:** that Kamal cannot run multiple apps (it can); that TLS is automatic (it is not); that static apps work (until DEP-04); that dry-run is safe (until DEP-08); that the console is multi-user (until TEAM-01); that it is “easy for non-technical people” (until Gate 3).

### Assumptions

- One Linux host per installation; one admin in v1; root/sudo available for setup.
- Docker Engine + Compose v2 and nginx are present or installed by `setup`.
- The operator controls DNS for the app hostnames and can open 80/443 (or run Cloudflare Tunnel).
- Apps are Docker Compose services; no Kubernetes, no multi-host, no managed database.
- The host is a supported platform (§8); anything else is “unsupported”, and `doctor` says so.
- If an assumption is false, the correct behavior is a clear refusal, not a best-effort guess.

### Glossary (use consistently in code, docs, and marketing)

| Term | Meaning | User-facing label |
|---|---|---|
| Front port | Stable port nginx owns; the tunnel/DNS points here. | “Public port” (advanced only). |
| Back port | One of the two loopback ports a release can occupy. | Hidden. |
| Slot | The pair of back ports assigned to an app for life. | Hidden. |
| Swap | Start the new release on the free back port, health-gate, flip nginx, drain, stop old. | “No-downtime deploy”. |
| Replace | Stop, start on the same port, health-gate. For stateful apps. | “Brief restart”. |
| Release | A git commit name and its image tag. | “Version”. |
| Registry | The directory of one TOML per app. | “Apps” / “App settings”. |
| Run log | Append-only JSONL of every step. | “Activity” / “Deploy history”. |
| Adopt / import | Take over an existing container or Compose app. | “Take over an existing app”. |
| Doctor | Host/compose preflight checks. | “Checks” / “Health checks”. |
| Verify | Access-log verdict for a deploy window. | “Traffic check”. |
| Static | Files served by nginx from a root, with a symlink flip. | “Static site”. |

---

## 3. Launch gates

A gate is **GREEN** only when every criterion has evidence that a second person can reproduce. No gate may be skipped for a launch. If a gate is amber, the launch is not scheduled.

### Gate 0 — Secure by default

**Rule:** no public URL, no tunnel, no shared host, until this gate is green.

| # | Criterion | Evidence required | Status |
|---|---|---|---|
| G0.1 | API binds loopback/Unix socket only by default; public binding is an explicit, audited opt-in. | Serve defaults to `127.0.0.1:8088`; the systemd unit and Cloudflare walkthrough keep the API on loopback. A public bind requires an explicit `--bind`; `--allow-unauthenticated` warns and is loopback-only. | `DONE` |
| G0.2 | Every route except health and login requires authentication; no default credentials; bootstrap token printed once and stored hashed. | Bearer + single-admin session login, cookie/CSRF tests, `/api/login`/`session`/`logout`. The token comes from `SWAPDOCK_API_TOKEN` and is never written to disk, so there is nothing at rest to hash; it is compared in constant time, length-checked (16+) and URL-safe. There is no rotation command and no per-user account; OIDC is deferred. | `IN PROGRESS` |
| G0.3 | Strict CORS (same-origin), CSRF protection, Host/Origin validation; remove the unprefixed mutation routes. | CORS opt-in; unprefixed mutations removed; cookie sessions require `X-CSRF-Token`; `SWAPDOCK_ALLOWED_HOSTS`/`ORIGINS` host-origin policy + unit tests; process-local per-client rate limit (240/min). | `DONE` |
| G0.4 | TLS on the public surface; HTTP→HTTPS redirect; HSTS; no credentials over HTTP. | Cloudflare Tunnel to a same-origin console/API at `console.m4marvin.com` returned HTTPS 200, HTTP 301, and HSTS; session login/CSRF worked over the public host. ACME is not used; tunnel setup is manual. | `DONE` |
| G0.5 | API runs as a dedicated non-root user; systemd hardening applied; privileged operations go through a narrow broker/sudoers entry. | Opt-in `--non-root` runs the API as `swapdock`; it is **not** in the docker group. A disposable systemd container passed `/health` and applied a root-owned nginx config through the root broker. Docker commands use a strict broker allowlist (ps/inspect/pull/tag/stop/rm, compose pull/up/stop/rm/config, build `-t image .`, push); unit tests cover routing and rejection. | `DONE` |
| G0.6 | No arbitrary argv reachable over HTTP; backup programs are allowlisted or run through a non-root, validated broker; no interpreter as `argv[0]`. | Positive program allowlist, argv checks at validation and execution, interpreter/option-injection tests; HTTP-reachable `allow_unsafe` removed. | `DONE` |
| G0.7 | `root` restricted to a realpath under an allowed root; `listen` restricted to loopback/private allowlist; `0.0.0.0` and `/` refused. | Registry normalization/canonicalization tests; `/`, system paths, `..`, wildcard and public listen addresses rejected; render test. | `DONE` |
| G0.8 | Secrets never logged in cleartext: argv, stdout/stderr, `detail`, and `/runs` redacted; log files `0640`; authenticated `/runs`. | Redactor tests cover URL-embedded tokens, JWT/AWS, env names and secret flags; trace tests enforce `0640` and tighten inherited `0664`; API tests require auth on `/runs`. | `DONE` |
| G0.9 | Request timeouts, body limits, auth rate limiting, SSE cap, per-run concurrency cap; blocking I/O off the async runtime; bounded `/verify` reads. | 8-job cap, 2 MiB body, login 5-failure/5-minute throttle, per-client 240/min limiter, 30s request timeout (SSE exempt), 64-stream SSE cap, bounded `/verify`. | `DONE` |
| G0.10 | `POST /probe` restricted to `public_url`/allowlist; blocks loopback, RFC1918, link-local, metadata after DNS resolution; no redirects. | Host allowlist + resolver that rejects loopback/RFC1918/link-local/metadata, then pins curl with `--resolve` to the checked public IP; no `-L`. Unit tests cover address classes and URL/port parsing. | `DONE` |
| G0.11 | A written threat model and `SECURITY.md`; an independent or second adversarial pass; no P0/P1 open. | `SECURITY.md` + threat model exist. Two independent read-only reviews found a backup-RCE P0 and several P1s; all P0/P1s were fixed with tests (positive backup flags, env_clear, XFF, valid-bearer CSRF, broker allowlist/run_capture, journal/durability, old-port/network cleanup). Remaining findings are P2/P3 (root-mode sudoers scope, broker serial acceptance). | `DONE` |
| G0.12 | Every HTTP-triggered run records the authenticated actor and source IP; CLI runs record the invoking user. | HTTP jobs record `api:<forwarded-IP>` (`api` when no forwarded header); CLI runs record `USER`/`SWAPDOCK_ACTOR`. | `DONE` |

**Gate 0 evidence artifact:** a short “red-team retest” note showing the old 3-request chain now fails at authentication and that the backup/root/listen escalation paths are closed.

### Gate 1 — Installable, upgradeable, recoverable

| # | Criterion | Evidence required | Status |
|---|---|---|---|
| G1.1 | One-line install for Linux x86_64/aarch64; no Rust required; checksum and signature verified. | `package-dist.sh` builds a glibc x86_64 bundle and prints a sha256; the installer and uninstaller were exercised in disposable containers. Missing: a tag, an aarch64/musl build, signature verification, and three recorded runs on a fresh host. | `IN PROGRESS` |
| G1.2 | `swapdock setup` installs/validates nginx snippets, systemd service, logrotate, directories, config, and the console; supports `--dry-run`; idempotent. | `swapdock setup [--dry-run] [--dist DIR]` is a root-only wrapper around the reviewed installer; it refuses non-root and forwards `SUDO_USER`. Installer dry-run is tested in `install_files.rs`; the full install/uninstall was exercised in a disposable container. | `DONE` |
| G1.3 | `swapdock uninstall` removes what setup added (sudoers, units, snippets, logrotate, UI), preserves app data, and documents manual leftovers. | Dry-run and real non-root install→uninstall in a disposable systemd container: units/binary/snippets removed, services inactive, `/srv/swapdock` and `/var/log/swapdock` kept, `nginx -t` still passes. | `DONE` |
| G1.4 | Upgrade path from the previous release; registry schema version and migration; `deny_unknown_fields` no longer bricks old files. | `DEFERRED` for the swapdock binary (no previous public release). A real app upgrade was tested instead: Charon v0.1.1→v0.1.2 through replace, volume preserved, rollback to v0.1.1 with data intact. | `DEFERRED` |
| G1.5 | `swapdock export`/`import` covers the registry and state needed to rebuild; docs for restore. | `DEFERRED`: no multi-host migration in the one-user v1; registry files and backups are documented. | `DEFERRED` |
| G1.6 | CI is required and green: fmt, clippy `-D warnings`, unit, integration, Docker e2e, web build/lint, a11y check, docs links, dependency audit. | `DEFERRED` for the one-user v1: the user explicitly does not want GitHub Actions. The same commands are run locally. | `DEFERRED` |
| G1.7 | Published artifacts: crates.io, GitHub Release with signed binaries/checksums, Docker image, SBOM. | `DEFERRED` for the one-user v1: local `package-dist` tarball/checksum only; no GitHub release or crate publish. | `DEFERRED` |
| G1.8 | The console is installed by `setup`, served same-origin, and has no hardcoded private IP or personal transfer target. | `package-dist` now includes `web/dist`; the installer writes `/srv/swapdock/ui` + `swapdock-ui.conf` (proxy `/api` to 8088, HSTS/body limit). Container install and public HTTPS same-origin test passed; no hardcoded host. | `DONE` |

### Gate 2 — Deploys are correct and recoverable

| # | Criterion | Evidence required | Status |
|---|---|---|---|
| G2.1 | Rollback and `up --force` preserve image-tag/digest integrity; two consecutive rollbacks are safe. | Distinct-image e2e; inspect tags and running digest. | `DONE` |
| G2.2 | Two apps deploying concurrently end with both registry `live_port`s matching installed nginx; both serve 200. `apply` takes the same lock. | Barrier test; final `nginx -T` + curl. | `DONE` |
| G2.3 | A crash at any step leaves a durable pending record; startup reconciles; `status` never lies; unfinished runs show `interrupted`, not “Running”. | Durable pending journal + `status.pending` + `swapdock reconcile`; `serve` reconciles before opening the API; pid/boot/interrupted state. Failpoints cover a static crash after flip and a container swap crash after flip; reconcile proves image/health/front and retires the old container. Missing: a failpoint on the `replace` path (no flip, so no journal entry today) and a matrix over every step. | `IN PROGRESS` |
| G2.4 | Static apps either deploy correctly (symlink/root/rollback/SPA) or are removed from docs, UI, `init`, and validator. | Live static deploy test passes in a disposable Ubuntu container with a writable `/srv/www`: release dir, symlink flip, nginx render and registry commit. The release dir is now created by the test; it previously skipped locally. | `DONE` |
| G2.5 | Swap does not reset named volumes or break same-project service discovery; apps with named volumes/dependencies are refused or handled. | Compose model test; file written in volume survives swap. | `DONE` |
| G2.6 | `adopt` keeps the legacy container until the new one passes health and auto-rolls back on failure. | Kill/fail the new container; legacy still serves. | `DONE` |
| G2.7 | `replace` failure auto-restores the previous release through the same health gate. | Fail the new release; old release returns. | `DONE` |
| G2.8 | `--dry-run` reaches `registry-commit` on a new app, spawns nothing, and says “would” rather than “applied”. | CLI trace; no process/step records. | `DONE` |
| G2.9 | `compose.yaml`/`compose.yml` supported; `docker-compose.yml` still works. | Import/deploy with each filename. | `DONE` |
| G2.10 | Validators agree on 7–64 hex, and a bad release cannot poison the estate; commit validates the final state. | Fuzz/boundary tests; `validate` after commit. | `DONE` |
| G2.11 | `apply` does not install an untested config; staging paths are unique; no two writers interleave. | Kill after rename; concurrent apply test. | `DONE` |
| G2.12 | Durability: directory fsyncs after rename; trace fsync policy documented; power-loss test on a disposable host. | Registry writes and static flips `fsync` the file and its directory; the run log is flushed to the kernel but deliberately not `fsync`ed, and `src/trace.rs` says why (a process crash keeps the line, a power cut may not). The power-loss test itself has not been run. | `IN PROGRESS` |
| G2.13 | IPv6 `listen` renders valid nginx; nginx test runs before the green container is promoted. | `listen ::1` test; `nginx -t`. | `DONE` |
| G2.14 | Backups: schedule, retention, restore, restore verification, and optional offsite (S3/B2) for app data and the registry. | `backup --all` + daily timer, `restore`, `verify_command`, `keep`, `offsite_command`; a real SQLite backup→verify→offsite→destroy→restore test passes. No hosted destination is configured. | `DONE` |

### Gate 3 — Onboarding within 15 minutes

**Target:** a technical operator with one existing Compose app and a DNS record can install, import, and deploy without editing Compose or reading source.

| # | Criterion | Evidence required | Status |
|---|---|---|---|
| G3.1 | `swapdock import` detects `compose.yaml`, service, image, container port, healthcheck; allocates front/back ports; writes the registry/override; prints a plan; keeps the old container until healthy; rolls back on failure. | Import tests plus two Docker tests: five real off-the-shelf Compose services (nginx, httpd, caddy, python http.server, busybox httpd) import and deploy; one app deploys a second release and fails a third cutover while the old container survives and the registry is unchanged. | `DONE` |
| G3.2 | No hand-edit of Compose required; `${IMAGE}`/`${APP_PORT}` are generated via override; healthcheck can be added automatically. | Generated override pins `${IMAGE}`/`${ENV_NAME}`; missing healthcheck warns unless `--healthcheck`; the five-service import test deploys without editing any Compose file. | `DONE` |
| G3.3 | Add-app flow in the console (connect source/Compose, choose domain, deploy), not a CLI instruction in the empty state. | New App dialog has an **Import Compose** mode: Compose directory, hostname and optional app name call `POST /api/import` (same engine as CLI) and open the imported app; create mode remains. No screen recording yet. | `IN PROGRESS` |
| G3.4 | Domain/TLS flow: ACME or a guided Cloudflare Tunnel; public URL verified after deploy. | Cloudflare Tunnel path documented. A static app was applied behind real nginx, exposed at `demo-test.m4marvin.com`, and returned HTTPS 200 with HTTP→HTTPS 301 + HSTS; test DNS/tunnel/container were removed. Console TLS was also verified. | `DONE` |
| G3.5 | `doctor` findings are deploy-blocking where they block, have plain-language consequences, and offer a fix/action. | UX test; no-critical-finding run. | `IN PROGRESS` |
| G3.6 | Time to first successful deploy ≤15 min (p50), ≤30 min (p90) across beta operators; install success ≥90%. | Beta log; stopwatch notes. | `DEFERRED`: no beta operators in the private one-user v1. |
| G3.7 | A “demo” path with a real sample app, no user secrets. | `swapdock demo`; docs. | `DEFERRED`: no public demo path in the private one-user v1. |

### Gate 4 — Distribution, docs, and trust

| # | Criterion | Evidence required | Status |
|---|---|---|---|
| G4.1 | README: one-sentence pitch, screenshots/GIF, 60-second quickstart, architecture diagram, “what it is not”, security note, supported platforms. | README review by a stranger. | `IN PROGRESS` |
| G4.2 | Docs: install, setup, import, deploy, rollback, backup/restore, upgrade, uninstall, troubleshooting, security/threat model, FAQ. | Docs site; link check. | `IN PROGRESS` |
| G4.3 | `SECURITY.md`, `CONTRIBUTING.md`, issue/PR templates, code of conduct, support channel. | Security/contributing/templates exist; no code of conduct or public support channel yet. | `IN PROGRESS` |
| G4.4 | Signed releases + checksums + SBOM; reproducible build notes; dependency audit in CI. | `DEFERRED` for the one-user v1: no GitHub Actions/signed public release. Local tarball + checksum; dependency audit command documented. | `DEFERRED` |
| G4.5 | Public comparison pages (Kamal, Coolify, Dokku, Dokploy) that are honest about where swapdock loses. | Published pages. | `DEFERRED`: no public comparison pages in the private one-user v1. |
| G4.6 | Migration guide from Compose + Cloudflare and from Coolify; “one VPS, 10 apps” tutorial. | Published guides. | `DEFERRED`: no public migration guides in the private one-user v1. |
| G4.7 | Read-only demo console with seeded data; 90-second video; 6 GIFs ≤3 MB; press kit. | Demo URL; video; assets. | `DEFERRED`: no public demo/video/press kit in the private one-user v1. |
| G4.8 | Supported-platform matrix and “unsupported host” behavior (`doctor` exits non-zero). | Matrix docs exist; unsupported-host exit behavior not verified. | `IN PROGRESS` |
| G4.9 | Notifications for deploy failure, app down, cert expiry, and disk threshold (webhook; no email in the one-user v1). | Failure webhook + app-down/cert/disk `swapdock monitor` with de-duplication and optional five-minute timer; no retry queue or hosted alert history. | `DONE` |
| G4.10 | Authenticated read-only app logs view in the console. | Log in, view logs; no unauthenticated access. | `DEFERRED` for the private one-user v1; the run log and CLI logs remain available. |

### Gate 5 — Launch-ready GTM

| # | Criterion | Evidence required | Status |
|---|---|---|---|
| G5.1 | 10 beta operators on real domains; 5 screen-share installs; 3 testimonials/case studies. | `DEFERRED`: the product is explicitly a one-user tool for v1; no public beta. | `DEFERRED` |
| G5.2 | No open P0/P1; 30 days of beta with no data-loss/outage incident caused by swapdock. | `DEFERRED`: no beta in the one-user v1. | `DEFERRED` |
| G5.3 | Support model and SLA published; security contact; response expectations; paid install/migration terms. | `DEFERRED`: no public support/SLA for a private one-user tool. | `DEFERRED` |
| G5.4 | Pricing/business model decided; no paid tier that gates security. | `DEFERRED`: no commercial launch; security features stay free. | `DEFERRED` |
| G5.5 | Launch checklist (Show HN, r/selfhosted, Lobsters, X, newsletters/podcasts) with owners and a launch-day runbook. | `DEFERRED`: no public launch in the one-user v1. | `DEFERRED` |
| G5.6 | Metrics baseline and a 30-day post-launch review scheduled. | `DEFERRED`: no public launch in the one-user v1. | `DEFERRED` |

### Gate 6 — Post-launch (first 30 days)

- No new scope for 30 days except P0/P1 fixes and the top three requested fixes.
- Publish a launch retrospective with installs, failures, and what changed.
- Weekly triage; security issues on a 48-hour acknowledgement target.
- First upgrade tested from the launched version on a disposable host.
- Decide whether the non-technical/Coolify path is worth pursuing, using real support data.

### Gate status at a glance

| Gate | Status | Blocking item |
|---|---|---|
| G0 Secure by default | 🟠 AMBER | Token auth/session/CSRF/Host-Origin, rate limits, source-IP actor, redaction, Cloudflare TLS, non-root broker and the retest all landed; only token rotation (G0.2) is still missing, and no reproducible retest note is linked. |
| G1 Installable & recoverable | 🟠 AMBER | Local package/installer/uninstaller/non-root/UI tested in disposable containers; no tag, no signed/aarch64 artifact, no published upgrade path. |
| G2 Deploys correct | 🟠 AMBER | Rollback/lock/dry-run/adopt/compose/volumes/replace/static/backup/restore, crash journal + `reconcile` and the reload-race probe fix are in with e2e; replace failpoints, the full step matrix and a power-loss test remain. |
| G3 Onboarding ≤15 min | 🟠 AMBER | Five real Compose imports + rolling/failed-cutover, version drift/new-app/token gate, console import, Cloudflare TLS console/app URL done; measured time-to-value and a screen recording missing. |
| G4 Distribution & docs | 🟠 AMBER | README/docs/security/notifications done; a marketing site and launch film exist, but screenshots, CoC, support channel and independent review are missing. Signing/press kit deferred. |
| G5 Launch-ready GTM | ⚪ DEFERRED | The user narrowed v1 to a private one-user tool: no beta, public launch, support/SLA or pricing. Q13 may reopen this. |
| G6 Post-launch | ⚪ NOT STARTED | Only after launch. |

### Phase 1 evidence (2026-10-22)

Recorded against the working tree; no live host was mutated and no release was cut.

| Check | Result |
|---|---|
| `cargo test --locked` (all suites) | pass: 484 unit, 12 e2e, 6 import, 1 import-deploy, 1 import-many, 1 import-failpoint, 2 backup-restore, 1 failure-injection, 14 example, 3 install, 3 spawn, 0 doc |
| `cargo test --test e2e_strategies` | 12 passed (88.8s), including distinct-image two-rollback, adopt restart, replace auto-rollback, stale snapshot, named-volume refusal, two-app serialization |
| `cargo test --test import` | 6 passed (real temp Compose fixture; writes registry + override) |
| `cargo test --test e2e_failures` | 1 passed (failed `docker stop` is fatal) |
| `example_registry`, `install_files`, `single_spawn_point` | 14 + 3 + 3 passed |
| `cargo clippy --all-targets -- -D warnings` | pass |
| `cargo fmt --all --check` | pass |
| `pnpm build` | pass |
| `pnpm lint` | 0 errors, 36 warnings |
| `package-dist.sh` | glibc pass; x86_64 musl must be built with `musl-tools` (CI installs it; local host lacks `musl-gcc`) |
| TLS smoke (self-signed) | `serve --tls-cert/--tls-key` answered HTTPS `/health` 200; `/api/apps` without a token returned 401 |
| Session/CSRF smoke | `POST /api/login` 200; cookie `GET /api/apps` 200; POST without CSRF 403; with CSRF 201 |
| Docs link check | 98 links / 17 files resolve |
| CI workflows | actionlint clean; not executed on GitHub |
| Real root install / release / signing / beta | not run |

New coverage: auth (header and SSE-only query token), CORS/route removal, probe SSRF allowlist, root/listen/positive-allowlist backup validation, log mode/detail redaction, static symlink flip + dry-run, pending journal + `reconcile`, Compose `import` (unit + 6 integration), version drift (`live_unbuilt` included), same-origin console config, token gate, failure webhook payload/URL, backup `--all`, optional API TLS, HttpOnly session/CSRF, and the six e2e deploy paths above.

### Phase 2 evidence (2026-10-24)

Recorded against the working tree. No live host was mutated and no release was cut. This host runs without root and without a Docker daemon, so container-backed cases self-skip instead of passing silently.

| Check | Result |
|---|---|
| `cargo test --locked` (all suites) | pass: 488 unit, 2 backup-restore, 1 failure-injection, 6 import, 1 import-deploy, 1 import-many, 1 import-failpoint, 14 example, 3 install, 3 spawn-point, 12 e2e (skipped: no Docker daemon) |
| `cargo clippy --all-targets -- -D warnings` | pass |
| `cargo fmt --all --check` | pass |
| `pnpm build` / `pnpm lint` (console) | pass / 0 errors, 36 warnings |
| `pnpm build` + `pnpm check` (`site/`) | pass: 13 routes with HTML, title, description, h1 and canonical; sitemap for 12 routes |
| Docs link check | 98 links / 17 files resolve |
| `package-dist.sh` | pass: `swapdock-dist/` with the console, units, installer and nginx snippets; glibc dynamic x86_64 binary, sha256 `b56078b1…`, `swapdock 0.2.0`. It prints a checksum but produces no tarball. |
| Real root install / aarch64 / musl / signing / power-loss / beta | not run |

Two defects were found and fixed by this run, which is why the unit count and the suite result changed:

- **D-029 (new, `DONE`)** — a step that timed out killed only its direct child. `sh -c` is not exec-optimised on every host (dash forks), so the surviving command held the stdout pipe and the run waited for its whole lifetime: the 150 ms timeout returned after 30 s. Steps now spawn in their own process group and the group is killed on timeout. Cost: a terminal Ctrl-C no longer reaches the child.
- The static crash-failpoint test panicked on a host where `/srv/www` exists and is root-owned, because it tested existence instead of writability. It now skips, like the live static test.

### Execution order (relative, not calendar commitments)

| Step | Focus | Exit criterion |
|---|---|---|
| **1 (G2 evidence)** | A `replace` failpoint (no flip today, so no journal entry to reconcile), the step matrix over every crash point, and a power-loss test on a disposable host. | G2 green. |
| **2 (G1 artifact)** | Tag the release; install on a fresh x86_64 and aarch64 host; decide signed artifacts or downgrade the claim; run the upgrade test from the tag. | G1 green; one real install someone else could repeat. |
| **3 (G0 close)** | Kill or add token rotation; write the retest note as an artifact (the old 3-request chain, the backup/root/listen paths). | G0 green. |
| **4 (G3/G4 evidence)** | `doctor` UX test, import screen recording, README screenshots, code of conduct and support channel. | G3/G4 green. |
| **5 (Q13)** | Decide private one-user or public launch. Public re-opens CI, signing, beta measurement, comparison/migration pages, the demo console and all of G5. | Decision recorded in §13. |
| **6 (release)** | Run the §8 release checklist and cut the tag. | A named version with evidence behind every gate row. |

Dates are **not** commitments and are not set until Gate 3 is green. If the work takes longer, the launch moves; the gates do not. The weekly phases assume full-time focus; if this is part-time, multiply the duration rather than cutting the gates.

---

## 4. Scope

### v1 — must ship

| ID | Item | Acceptance | Priority | Status |
|---|---|---|---|---|
| SEC-01 | Auth, sessions, CSRF, strict CORS, remove unprefixed routes | G0.2/G0.3 | P0 | `DONE` |
| SEC-02 | TLS/Cloudflare Tunnel + domain flow | G0.4/G3.4 | P0 | `DONE` |
| SEC-03 | Non-root service + narrow privileged broker | G0.5 | P0 | `DONE` |
| SEC-04 | Backup allowlist, root/listen restrictions, no HTTP argv | G0.6/G0.7 | P0 | `DONE` |
| SEC-05 | Secrets/redaction/log permissions/limits/SSRF | G0.8–G0.10 | P0 | `DONE` |
| SEC-06 | Threat model + SECURITY.md + retest | G0.11 | P0 | `DONE` |
| DIST-01 | CI + signed releases + crate + Docker + SBOM | G1.6/G1.7 | P0 | `DEFERRED` |
| DIST-02 | Install script + `setup` + `uninstall` + `upgrade`/migration | G1.1–G1.5 | P0 | `IN PROGRESS` |
| DIST-03 | Console installed same-origin; no hardcoded config | G1.8 | P0 | `DONE` |
| DEP-01 | Rollback/force tag integrity | G2.1 | P0 | `DONE` |
| DEP-02 | Estate lock + apply lock | G2.2 | P0 | `DONE` |
| DEP-03 | Crash journal + interrupted status + reconcile | G2.3 | P0 | `IN PROGRESS` |
| DEP-04 | Static deploy implemented (or removed everywhere) | G2.4 | P0 | `DONE` |
| DEP-05 | Green Compose project/volumes fix or refusal | G2.5 | P0 | `DONE` |
| DEP-06 | Safe adopt, auto-rollback | G2.6 | P0 | `DONE` |
| DEP-07 | Replace auto-rollback | G2.7 | P0 | `DONE` |
| DEP-08 | Dry-run honesty, compose.yaml, validator alignment, apply guard, IPv6, fsync | G2.8–G2.13 | P0 | `DONE` |
| ONB-01 | `import` from Compose + port allocation + healthcheck auto | G3.1/G3.2 | P0 | `DONE` |
| ONB-02 | UI add-app + first-run + domain/TLS + doctor actions | G3.3–G3.5 | P0 | `IN PROGRESS` |
| ONB-03 | Demo path | G3.7 | P0 | `DEFERRED` |
| DATA-01 | Scheduled backups, retention, restore, verification, offsite | G2.14 | P0 | `DONE` |
| DATA-02 | Registry export/import + restore docs | G1.5 | P0 | `DEFERRED` |
| OPS-01 | Notifications: deploy failure, app down, cert/disk | G4.9 | P0 | `DONE` |
| OPS-02 | App logs view (read-only first) | G4.10 | P1 | `OPEN` |
| TEAM-01 | Single admin + “who deployed” attribution + API tokens | G0.2 + G0.12 | P0/P1 | `DONE` |
| DOC-01 | README/docs/security/uninstall/upgrade/backup | G4 | P0 | `IN PROGRESS` |
| QA-01 | CI gates incl. e2e failure injection + a11y/contrast | G1.6 | P0 | `DEFERRED` |
| GTM-01 | Beta, demo, video, comparison/migration pages, pricing | G5 | P0 | `DEFERRED` |

### v1 — should ship (after the P0s are green)

- Environments (staging/prod) and promotion.
- Build args/secrets, private registry auth, Dockerfile path, multi-arch, cache.
- GitHub Actions/GitLab integration and webhook push-to-deploy with HMAC.
- Pre/post build/deploy hooks, migration/release command, smoke test.
- Cron/scheduled jobs and one-off tasks.
- App `exec`/shell (authenticated, audited, ideally opt-in).
- Uptime checks/history, response time, cert expiry, disk alerts.
- Users/roles, audit export, retention.
- Release diff/notes/approvals, maintenance mode.
- Import from Kamal/Coolify/Dokku/Heroku.
- Static asset upload/CDN invalidation.
- Multi-domain, redirects, per-app headers, basic auth/IP allowlist.
- Backup encryption and S3/B2 targets.

### Post-v1 / only if the business case is proven

- Canary/weighted traffic.
- Preview environments / PR deploys.
- One-click templates and managed Postgres/MySQL/Redis.
- Teams/SSO/OIDC/RBAC beyond a single admin.
- Multi-host/roles/HA.
- Billing/cloud control plane.
- Full monitoring/APM/log search.
- Windows/macOS hosts.

### Won't do

- Kubernetes orchestration.
- A general container UI.
- A hosted service that holds customers' deploy credentials.
- Gating security features behind a paid tier.
- Windows/macOS host support.
- SOC2/compliance certification in v1.
- A mobile app.

---

## 5. Decisions

| ID | Decision | Rationale | Date | Status |
|---|---|---|---|---|
| D-001 | Target the technical ICP first; do not market to non-technical self-hosters until Gate 3 is green. | The product cannot yet install/import/TLS for them; support load would be unbounded. | 2026-10-22 | `DECIDED` |
| D-002 | Single-host only in v1; multi-host is explicit non-goal. | Protects focus and the “one box” wedge. | 2026-10-22 | `DECIDED` |
| D-003 | MIT core forever, including auth; no open-core security gating. | Trust is the only durable advantage of a solo-maintained deploy tool. | 2026-10-22 | `DECIDED` |
| D-004 | Keep nginx + Compose as the substrate; do not replace them with a custom proxy. | The generated-nginx model is the differentiator and the exit strategy. | 2026-10-22 | `DECIDED` |
| D-005 | The console and API are same-origin behind TLS; no absolute private API base in shipped assets. | Fixes the config/mixed-content/CSRF class and makes SSE auth possible. | 2026-10-22 | `DECIDED` |
| D-006 | The API runs non-root; a small privileged broker performs only fixed, validated operations. | Root API is the single largest risk; least privilege is table stakes. | 2026-10-22 | `DECIDED — opt-in non-root; broker handles nginx + allowlisted Docker; API has no docker group` |
| D-007 | Auth is in the service, not “put basic auth in front”; bootstrap admin + session, OIDC later. | Path-scoped proxies are bypassable and SSE cannot send headers. | 2026-10-22 | `DECIDED` |
| D-008 | Static apps are v1 scope; implement rather than remove. | The product already advertises them, the author's own UI is static, and it is a common use case. | 2026-10-22 | `DECIDED — live test passes in a disposable container` |
| D-009 | Backups are allowlisted argv in v1; no shell/interpreter and no HTTP-reachable `allow_unsafe` escape hatch. | The HTTP-reachable registry must not be a shell. | 2026-10-22 | `DECIDED — positive program allowlist + argv checks; tests for tar/rsync/sqlite payloads` |
| D-010 | Registry is root-owned/`0640`; writes go through the broker; the deploy user does not edit it directly. | Removes the root-writable-by-user boundary violation. | 2026-10-22 | `DEFERRED — not implemented; non-root API still owns the registry` |
| D-011 | No telemetry by default; any future telemetry is opt-in and documented. | Deploy tools are trusted with infrastructure; default silence is a feature. | 2026-10-22 | `DECIDED` |
| D-012 | Name stays `swapdock` for v1; secure `swapdock.dev`. | Search-empty and ownable; renaming at zero users is cheap but not the bottleneck. | 2026-10-22 | `DECIDED` |
| D-013 | No paid tier in v1; paid install/migration support is the first revenue experiment. | Team features (RBAC/SSO/audit export) do not exist yet; charging for them early would gate trust. | 2026-10-22 | `DEFERRED — one-user v1 has no commercial launch` |
| D-014 | `compose.yaml`/`compose.yml` and `docker-compose.yml` are all supported. | Compose v2 defaults to `compose.yaml`; rejecting it is an avoidable onboarding failure. | 2026-10-22 | `DECIDED` |
| D-015 | The console is the primary interface; the CLI remains first-class and scriptable. | The UI is a differentiator, but the audit log and automation require a CLI. | 2026-10-22 | `DECIDED` |

**Rule for `PROPOSED`:** a proposed decision must be resolved before the gate that depends on it can be marked green. Record the outcome in this table with a date. Do not carry an unresolved proposed decision into a release. The linked gate is the forcing function.

---

## 6. Defect register

Status as of 2026-10-24. Details and line-level evidence are in the [audit](swapdock-pre-launch-audit.md).

### P0 — blocks launch

| ID | Defect | Fix / acceptance | Status |
|---|---|---|---|
| D-001 | Unauthenticated root-capable API; `CORS *`; private-IP UI. | SEC-01/02/03/05; 3-request RCE retest fails; TLS + auth. | `DONE — auth/TLS/non-root broker/SSRF fixes landed` |
| D-002 | `rollback`/`up --force` retag the wrong image; wrong code can go live. | DEP-01; distinct-image rollback test; digest check. | `DONE` |
| D-003 | Concurrent deploys both succeed; one can 502; `apply` unlocked. | DEP-02; estate lock; concurrent test. | `DONE` |
| D-004 | Static apps unimplemented despite docs/UI. | DEP-04; implement or remove. | `DONE — live symlink flip, SPA fallback and rollback tested` |
| D-005 | Swap changes Compose project; named volumes/network reset. | DEP-05; same-project design or refusal test. | `DONE` |
| D-006 | `adopt` removes legacy before health; no rollback. | DEP-06; failure test keeps legacy alive. | `DONE` |
| D-007 | No crash journal between flip and commit; status can lie; runs “live” forever. | DEP-03; failpoint matrix; `interrupted` state. | `IN PROGRESS — journal, reconcile and two failpoints landed; replace path and the full step matrix missing` |
| D-008 | No installable artifact; both documented install paths fail; public repo stale. | DIST-01/02/03; fresh-host install. | `IN PROGRESS — local dist + installer work; no tag, no signed/aarch64 artifact, no fresh public host install` |
| D-009 | No ACME automation; Cloudflare Tunnel is the TLS path for v1. | Public `console.m4marvin.com` HTTPS/301/HSTS test passed; setup is manual and there is no ACME. | `DONE` |
| D-010 | Console hardcodes the author's API IP and `hetzner`; no auth/first-run. | DIST-03/ONB-02; same-origin; setup. | `DONE — same-origin API, token gate + session/CSRF, no hardcoded host` |

### P1 — fix before/with public exposure

| ID | Defect | Status |
|---|---|---|
| D-011 | Same-app deploy uses stale pre-lock snapshot. | `DONE` |
| D-012 | `--dry-run` fails at the health gate / says “applied”. | `DONE` |
| D-013 | Release validators disagree; a 4–6 char release can poison the estate. | `DONE` |
| D-014 | `replace` failure leaves the app down with no auto-rollback. | `DONE` |
| D-015 | Front probe checks only status while the old upstream is serving; failed stop is ignored. | `IN PROGRESS — the reload race is retried (502 then 200 is a pass); a stale old worker that answers 200 still passes, and a failed stop is surfaced but not fatal` |
| D-016 | Durability: no directory fsync after registry rename; trace not fsynced. | `DONE — registry and static flips fsync file + directory; the run log is flushed, not fsynced, and says why` |
| D-017 | Log rotation/SSE stall; unfinished runs never marked interrupted. | `IN PROGRESS` — SSE is resumable with a `since` cursor and rotation is handled when probing, and orphaned runs are marked interrupted; there is no rotation test |
| D-018 | IPv6 `listen` renders invalid nginx. | `DONE` |
| D-019 | `apply` installs before `nginx -t`; no apply lock; shared staging name. | `DONE` |
| D-020 | `compose.yaml` unsupported; healthcheck wiring not checked. | `DONE` |
| D-021 | `validate` passes when compose/healthcheck is missing; `doctor` is the real check. | `IN PROGRESS` |
| D-022 | `adopt` precondition doesn't fit a normal Compose app; no import. | `DONE` — `import` is the path for a Compose app; `adopt` keeps its narrower precondition |
| D-023 | No registry backup/restore, uninstall, upgrade, or migration. | `DONE` — backup/restore/uninstall and an in-place app upgrade shipped; registry `export`/migration is deferred with G1.4/G1.5 |
| D-024 | No users/actor/RBAC; run log cannot say who deployed. | `DONE` — the run records `api:<ip>` or the CLI user as its actor |
| D-025 | No app logs, exec, cron, or notifications. | `DONE — failure/app-down/cert/disk webhooks; app logs, exec and cron deferred (G4.10)` |
| D-026 | UI: one-click production deploy with no site target; false rollback; clean-on-error; contrast/type/labels. | `IN PROGRESS — the dialog plans build→transfer→deploy as one job and has an honest static path; a11y/contrast evidence missing` |
| D-027 | Split-box SSH build script interpolates unquoted values; no registry auth. | `DONE — that script is gone; `web/deploy.sh` quotes and normalises modes; private-registry auth stays post-v1` |
| D-028 | `panic = "abort"` defeats the job panic guard; a panic kills the daemon. | `DONE — the release profile keeps unwinding and the API catches a panicking job` |
| D-029 | A step timeout killed only the direct child; a forked grandchild kept the pipe open, so a 150 ms timeout returned after 30 s. | `DONE — steps run in their own process group, which is killed on timeout; unit test asserts a prompt return` |

---

## 7. “Non-technical” decision

**Current answer: no.** Do not market to fairly non-technical self-hosters until Gate 3 is green and the following are true:

- One-line install with no Rust, checksums, and a working `uninstall`.
- `swapdock import` from an existing Compose app, no Compose edits.
- Automatic TLS/domain (or a guided Cloudflare flow) and a verified public URL.
- Auth on by default; one admin account; no exposed API.
- Console first-run/add-app, not “run this CLI command”.
- `doctor --fix` for missing healthchecks/ports/paths.
- Backup schedule + restore, uninstall, upgrade, and a documented support boundary.
- A 5-minute demo and a read-only live demo.

If those are not true, the honest positioning is **“for operators who already run Docker/nginx”**. Selling to non-technical users before this produces a support burden that a solo maintainer cannot carry and a security surface they cannot evaluate.

**Decision point:** after 30 days of technical-ICP beta, use support data (tickets per install, install success, time-to-value) to decide whether to build the non-technical path. Do not decide from enthusiasm.

---

## 8. Quality, security, and release process

### Definition of Done (every item)

An item is done only when:

- The code change is merged behind a green CI.
- A test reproduces the old failure and passes after the fix (for P0/P1 defects).
- Security-sensitive changes have an explicit review and a retest note.
- User-facing changes have docs and a screenshot/GIF if the UI changed.
- Registry/schema changes have a migration and an upgrade test from the previous release.
- Observability: the run log still tells the truth; no new secret can reach a log.
- A changelog entry exists (or the change is explicitly internal).
- The gate/defect row in this document is updated with the date and evidence link.

### Required CI checks (branch protection)

1. `cargo fmt --check`
2. `cargo clippy --all-targets -- -D warnings`
3. `cargo test` (unit + integration)
4. Docker e2e suite (real containers, fake nginx where appropriate)
5. Failure-injection suite (kill at each step; concurrency barrier; rollback digest)
6. `pnpm build`, `pnpm lint` (0 errors), a11y/contrast check
7. `cargo audit`, `pnpm audit` (or equivalent)
8. Docs link check and `setup`/install shellcheck
9. Secret scan (no committed tokens)
10. Release build for x86_64/aarch64 (signed, checksummed)

### Release checklist

- [ ] All gates for the release are green; no open P0/P1.
- [ ] Changelog updated (Keep a Changelog); migration notes if schema/flags changed.
- [ ] Upgrade test from the previous release on a disposable host.
- [ ] Install test on a fresh supported host.
- [ ] Rollback test of the tool itself (reinstall previous binary; registry still works).
- [ ] Signed artifacts + checksums + SBOM published.
- [ ] Docs/README updated; screenshots refreshed if UI changed.
- [ ] Security notes updated; no new secret/permission regression.
- [ ] Release notes written for humans, not just commits.
- [ ] Post-release monitoring window (24–48h) and a named responder.

### Versioning and migrations

- Semver for the binary; `0.x` until v1, but no breaking registry change without a migration.
- The registry carries a `schema_version` (or a migration is automatic and idempotent).
- `deny_unknown_fields` must not make an older binary reject every file after a newer one wrote a field; either tolerate unknown fields with a warning or provide `swapdock migrate`.
- Never remove a registry field without a deprecation window and a migration note.
- The tool must be able to read the previous release's registry on upgrade.

### Security program

- Threat model in `SECURITY.md` or `docs/security.md`; reviewed at every Gate 0 change.
- Disclosure: `security@swapdock.dev`; 48-hour acknowledgement; coordinated disclosure; credit.
- Least privilege: non-root API, narrow broker, root-owned registry, `0640` logs.
- Secrets: encrypted at rest; never in registry plaintext or argv; redaction tested with URL userinfo, flags, env values, `detail`.
- Auth: Argon2id (or equivalent) for the bootstrap password; secure session cookies; CSRF; rate limiting; no default credentials.
- Supply chain: pinned actions, lockfiles, signed releases, SBOM, dependency audit.
- No telemetry by default; no app data leaves the host.
- Annual (or per-major) independent review once there are real users.

### Accessibility and browser support

- Target WCAG 2.1 AA for the console: contrast, keyboard, labels, focus, reduced motion, zoom 200%.
- Supported browsers: last two versions of Chrome, Firefox, Safari, Edge.
- No horizontal overflow at 390/768/1440; test at 200% zoom and with long names/hostnames.
- The UI must be usable without a mouse; dialogs/palette trap and restore focus.

### Supported platforms (v1)

- Ubuntu 24.04/26.04 LTS, Debian 12/13; x86_64 and aarch64.
- Docker Engine 27+ with Compose v2; nginx 1.24+.
- A domain and DNS control; Cloudflare Tunnel optional.
- No Windows/macOS hosts. No rootless Docker in v1 (document as unsupported).
- `doctor` must fail clearly on unsupported hosts.

---

## 9. Operations and data

### Backups

- **App data:** per-app `[backup]` command (allowlisted) or a documented native tool; schedule, retention, restore, and restore verification. Default off, but `writes_state` without a backup remains a warning that is visible and explained.
- **swapdock state:** registry + generated overrides + UI config; `swapdock export`/`import`; docs for restore on a new host.
- **Offsite:** S3/B2-compatible target, encrypted; optional in v1, required for “production” recommendation.
- **Restore drills:** quarterly for the reference estate; document the result.

### Notifications

- Deploy succeeded/failed, app down/up, cert expiring, disk above threshold.
- Channels: generic webhook only in the one-user v1; email is out of scope.
- No notification contains secrets; link to the run, not the payload.

### Observability

- Run log is the audit trail; app logs are separate and read-only in v1.
- `verify` is the traffic proof; add a UI panel for 5xx rate, upstream flip, and latency.
- `doctor` is a preflight, not a monitoring system; scheduled checks are a later feature.
- No telemetry by default; if added, opt-in with a clear data list.

### Support model

- Free: GitHub Discussions/issues, docs, best-effort.
- Paid: $400 one-off production install/migration; 2 business-day response; no 24/7.
- Security: `security@swapdock.dev`, 48-hour acknowledgement.
- Support boundary: supported platforms and topologies only; `doctor` output required for triage.
- Do not promise “we will fix your Compose file” for free; provide a template and a paid path.

---

## 10. Go-to-market source of truth

### Position

**The deploy console for the one server that runs everything.**  
*One box. Many apps. No surprises.*

### Message hierarchy

1. **You own the files.** One TOML per app; generated nginx; readable, diffable, recoverable.
2. **Every change leaves a receipt.** Run log + `verify` + `--dry-run`.
3. **The boring parts are enforced.** Ports, healthchecks, stateful apps, cross-app conflicts.
4. **A console for the whole estate**, not one app at a time.

### Proof assets to build

- 90-second demo: fleet → deploy → health gate → traffic flip → bad release refused → rollback → run log.
- Six GIFs: fleet, deploy timeline, traffic proof, failed release, rollback, run log.
- Read-only seeded demo console; a stranger can click without installing.
- Screenshots in README: overview, app topology, pipeline, doctor.
- Comparison pages: Kamal, Coolify, Dokku, Dokploy — concede install/TLS/community; win on audit, host inventory, adopt, stateful, console.
- Migration guides: Compose + Cloudflare → swapdock; Coolify → swapdock; Kamal → swapdock (when it makes sense).
- “One VPS, 10 apps” tutorial; “Rollback in 40 seconds” with a real log excerpt.

### Launch sequence

1. **Private beta** (10–12 operators; screen-share installs; fix frictions).
2. **Public beta / launch** (Show HN, r/selfhosted, Lobsters, X; 90-second video; read-only demo).
3. **Follow-up content** (comparisons, migration, case study, podcasts/newsletters).
4. **30-day post-launch** (fix top three, publish numbers, no new scope).

### Pricing (proposal, not final)

- Free: single-host, all core features, auth included.
- Team (post-v1): ~$290/yr per 3 hosts for RBAC/OIDC/audit export/scheduled backups/alerting.
- Paid install/migration: $400 one-off, capped.
- Sponsors: tip jar.
- No hosted control plane that holds customer credentials.

### Realistic outcome

1–3k stars, 300–800 active hosts, $5–15k/yr from support/Team in year one. A strong open-source project and reputation asset; not a venture-scale company unless it becomes the agency console (multi-host, RBAC, per-client isolation, backups).

### Claims audit (must stay true)

- “Kamal cannot run multiple apps” — **false**; do not use.
- “Automatic TLS” — **false**; TLS is a Cloudflare Tunnel or a certificate you pass in. There is no ACME. Say “TLS via Cloudflare Tunnel”.
- “Static sites” — allowed; implemented and tested.
- “Dry-run” — allowed; a dry run reaches `registry-commit` and spawns nothing.
- “Multi-user” — **false**; one admin, no per-user accounts.
- “Easy for non-technical users” — only after Gate 3 and §7.
- “Zero downtime for stateful apps” — **false**; `replace` has deliberate downtime; say so.

---

## 11. Metrics

| Metric | Target | How measured |
|---|---|---|
| Install success on a fresh supported host | ≥90% | Beta sessions; setup logs |
| Time to first successful deploy | p50 ≤15 min, p90 ≤30 min | Stopwatch/notes in beta |
| Deploy success rate (beta) | ≥99% | Run log |
| Rollback correctness | 100% in tests; 0 wrong-image incidents | Distinct-image e2e + incidents |
| Open P0/P1 at launch | 0 | Issue tracker |
| Security findings at launch | 0 P0/P1 | Retest/review |
| Beta operators | 10 by Gate 5 | Beta tracker |
| Support load | <3 tickets per install in month 1 | Issue tracker |
| Console a11y | 0 serious/critical axe findings; AA contrast | CI + manual |
| Docs completeness | Every Gate 4 item exists and is linked | Link check/review |

Vanity metrics (stars, upvotes) are monitored but are **not** launch gates.

---

## 12. Risks

| Risk | Likelihood | Impact | Mitigation | Owner |
|---|---|---|---|---|
| Security incident (root API, secrets) | Medium | Critical | TLS, non-root API + narrow broker (nginx + Docker allowlist), sessions/CSRF, external retest, disclosure policy | Marvin |
| Solo-maintainer bus factor | Medium | High | Docs, CI, reproducible builds, named backup maintainer/succession note | Marvin |
| Support swamp from non-technical users | High if marketed early | High | Gate 3 + §7; narrow ICP; support boundary | Marvin |
| Kamal/Coolify competition | High | Medium | Do not fight on their terms; win on audit/host inventory/adopt/console | Marvin |
| Scope creep into a PaaS | High | High | §4; “won't do”; 30-day no-new-scope post-launch | Marvin |
| Registry migration breaks upgrades | Medium | High | schema version, migrations, upgrade test in CI | Marvin |
| Data loss on swap (volumes) | Low after DEP-05; residual for import/static | Critical | Refuse named volumes/deps; backup; e2e | Marvin |
| Wrong image after rollback | Low after DEP-01 | Critical | Distinct-image e2e; tag-conflict failure; keep as a release test | Marvin |
| Host variance (nginx/Docker versions) | Medium | Medium | Supported matrix; `doctor`; setup preflight | Marvin |
| TLS/DNS dependency | Medium | Medium | ACME + Cloudflare options; clear docs | Marvin |
| Backup failure discovered at restore time | Medium | High | Restore drills; verification; alerts | Marvin |
| Launch before gates are green | Medium | Critical | This document; explicit no-go rule | Marvin |

**No-go rule:** if any P0 is open, Gate 0/1/2 is not green, or the install/upgrade test fails, the launch is postponed. No exceptions for a date, a HN slot, or a conference.

---

## 13. Open questions

| # | Question | Recommendation | Needed by |
|---|---|---|---|
| Q1 | Static apps in v1? | Decided: implement (D-008); path exists, release evidence pending. | DEP-04 |
| Q2 | Auth method first? | Shared bearer token landed; session/login and OIDC are the next layer. | SEC-01 |
| Q3 | TLS default? | Cloudflare Tunnel for the one-user v1; ACME deferred. | SEC-02 |
| Q4 | Secrets storage? | Built-in encrypted store + OS key; no plaintext registry. | SEC-05 |
| Q5 | Backup allowlist vs arbitrary argv? | Decided: positive program allowlist plus argv checks; no HTTP-reachable unsafe escape hatch. | SEC-04 |
| Q6 | Single admin vs multi-user in v1? | Single admin + API tokens + actor in log; RBAC post-v1. | TEAM-01 |
| Q7 | Launch date? | Not set until Gate 3 is green. | G5 |
| Q8 | Paid support at launch? | Yes, $400 install/migration; no paid feature tier. | G5.4 |
| Q9 | Public read-only demo? | Yes; required for HN/selfhosted. | G4.7 |
| Q10 | Rename? | No; keep swapdock; secure `.dev`. | — |
| Q11 | Hosted control plane? | No; it would hold customer credentials and create on-call. | — |
| Q12 | Telemetry? | None by default; opt-in only with a documented data list. | G0/G4 |
| Q13 | Public launch or private one-user tool? | A marketing site (`site/`) and a 30 s launch film (`launch-video-v2/`) are in the tree while every GTM gate is `DEFERRED` under the private framing. **Recommended:** answer before writing more launch copy. A public launch re-opens G1.6/G1.7 (CI, signed artifacts), G3.6 (beta measurement), G4.5–G4.7 (comparisons, migration guide, demo console) and all of G5. | Launch planning |

---

## 14. Repository map and references

| Path | Purpose |
|---|---|
| `src/cli.rs` | CLI surface, dispatch, command implementations. |
| `src/api.rs` | Axum HTTP API; auth boundary to add. |
| `src/deploy.rs` | Swap/replace/adopt/rollback flows; the P0s live here. |
| `src/apply.rs` | Atomic nginx apply, test, reload, restore. |
| `src/registry.rs` | App model, TOML load/save, validation. |
| `src/validator.rs` | Cross-app and tunnel validation. |
| `src/render.rs` | Pure nginx renderer. |
| `src/health.rs` | Health gate and HTTP probe. |
| `src/backup.rs` | Backup command runner; needs schedule/restore/allowlist. |
| `src/trace.rs` | Append-only run log: actor, pid/boot, redaction, flush policy. |
| `src/redact.rs` | Secret masking for argv, stdout/stderr, `detail` and query strings. |
| `src/reconcile.rs` | Proves or rolls back a deploy whose process died mid-run. |
| `src/exec.rs` | The single subprocess chokepoint; owns timeouts and process-group kill. |
| `web/` | React console (operator surface). |
| `site/` | Prerendered marketing site (TanStack Start, 13 routes); GTM deferred, see Q13. |
| `launch-video-v2/` | 30-second launch film rendered from Rust + SVG; GTM deferred, see Q13. |
| `install/` | Installer, nginx snippets, systemd, logrotate. |
| `tests/` | Unit/integration/e2e; add failure injection and concurrency. |
| `examples/` | Fictional estate; useful for docs/tests, not a quickstart. |
| `swapdock-pre-launch-audit.md` | Point-in-time evidence and full findings. |
| `LAUNCH.md` | This document. |

**References**
- [swapdock pre-launch audit](swapdock-pre-launch-audit.md)
- [Kamal proxy docs](https://kamal-deploy.org/docs/configuration/proxy/)
- [Kamal installation](https://kamal-deploy.org/docs/installation/)
- [Kamal 2.0](https://dev.37signals.com/kamal-2/)
- [basecamp/kamal](https://github.com/basecamp/kamal)
- [Coolify comparison](https://coolify.io/compare/coolify-vs-dokploy-vs-caprover-vs-dokku-vs-kamal)
- [Coolify pricing](https://coolify.io/pricing/)
- [Dokploy pricing](https://dokploy.com/pricing)
- [Dokku Pro](https://pro.dokku.com/)

---

## 15. Change log of this document

| Date | Change | By |
|---|---|---|
| 2026-10-22 | Created from the audit; gates, scope, decisions, defect register, GTM source of truth. | Marvin |
| 2026-10-22 | Phase 1 implementation: token auth/CORS/probe/root/listen/log hardening; deploy correctness (rollback tags, estate lock, dry-run, adopt, compose resolver, volumes, replace rollback); static symlink deploy; pending journal; version drift; token gate/UI fixes; CI/package/installer/docs. Statuses updated; Phase 1 evidence recorded. | Marvin |
| 2026-10-22 | Phase 1b: `swapdock import`, `restore_command`/`swapdock restore`, `swapdock reconcile`, adopt pending journal, positive backup allowlist with no HTTP unsafe escape hatch, SSH destination validation, apply/PUT lock ordering, SSE cap, trace reads off the async runtime, TLS proxy docs, drift static-state correction. Full suite green. | Marvin |
| 2026-10-22 | Phase 1c: failure webhook for API jobs and CLI backups, `swapdock backup --all`, optional `swapdock-backup.timer` (installer/uninstaller/docs), notification docs. Full suite green. | Marvin |
| 2026-10-22 | Phase 1d: optional API TLS (rustls/ring) with CLI/systemd/docs and a self-signed HTTPS smoke test; single-admin HttpOnly session cookie + CSRF for the console, with login/session/logout endpoints and frontend gate fallback for older APIs. Full suite green. | Marvin |
| 2026-10-22 | Phase 1e: Host/Origin allowlist (`SWAPDOCK_ALLOWED_HOSTS`/`ORIGINS`) with unit tests, 2 MiB request-body limit; session login smoke (200), CSRF rejection (403) and accepted mutation (201). | Marvin |
| 2026-10-22 | Phase 1f: per-key login throttle (5 failures / 5 minutes), `run_start.actor` (`api` or CLI user) exposed in run summaries, systemd hardening baseline for API/backup units. Full suite green. | Marvin |
| 2026-10-22 | Phase 1g: `swapdock monitor` (app-down, certificate-expiry, disk-threshold checks; de-duplicated webhook), optional `swapdock-monitor.timer`, installer/uninstaller/docs. | Marvin |
| 2026-10-22 | Phase 1h: backup `verify_command` and `keep` retention (with tests), UI/docs; live static deploy test (CI creates `/srv/www`; local skip). | Marvin |
| 2026-10-22 | Phase 1i: backup `offsite_command` (rclone/rsync path) with execution-time allowlist and failed-copy test; UI/docs. | Marvin |
| 2026-10-22 | Phase 1j: real SQLite backup→verify→offsite→destroy→restore drill (`tests/backup_restore.rs`), plus retention pruning test. | Marvin |
| 2026-10-22 | Phase 1k: keyless Sigstore signing + SPDX SBOM in `release.yml`, duplicate download-artifact fix, release verification docs. Not yet run on GitHub. | Marvin |
| 2026-10-22 | Phase 1l: import→first-deploy Docker test (`tests/import_deploy.rs`): imported Compose service starts, health-gates, applies nginx, probes the front door and commits. | Marvin |
| 2026-10-22 | Phase 1m: non-root API + root broker (Unix socket, token-authenticated) for nginx and an allowlisted Docker argv; opt-in installer mode, hardened units; tested under systemd in a disposable container without a docker group. | Marvin |
| 2026-10-22 | Phase 1n: second imported release deploy (rolling swap with health gate, nginx apply, front probe, commit) in `tests/import_deploy.rs`. | Marvin |
| 2026-10-22 | Phase 1o: public TLS via Cloudflare Tunnel (HTTPS/301/HSTS, session+CSRF), packaged/installed same-origin console at `/srv/swapdock/ui`, non-root API + broker under systemd in a disposable container. | Marvin |
| 2026-10-22 | Phase 1p: fixed green Compose project collision (old container was removed before the new cutover); per-release project; imported-app failed-cutover test now proves old container survives. | Marvin |
| 2026-10-22 | Phase 1q: private-host walkthrough in a disposable systemd container: non-root installer, installed console login, Compose import, first deploy, second deploy and rollback; console/app returned 200. | Marvin |
| 2026-10-22 | Phase 1r: run records carry pid + boot id; `runs`/`/runs` report `Running` only for a live same-boot process and `Interrupted` for an orphaned run. Unit + API tests. | Marvin |
| 2026-10-22 | Phase 1s: `serve` reconciles pending journals before opening the API; startup reconcile test covers an already-committed pending record. | Marvin |
| 2026-10-22 | Phase 1t: static-flip failpoint test: process crash after the symlink flip leaves the pending journal; `reconcile` proves and commits it. | Marvin |
| 2026-10-22 | Phase 1u: DNS-rebinding guard for `/probe`: resolve once, reject non-public addresses, pin curl with `--resolve`; address/URL tests. | Marvin |
| 2026-10-22 | Phase 1v: `POST /api/import` + console “Import Compose” mode (compose dir, hostname, optional name) using the same import engine as the CLI. | Marvin |
| 2026-10-22 | Phase 1w: independent security/deploy review fixes: positive backup flag allowlist, child env_clear, XFF overwrite, valid-bearer CSRF, broker-routed run_capture, bounded broker requests, replace/swap rollback journal cleanup, reconcile image+front evidence, registry/static dir fsync. | Marvin |
| 2026-10-22 | Phase 1x: reconcile also probes the public front door and retires a stale old-port container; per-release Compose green projects are torn down after cutover/rollback, stopping network leaks. | Marvin |
| 2026-10-22 | Phase 1y: broker accepts connections concurrently with a 16-connection cap (no single connection can stall all applies/docker); G0.11/SEC-06 marked done after the independent review findings were fixed. | Marvin |
| 2026-10-22 | Phase 1z: root-mode sudoers narrowed to a wrapper that rejects `serve`, `broker` and path-override flags; non-root mode installs no sudoers. | Marvin |
| 2026-10-22 | Phase 1aa: `swapdock setup [--dry-run] [--dist]` root-only wrapper over the installer; no sudoers/root privilege escalation. | Marvin |
| 2026-10-22 | Phase 1ab: root-mode sudo wrapper; failpoints restricted to debug builds; broker accepts concurrent bounded connections; all security-review P0/P1 fixes verified. | Marvin |
| 2026-10-22 | Phase 1ac: container swap crash-after-flip failpoint test; reconcile proves image/health/front, commits, and retires the old container. | Marvin |
| 2026-10-23 | Phase 1ad: real Charon upgrade test in a disposable systemd host: v0.1.1 → v0.1.2 through replace, health/front 200, data volume preserved (2 files); rollback to v0.1.1 200 with data preserved. | Marvin |
| 2026-10-22 | Phase 1w: independent security review fixes: positive backup flag allowlist (tar -I/-F/abbreviations, sqlite `.shel`), child env_clear, nginx XFF overwrite, real-bearer CSRF check, broker-routed run_capture, bounded broker requests, replace/swap rollback journal cleanup. | Marvin |
| 2026-10-24 | Phase 2 verification against the working tree: 488 unit + all integration suites green, clippy/fmt clean, console and site builds pass, docs links resolve, local dist packages. Fixed two defects found by that run: D-029 (timeouts killed only the direct child, so a 150 ms timeout returned after 30 s — steps now run in their own process group) and the static failpoint test that panicked on a root-owned `/srv/www`. Dashboard, gates, scope and defect rows reconciled with the code; nginx crash reconcile, redaction, adopt/import, notifications, actor, fsync, panic=abort and the split-box script corrected from stale statuses. Q13 added: private one-user tool or public launch, now that `site/` and `launch-video-v2/` exist. | Marvin |

> Next review: after the first Gate 0 change, then weekly. Update “Last verified” and the dashboard every time a gate or P0 status changes.
