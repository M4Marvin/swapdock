# swapdock — pre-launch audit and positioning review

**Date:** 2026-10-22  
**Scope:** current working tree at `/home/marvin/codes/deploy` (including 50 uncommitted files), live console at `http://100.80.96.4:8015/`, live API at `http://100.80.96.4:8088/api` (read-only probes), unit tests, web build/lint, competitor research.  
**Method:** adversarial review of security, deploy correctness, frontend/UX, onboarding, and go-to-market; every material claim below is tied to code, a command I ran, or a public source.

**Contents:** [Verdict](#1-verdict) · [What works](#2-what-already-works) · [Launch blockers](#3-launch-blockers-p0) · [Reliability P1](#4-reliability-and-correctness-p1) · [Frontend](#5-frontend-and-ux-review) · [Non-technical setup](#6-ease-of-setup-for-a-fairly-non-technical-user) · [Kamal & positioning](#7-kamal-comparison-and-positioning) · [Sequence](#8-recommended-sequence) · [Evidence](#9-evidence-and-reproduction-notes)

---

## 1. Verdict

**The engine is real. The product is not launch-ready.**

swapdock is a genuinely well-engineered opinion about deploying several Docker apps on one host: a registry as the source of truth, a deterministic nginx renderer, an atomic apply gated by `nginx -t`, a health-gated swap, a single subprocess chokepoint, a redacted JSONL run log, and 346 passing unit tests. The console is visually strong — better than most open-source deploy tools — and the `doctor`, `adopt`, `verify`, and `prune` ideas are differentiated.

But the gap between "works on my preconfigured host" and "a stranger can install it and trust it with production traffic" is large:

- The HTTP API that can deploy, roll back, and run backups is **unauthenticated, root-privileged, plain HTTP, and `CORS *`**. I proved arbitrary command execution through it locally with two requests.
- **Rollback corrupts release tags**, so a later deploy can run the wrong image while the health gate says OK.
- **Two apps deploying at once can both report success while one 502s.**
- **Static apps are documented and shown in the UI but are not implemented.**
- **Swap starts the new release in a different Compose project**, so named volumes and the default network change; "stateless" apps with uploads can come up on empty data.
- **`adopt` removes the legacy container before the new one passes health**, with no rollback.
- There is **no installable artifact**: the README's `cargo install` command targets a crate that does not exist, the GitHub repo is six commits behind at 0.1.0, and there are no releases, tags, CI, binaries, or Docker image.
- The web console **ships the author's private API IP and a `hetzner` transfer target** and has no setup screen or auth.

**Recommendation:** do not launch publicly this week. Spend 3–5 weeks on the P0 list, launch to a technical ICP first (solo operators/agencies with 5–30 apps on one box), and only then decide whether to productize for non-technical users. If the goal is the broad "fairly non-technical" audience, the required work is much larger: one-line install, automatic TLS/domain, `import` from Compose, auth on by default, a first-run wizard, backups/restore, uninstall, and support. That is a different product and a 6–12 month effort.

**Honest framing:** this is launch-worthy as a sharp open-source tool and a credibility asset for its author. It is not yet a company, and the current UI is an operator console, not a beginner's control panel.

**Why it works for you — and where it stops.** Your host already has nginx, Docker, a tunnel, a hand-written `docker-compose.yml`, and a registry you understand. You deploy one app at a time, from a tree you control, and you have not (as far as the live state shows) exercised rollback, a concurrent deploy, a static app, or an untrusted API client. Those are exactly the paths that break. The design is good; the failures are at the edges that a second user, a second app, or a bad day will find.

### Scorecard

| Dimension | Score | Notes |
|---|---:|---|
| Core deploy design | 8/10 | Port pairs, registry, generated nginx, health gate, run log are coherent and testable. |
| Deploy correctness in edge cases | 4/10 | Rollback tags, concurrency, crash recovery, static, and Compose volumes are broken or missing. |
| Security posture | 2/10 | Root API with no auth, permissive CORS, no TLS, blanket sudo, no rate limiting. |
| Install / distribution | 1/10 | No release artifact; both documented install paths fail. |
| Frontend craft | 7/10 | Distinctive, coherent, responsive, good motion and theming; undermined by density and technical leakage. |
| Frontend clarity for non-technical users | 4/10 | Jargon and internal fields dominate; no add-app/onboarding flow. |
| Frontend safety | 3/10 | Private-IP config, no auth, one-click production deploy with no site target, false rollback, false "clean estate". |
| Frontend state correctness | 5/10 | Good loading/error/empty in places, but error states render as clean, "Latest" can be stale, run detail can claim completion. |
| Accessibility | 5/10 | Good focus/reduced-motion/skip link; light-theme status contrast fails AA; unlabeled searches; menu keys incomplete. |
| Documentation / trust | 3/10 | Dense expert README, no screenshots, no security policy, no uninstall/upgrade/restore. |
| Market differentiation | 7/10 | Audit trail + host-wide registry + console is a real wedge; not unique on deploy mechanics. |
| **Launch readiness today** | **2/10** | Not safe to expose; not installable by a stranger. |

---

## 2. What already works

These are worth protecting; the fixes below should not throw them away.

- **One spawn point.** `src/exec.rs` is the only module that creates a process, enforced by `tests/single_spawn_point.rs`. This is why the run log is trustworthy and why redaction can be applied uniformly.
- **Deterministic renderer.** `src/render.rs` is a pure function; the same registry produces byte-identical nginx config. `render`, `validate`, and `apply` are separated correctly.
- **Apply is gated and restores.** `src/apply.rs` stages, backs up, renames atomically, runs `nginx -t`, restores and re-tests on failure, reloads, and checks the master pid did not change. The main path is solid.
- **Validation catches real estate mistakes.** Port collisions, duplicate hostnames, duplicate slots, `writes_state` + `swap`, unknown fields, bad release names, missing health URLs, and tunnel route mismatches are all checked.
- **Run log.** Append-only JSONL with argv, exit code, duration, per-step detail, redaction, and a resume query. The idea is good; the execution has gaps (see P1).
- **Test suite.** 346 unit tests pass in 0.36 s. `example_registry`, `install_files`, and `single_spawn_point` also pass. The e2e suite needs Docker and is not run in CI (there is no CI).
- **Console craft.** The design system is coherent: Geist, OKLCH/hex tokens, one accent, semantic success/warning/danger/info, a 12px card radius, dark/light parity, reduced-motion handling, skip link, focus rings, command palette, live run timeline, and a genuinely useful fleet overview.
- **License and repo.** MIT, public GitHub, no copyleft surprises. The core is readable and the docs in source are often excellent.

---

## 3. Launch blockers (P0)

### P0-1. The root-capable API has no authentication and permissive CORS

**Evidence**

- `src/api.rs:83` adds `CorsLayer::permissive()`.
- Every mutating route is open: `/apps/{name}/deploys`, `/rollback`, `/build`, `/sync`, `/adopt`, `/backup`, `/probe`, `/prune`, `/apply`, and `PUT /apps/{name}` (`src/api.rs:90-119`). There is no auth extractor, token check, or authorization layer anywhere.
- `install/systemd/swapdock.service:7` runs the server as root (`ExecStart=/usr/local/bin/swapdock serve --bind 100.80.96.4:8088 ...`) with no `User=`, no `ProtectSystem`, and no sandboxing.
- `install/install-swapdock.sh:78` grants `NOPASSWD: /usr/local/bin/swapdock` for the whole binary, so the CLI can perform any root operation the tool supports.
- The live API answered `access-control-allow-origin: *` on a request with `Origin: https://evil.example`.
- `/api/apps` returns the full registry, including `compose_dir`, `repo`, `git_remote`, and `[backup]` commands.

**Proof of code execution (local, not against the live host).** I started the binary on a private temp registry, `PUT` an app with a `[backup]` command of my choosing, then `POST /apps/<name>/backup`. The command ran as my user and created the file. Under the shipped systemd unit, that same sequence runs as root. The chain is:

1. `GET /api/apps` to learn an app name.
2. `PUT /api/apps/<name>` with a modified `backup.command`.
3. `POST /api/apps/<name>/backup` (or a deploy with `before_deploy = true`).

No authentication, no CSRF token, no origin check. This is remote code execution for anyone who can reach the port, and privilege escalation to root under the shipped unit.

**Live observation (safe GETs only).** `GET http://100.80.96.4:8088/api/apps` returned 6,897 bytes listing all 11 apps with `access-control-allow-origin: *` and no `WWW-Authenticate`; `GET /api/render` returned the generated nginx config; `GET /apps` (unprefixed) returned the same registry because the router mounts every route twice (`.nest("/api", api).merge(api)`, `src/api.rs:80-82`). That last point matters for the fix: a proxy rule that basic-auths `/api/` leaves `/apps`, `/apply`, `/backup`, `/deploys`, etc. open.

**Current exposure is tailnet-only, not public — but that is not a boundary.** The live address is in `100.64/10`, and I could not verify public internet reachability. The point is not that it is already open to the internet; it is that the service is unauthenticated to anything that can reach it, the shipped unit binds a non-loopback address, and any compromised tailnet device/laptop gets root. The public launch plan necessarily increases the reachable set; the auth model must exist before that happens.

**Why this is worse than "bind loopback".** The service is not loopback in the shipped unit; it binds a tailnet address. A malicious page in the operator's browser can attempt requests to `127.0.0.1` or a private address; CORS does not prevent the side effect, and `permissive()` allows the browser to read responses where the request is allowed. If the API is ever put behind a public tunnel/domain to make the web UI remotely usable, it is a full compromise. The `EventSource`-based live logs cannot send an `Authorization` header, so auth must be designed deliberately (same-origin cookie or short-lived query token), not bolted on. The shipped UI also calls the API's absolute private URL from `config.json`, so auth on the UI's port does not protect the API; and there is no Host/Origin validation to stop DNS rebinding to loopback.

**A second unauthenticated write can expose the filesystem and bind all interfaces.** `root` is only validated as absolute and free of nginx metacharacters (`src/registry.rs:470-493`); `listen` is only parsed as an IP (`:394-403`); both are rendered verbatim (`src/render.rs:164-170,220-222`). A local `PUT /api/apps/docs` with `root = "/"` and `listen = ["0.0.0.0"]` returned 200, and `GET /api/render?app=docs` produced `listen 0.0.0.0:8003; server_name evil.example.com; root /;`. `POST /api/apply` would install it. That turns the static app into a world-readable file server for `/` (registry TOMLs, `.env`, backups) and publishes an app on every interface, bypassing the tunnel/firewall. This is not a theoretical hardening item; it is a second unauthenticated control-plane primitive.

**Other exposed surfaces from the same root cause:** `/verify?since=...&access_log=/dev/zero` reads an arbitrary path with `read_to_string` on a tokio worker (unbounded OOM, `src/api.rs:1390-1399`); `/probe` accepts any URL and runs `curl` as root, unlogged (`:555-600`); each mutation spawns an unbounded OS thread (`:950`); `/runs` and SSE scan the whole log on the async runtime; `panic = "abort"` in `Cargo.toml` makes the `catch_unwind` job guard in `spawn_job` useless (`src/api.rs:950-972`), so one panic kills the root daemon. The run log is `0644`/`0664` (`src/trace.rs:159-170`, installer, logrotate), and `detail` is written unredacted (`src/trace.rs:385`), so a backup command such as `pg_dump postgres://user:pass@...` can leak through `/api/runs`. In split-box mode, `remote_build_script` interpolates `git_remote` and `image_repo` unquoted into an SSH shell command (`src/builder.rs:202-216`); a non-local `build_host` plus a malicious `git_remote` is code execution on the build host.

**Fix (minimum):**

- Bind `127.0.0.1` by default; serve the SPA and API same-origin behind nginx with TLS.
- Add real auth: a bootstrap admin password/token printed on install, stored hashed; OIDC later. Sign/verify a session cookie (`HttpOnly`, `Secure`, `SameSite=Strict`), or a bearer token for CLI use. Deny by default on every route; allow only `/health` unauthenticated.
- Remove the unprefixed route mount, or allow only `GET /health` there. Do not rely on a path-scoped proxy rule.
- Replace `CorsLayer::permissive()` with an allowlist of the UI origin, or no CORS at all when same-origin; add Host/Origin validation and CSRF.
- Run the service as a dedicated non-root user. Keep a narrow sudoers entry for the exact privileged operations (`nginx -t`, `nginx -s reload`, Docker socket access), not the whole binary. Prefer a small root broker on a Unix socket for fixed operations.
- Add `NoNewPrivileges=yes`, `PrivateTmp=yes`, `ProtectSystem=strict` with explicit writable paths, `ProtectHome=yes`, an empty capability bounding set, and `TasksMax`/`MemoryMax`.
- Add request timeouts, body limits, auth rate limiting, an SSE connection cap, a per-run concurrency semaphore, and move blocking log/file I/O to `spawn_blocking`. Bound `/verify` reads to a max size.
- Restrict `POST /probe` to the app's `public_url` or an allowlist; block loopback/RFC1918/link-local/metadata addresses after DNS resolution; turn off redirects.
- Restrict `root` to a realpath under an allowed root (e.g. `/srv/www`) and `listen` to loopback or an explicit private allowlist; reject `0.0.0.0`/`::` and privileged ports.
- Remove arbitrary argv from the HTTP-reachable registry: allowlist backup programs (no `sh`/interpreters), or run backups through a separate non-root, validated broker.
- Redact `[backup]` command values and structured `detail`; add URL-userinfo masking; make the run log `0640` and `GET /api/runs` authenticated.
- Fix `panic = "abort"` or isolate jobs in processes so a panic cannot kill the daemon; use unique staging paths and one estate lock (also fixes P0-3).
- Quote/validate `git_remote`, `image_repo`, and all values interpolated into the SSH build script; allowlist build hosts.
- Add `SECURITY.md`, a threat model, a disclosure SLA, `cargo audit`/`pnpm audit`, and signed releases before inviting users.

### P0-2. `rollback` and `up --force` corrupt release tags, so wrong code can go live

**Evidence**

`retain_old_image` (`src/deploy.rs:707-719`) is called before the old container is stopped (`src/deploy.rs:376` in swap, `:416` in replace). It takes `app.old_release`, finds the container publishing the **current** `live_port`, and tags that image as `image_repo:old_release`:

```rust
let Some(old_release) = app.old_release.as_deref() else { ... };
let Some(target) = app.image_ref(old_release) else { ... };
...
let id = run.exec(&docker::inspect_image_id(name))?;   // image of the live container
let tag = run.exec(&docker::tag(image_id, &target))?;  // tagged as old_release
```

That is correct for a forward deploy, where the live container is the previous release. It is wrong for `rollback`, where `old_release` is the **target** being deployed. Sequence:

1. `r2` is live; `app.release = r2`, `app.old_release = r1`.
2. `swapdock rollback app` starts `repo:r1` on the other port, health-passes, flips nginx.
3. `retain_old_image` tags the still-running `r2` container's image as `repo:r1`, overwriting the real `r1` tag.
4. The run stops `r2` and commits: `release = r1`, `old_release = r2`.
5. The running container is r1, but `repo:r1` now points at r2's image. The next deploy of r1 pulls and runs r2's code; the health gate passes because it never verifies provenance.

Two consecutive rollbacks swap `repo:r1` and `repo:r2`. `up --release <current> --force` also triggers it: it tags the current image as `old_release`, then `advance` sets `old_release = current` (`src/registry.rs:314-324`), so "rollback" redeploys the same release. The console makes this worse: the rollback action is enabled whenever `old_release != null && live_port != null` (`web/src/routes/apps/$appName.tsx:130`) and does not check `sameRelease(app.release, app.old_release)`. The live `chat` app shows `release == old_release` (both `f437c8a`) and still offers "Roll back now" with a production-change warning. A rollback that cannot move backward should be disabled, and a force-redeploy should never write the rollback target.

The e2e rollback test cannot catch this: it uses `nginx:alpine` for every release, so all image IDs are identical.

**Fix:** `retain_old_image` must receive the release being **replaced** (`app.release`), not `app.old_release`, and must not overwrite a tag that already points at a different image. Better: record the running container's image digest and the release it corresponds to in the trace, and verify the image digest before starting a rollback. Add a test with two distinct images and assert tag/digest integrity across one and two rollbacks.

### P0-3. Concurrent deploys of different apps both succeed; one can 502

**Evidence**

`apply_new_state` loads the registry and renders the new config (`src/deploy.rs:589-636`), then takes the nginx lock (`:637`). The outer lock is per-app (`run_swap`/`run_replace` hold `app_lock_name`), so two different apps can be inside this function at the same time. `swapdock apply` (CLI) takes no lock at all (`src/cli.rs:1000-1010`); the API `apply` handler also takes none (`src/api.rs:860-899`).

Race: A and B deploy concurrently. Each renders the other's pre-commit `live_port`. A applies; B applies last, with a config that points A at its old port. A then stops that old container. B's config is now serving a 502 for A; both runs exit 0. The same clobbering can happen between a deploy and a manual `swapdock apply`, or two apps in two browser tabs.

**Fix:** hold one **estate lock** from registry read through render, apply, stop-old, and registry commit; re-read the registry under that lock. Have `apply` take the same lock. Add a two-app concurrency test with a barrier after render.

### P0-4. Static apps are documented, shown in the UI, but not implemented

**Evidence**

`deploy()` dispatches only on strategy (`src/deploy.rs:268-272`); `run_swap` always starts a compose service (`:534-567`). There is no `Kind::Static` branch and no symlink code anywhere in `src/` (`grep -rn "symlink" src` returns only comments). Running `swapdock --dry-run --registry examples/apps up docs --release abcd1234` (docs is `kind = "static"`) prints `deploying docs ... via swap` and then fails at `health gate failed: no container publishes port 9004; nothing to gate on`.

The README says: "Static apps need `root` instead of ports and images; the flip is an atomic symlink rename and rollback is instant" ([README.md:147-150](README.md#L147-L150)). The validator also tells operators "a static app must use 'swap': the symlink rename is atomic" (`src/registry.rs:511`). The console shows a Static badge and a `swapdock-ui` static app, but the author's own UI is shipped by `web/deploy.sh`, not by swapdock. This is false advertising and a dead end for a common use case.

**Fix:** implement the static path (symlink `<root>/current` to a release directory, gate, rollback by repointing), with tests; or remove `static` from the docs, UI, `init`, and validator until it exists. For a launch, implementing it is not hard and is a good differentiator.

### P0-5. Swap changes the Compose project, resetting named volumes and the network

**Evidence**

Green runs as project `"{}-green"` (`src/deploy.rs:547`), where the base project is the compose directory name (`src/registry.rs:295-301`). `green_override` adds only `container_name` (`src/deploy.rs:258-260`). Compose namespaces named volumes and the default network by project, so `apps_data` becomes `apps-green_data` (empty), and the green container joins a new network. A "stateless" app with an uploads volume starts healthy on empty data and flips; a multi-service compose file (app + redis/db) cannot resolve its siblings in the new project, so it fails health.

`writes_state` is operator-declared and never detected. `doctor` checks bind mounts, not named volumes or inter-service dependencies.

**Fix:** keep the same Compose project and add a uniquely named green service with shared volumes/network (then `writes_state` stays enforced by refusing swap); or pin external volume/network names via `!override`; or inspect the merged model and refuse swap when named volumes or dependencies exist. Do not silently give a new container a different data set.

### P0-6. `adopt` removes the legacy container before health, with no rollback

**Evidence**

`src/deploy.rs:1100-1140` says it plainly: "From here on the legacy container is gone. Everything that could be checked without stopping it has already been checked." It stops the legacy container, `rm`s it, applies nginx, starts the adopted release, and only then runs the health gate and probe. If the new container fails, the app is down, the legacy container is gone, and there is no automatic restore.

For a one-time migration, this is the riskiest operation in the product. The target audience will use it on a live client site. The image is tagged before removal, so a technically skilled operator could reconstruct a container, but a non-technical user cannot.

**Fix:** stop (do not remove) the legacy container until the new one passes; on failure, restart it and leave the registry unchanged. Or perform the cutover with a durable journal and an automatic rollback. Add a failure-injection test.

### P0-7. No crash journal between the nginx flip and registry commit

**Evidence**

`run_swap` applies the new config and reloads (`src/deploy.rs:370`), probes, drains, retains the old image, stops the old container (`:378`), and only then saves the registry (`:380`). A `SIGKILL`/power loss after the flip leaves nginx serving the new release while the registry still says the old release and old port. `status` probes the recorded `live_port` (`src/deploy.rs:235-253`) and reports a plausible lie; a bare `up` refuses "already on old"; `rollback` goes two releases back; `apply` silently reverts traffic.

There is no pending/transaction record. The run has no `run_end`, and `list_runs` reports the run's **mode** (`live`) as its status (`src/api.rs:1409-1435`), so the UI shows "Running" until the 45-minute client timeout. `RunStatus::Interrupted` exists in `src/trace.rs:66` but is never emitted.

**Fix:** write an fsynced pending record (`release`, from/to ports, phase) before the flip; reconcile it against the installed nginx target and registry on startup; expose "interrupted — here is what to do"; make `status` compare the recorded state with the actual nginx upstream/access log. Add crash failpoints.

### P0-8. There is no installable artifact, and both documented install paths fail

**Evidence**

- `README.md:39` says `cargo install --locked swapdock`. crates.io returns `404 {"errors":[{"detail":"crate `swapdock` does not exist"}]}`.
- `README.md:57` says `sudo bash install/install-swapdock.sh ./examples`. The installer requires `$DIST/swapdock`, `$DIST/apps/*.toml`, `$DIST/nginx/*`, and `$DIST/logrotate-swapdock` (`install/install-swapdock.sh:18-37`); `examples/` has only `apps/` and `cloudflared.yaml`, no binary and no `nginx/`. It fails at `./examples/swapdock missing or not executable`.
- The public GitHub repo is at 0.1.0 (Oct 13), six commits behind local, with no tags, no releases, no `.github/`, no CI, no Dockerfile/image, and 0 stars/forks. The local tree is 0.2.0 plus ~5,000 uncommitted changed lines (mostly the new console); many new components are untracked.
- The installer never installs `install/systemd/swapdock.service`; the API/UI deployment is undocumented. `web/deploy.sh` defaults to host `hetzner`, `/srv/www/swapdock-ui`, and `http://100.80.96.4:8015/`.
- The console's `web/public/config.json` ships the author's private API IP and `transferTarget: "hetzner"`; `web/src/api.ts:272-276` defaults to the same. There is no UI setup screen.
- No `SECURITY.md`, `CONTRIBUTING.md`, upgrade guide, uninstall guide, registry backup/restore, or supported-platform matrix.

**Fix:** build a release pipeline before anything else: CI (fmt, clippy, unit + integration, web build/lint, e2e with Docker), signed static musl binaries for x86_64/aarch64 with checksums, a crates.io publish, a Docker image, an install script, and a `swapdock setup` command that installs the binary, nginx snippets, logrotate, systemd service, and UI, with `--dry-run`, `--uninstall`, and a non-root service. Push the current tree and tag 0.2.0. Replace the hardcoded config with a setup-generated `/config.json` or same-origin defaults.

### P0-9. TLS/domain setup is still a manual Cloudflare exercise

The generated front door is HTTP only; `install/nginx/00-swapdock-http.conf` has no TLS server and no ACME. The README says "Cloudflare, Tailscale, or plain DNS in front", but generated nginx listens on `127.0.0.1:<front_port>`, so plain public DNS cannot reach it without a tunnel or an additional public listener. There is no tunnel helper, no domain prompt, no certificate automation, no HTTP→HTTPS redirect, and no public-URL check after deploy. For a "fairly non-technical" user, this is the second wall after install. For a technical user, it is a documented manual step.

Either make Cloudflare Tunnel the first-class, guided default (create/point/verify), or add automatic TLS (Caddy/ACME) and a `--domain` flow. Do not market "easy HTTPS" until one of these exists.

### P0-10. The console is not distributable or safe to expose

- `web/public/config.json` hardcodes `100.80.96.4` and `hetzner` ([web/public/config.json:2-4](web/public/config.json#L2-L4)). The same absolute HTTP URL is in `web/dist/config.json` and displayed in the sidebar. On an HTTPS launch this is mixed-content-blocked; on HTTP it sends the console's traffic to the author's box. A new user who serves `dist/` cannot reach their own API.
- The empty state says "Scaffold a first entry from the CLI" and shows `swapdock init my-app` ([web/src/routes/index.tsx:601-609](web/src/routes/index.tsx#L601-L609)). There is no add-app form, no Git connection, no domain setup, no first-run wizard.
- The API is unauthenticated (P0-1), and the app has no login screen.
- The console is served over plain HTTP on a tailnet IP; there is no documented nginx server block for it.
- The app header exposes `front :8002`, `slot 7`, `release f437c8a`, and `live on :9014` ([web/src/routes/apps/$appName.tsx:395-402](web/src/routes/apps/$appName.tsx#L395-L402), `:495`), contradicting the README's claim that the commit only appears under technical details ([README.md:246-251](README.md#L246-L251)).

**Fix:** generated config, same-origin `/api`, auth, TLS, a real first-run, and a removable "advanced" disclosure for internal fields.

---

## 4. Reliability and correctness (P1)

These are not all launch blockers by themselves, but several will bite a real fleet in the first month.

1. **Same-app deploys use a stale snapshot.** The CLI loads the App before `deploy()` takes the lock (`src/cli.rs:1111-1123`); the API does the same (`src/api.rs:998-1008`). A second `up` waits, then computes the green port from the old `live_port` and can recreate the serving container, skipping a release. Re-read inside the lock.

2. **`up --dry-run` always fails at the health gate.** Dry-run `exec` returns empty stdout (`src/exec.rs:205-220`), so `health-wait-discover` sees no container and aborts. The README says dry-run "records the plan and spawns nothing" ([README.md:90-92](README.md#L90-L92)); it does spawn `docker ps` and fails before showing the plan. I reproduced this on `examples/apps/site` and on a fresh `init` app: `error: health gate failed: no container publishes port 9000; nothing to gate on`. A dry-run must simulate/skip the gate and reach `registry-commit`.

3. **Release validators disagree and can brick the estate.** `up` accepts 4–64 hex (`src/deploy.rs:203`, `src/registry.rs:963-965`); registry validation requires 7–40 (`src/registry.rs:954-956`). `up --release abcd` can succeed, write `release = "abcd"`, and then `refuse_if_broken` rejects every deploy on the host until the TOML is hand-edited. There is no repair command.

4. **`replace` failure leaves the app down.** `run_replace` stops the old container before starting the new (`src/deploy.rs:406-428`); a start or health failure leaves the app hard down with no automatic rollback, for up to ~120 s health plus ~210 s probe. The information needed to roll back is present; use it.

5. **The front probe cannot prove the flip.** `probe_front` (`src/deploy.rs:658-692`) checks only the status while the old container is still serving. A reload that exits 0 but keeps the old config (or a concurrently applied config) passes against the old upstream, then `stop_publishing` removes it → outage, run Ok. It also records the step as `Ok` before checking the status, so a 502 is logged as an OK step and `non_ok` stays 0.

6. **`stop_publishing` ignores `docker stop`/`rm` exit codes** (`src/deploy.rs:785-810`). A failed stop still commits success and leaves the old container holding the port; the next deploy can conflict.

7. **Durability claims are overstated.** `registry::save_app` has no directory fsync (`src/registry.rs:820-846`); `apply` syncs the directory before the rename, not after (`src/apply.rs:329-334`); the trace appends but never fsyncs (`src/trace.rs:185-191`). A power loss can lose the last steps (the ones recovery needs) or the registry rename while nginx already points elsewhere.

8. **Log rotation and SSE.** `logrotate` renames and recreates the file; the process keeps writing to the old inode, so a run spanning rotation disappears from the new path; SSE tails by path/byte offset (`src/api.rs:1644-1700`) stalls and closes after 60 s. Unfinished runs show "live" forever (see P0-7).

9. **IPv6 listen renders invalid nginx.** `listen` accepts an `IpAddr` (`src/registry.rs:395-400`), then renders `listen ::1:8001;` (`src/render.rs:164-171`); nginx needs `[::1]:8001`. This surfaces only at `nginx -t`, after the green container is up.

10. **`apply` installs before testing, and no one locks it.** `apply` renames the new file before `nginx -t` (`src/apply.rs:223-237`); a crash in that window leaves an untested target that may stop nginx on reboot. CLI/API `apply` take no nginx lock, so two writers can interleave backup/restore.

11. **`compose.yaml` is unsupported.** The code hardcodes `docker-compose.yml` (`src/deploy.rs:546,571,1028`, `src/doctor.rs:104`). Compose v2's default filename is `compose.yaml`; a modern user's first attempt fails with `compose-missing`. Support `compose.yaml`/`compose.yml`, or add a `compose_file` field.

12. **`validate` passes when the host cannot possibly deploy.** A fresh `init` app with `compose_dir = "/srv/example/compose"` validates with `0 error(s), 1 warning(s)`; `doctor` then reports `compose-missing`. `validate` is documented as "per-app rules, cross-app clashes, tunnel routes" (registry-only), but the quickstart presents it as readiness. Either make the default quickstart run `doctor`, or make `validate` fail on missing compose/healthcheck wiring and rename the pure check.

13. **`adopt` does not fit a normal app.** It only finds a container already publishing the registry's `front_port` (`src/deploy.rs:1140-1155`). A normal Compose app publishes 80/3000; the user must first edit and restart the live app to the tool's port, then adopt. The UI offers Adopt only when `live_port == null` and does not link from doctor findings. Add `import --from-compose` that detects service/image/port/healthcheck, allocates ports, generates an override, keeps the old container until healthy, and auto-rolls back.

14. **No registry backup/restore or uninstall.** The API keeps a per-app `.toml.bak` on edit (`src/api.rs:267-272`), but deploy commits do not; a bad TOML or a lost `/srv/swapdock/apps` is not recoverable from the UI. There is no uninstall; the installer leaves sudoers, nginx snippets, logrotate, systemd, `/srv`, and logs.

15. **No multi-user attribution or RBAC.** The log records argv/app, not the operator. The API has no users. For a team or agency, this is a gap; for the recommended ICP (solo/2–3 person agency), it becomes relevant quickly.

---

## 5. Frontend and UX review

**Method:** read the route/component/lib code, built the SPA (`pnpm build` passes), ran `pnpm lint` (0 errors, 34 warnings), inspected fresh and existing screenshots at 390/768/1440 px and in both themes, and checked contrast and keyboard patterns.

### Scores

| Dimension | Score | One-line justification |
|---|---:|---|
| Visual craft | 7/10 | Distinctive, coherent, confident; undermined by density and technical leakage. |
| Clarity for a non-technical user | 4/10 | The first screen is full of ports, slots, strategies, SHAs, and run IDs. |
| Action safety | 3/10 | One-click production deploy with no site target, false rollback availability, private-IP config, no auth boundary. |
| Accessibility | 5/10 | Good focus/motion/skip link; light-theme status contrast fails AA; 10–11px text is pervasive; searches unlabeled. |
| State correctness | 5/10 | Error states can render as "clean"; "Latest" can be stale; run detail can claim completion. |
| Mobile | 6/10 | Responsive with no horizontal overflow, but dense and hides navigation; small targets and text. |

### What is good

- **A real design system.** `web/src/index.css` is the single source of truth; tokens are consistent, dark/light parity is careful, and the accent is used for the one primary action. The console looks like a product, not a shadcn demo.
- **Command palette.** `⌘K`/`Ctrl K`, arrow/enter navigation, `Escape`, focus restoration, and a searchable app/action list. This is the fastest path between screens.
- **Fleet overview.** A useful scan: health, attention, runs, latest deploy, app table, host checks, recent activity. The information architecture is right for an operator.
- **Run ledger and live pipeline.** SSE with reconnect/polling fallback, per-step timing, expandable argv/detail, a clean build → transfer → deploy flow. This is the strongest product surface.
- **Doctor.** Grouped findings with severity, app links, filters, and a read-only contract. The idea is excellent; the execution needs actionability.
- **Craft details.** Skip link, `main` landmark, focus-visible rings, reduced-motion media query, no `scale(0)` entrances, one shared relative-time clock, no `dangerouslySetInnerHTML`, error boundary, 404 route.
- **Build health.** `pnpm build` passes; main bundle ~392 KB (123 KB gzip), app detail ~278 KB (73 KB gzip), CSS 93 KB (15 KB gzip). Reasonable for a console.

### What fails for the target user

1. **Jargon on the primary screen.** `front :8002`, `slot 7`, `release f437c8a`, `live on :9014`, `strategy: replace`, `container`, `registry`, `estate`, `fleet`, `swap`, `adopt`, `probe`, `prune`, `verify`, `nginx`. A non-technical user does not know what a slot is, and does not need to. The README claims the commit "only appears under technical details", but the app header and overview table show it all the time ([web/src/routes/apps/$appName.tsx:395-402](web/src/routes/apps/$appName.tsx#L395-L402); [web/src/routes/index.tsx](web/src/routes/index.tsx) app table). This is a direct doc/UI contradiction.

2. **No first-run.** With no apps, the empty state says "Scaffold a first entry from the CLI" and shows `swapdock init my-app` ([web/src/routes/index.tsx:601-609](web/src/routes/index.tsx#L601-L609)). A web-first user is sent to a terminal, then told to refresh. There is no "Add app" form, Git connection, domain setup, or guided deploy. The API has `POST /api/apps` (`init_app`), but the UI does not use it.

3. **The console is hardwired to the author's infrastructure.** `web/public/config.json` points at `100.80.96.4:8088` and defaults `transferTarget` to `hetzner`; there is no settings screen. A new user who serves `dist/` gets a console that cannot reach its API and a transfer target that does not exist.

4. **The "Latest from main" promise is not guaranteed.** `resolve_latest` reads the local `origin/<branch>` without fetching (`src/api.rs:712-740`); the deploy dialog uses that value (`web/src/components/app/DeployDialog.tsx:315-333`) and never calls `sync`. If the checkout is stale, the UI deploys an older commit while saying "Latest from main". Either fetch (read-only) with a timeout, or label it "Last fetched from main" and prompt a sync.

5. **Attention/doctor counts disagree.** The overview's "Attention 4 of 13" is based only on registry `entry.problems` plus down probes (`web/src/routes/index.tsx:476-481`); the Doctor page shows 28 warnings across 12 apps, and the host-checks card shows 28 warnings. A user sees "4 need a look" and then 12 apps with findings. The overview also fetches `doctorQuery` but does not use it for attention. Make one source of truth, or separate "registry" vs "host" clearly.

6. **"Latest run" is actually the latest app deploy.** `runs.find((run) => run.app != null)` (`web/src/routes/index.tsx:497`) skips registry-wide runs; the card can say "24h ago" while Recent activity shows a run 7 minutes ago. Rename the card to "Latest deploy" or use the true latest run.

7. **Doctor findings are warnings even when they block deploys.** `no-healthcheck` is `Problem::warning` (`src/doctor.rs:164-172`) with the text "the deploy gate will refuse it", but the summary says "0 errors / nothing blocking". A user can believe they are ready and then hit a hard failure. Severity must reflect deploy-blocking, and the finding should link to a fix (add healthcheck override / open Operations → Adopt).

8. **No remediation from the console for the most common blockers.** "No healthcheck" requires editing `docker-compose.yml` on the server; "legacy front publisher" says `run swapdock adopt ...` in a CLI command, even though the Operations tab has an Adopt flow. The app-level `doctor` is read-only by design, which is fine, but the empty/attention states should offer the exact next action.

9. **Light-theme status color fails WCAG AA.** The success pill is `text-success` (#0b8f5f) on `bg-success/10` over white: **~3.6–3.7:1** at 11px. Warning and danger pills are **~4.1–4.2:1**. `--subtle-foreground` (#73747c) on `#f8f8f9` is **4.38:1**. These are small text, so the 4.5:1 threshold applies. Dark theme is fine. Fix the light tokens (darker success/warning/danger for text-on-soft; use a `-strong` variant for text) and raise the smallest type from 11px where it carries meaning.

10. **Destructive actions look ordinary.** Prune is an outline button in the same grid as read-only Doctor/Inspect; the confirm dialog exists, but the card and button do not carry the visual weight of a destructive action. Adopt (a one-time cutover that can take a site down) is an accent card, but its risk is not stated. Use danger styling, explicit consequences, and a typed confirmation for adopt/prune.

11. **Density and mobile.** A live DOM scan (1440px and 390px) found **90 of 148 text elements on the app overview below 12px**, with **27 at 10px or smaller** (run IDs, ports, timestamps). At 390px, 61 of 102 are still small. There is no horizontal overflow and no console error, which is good, but the sidebar collapses to a hamburger and the app-detail header wraps several mono labels. The run detail page's `<h1>` is a raw run id (`1a15b16318`), not a human title. It works, but it is an operator's phone view, not a beginner's. Test at 200% zoom and with long app names/hostnames; several labels truncate with no tooltip.

12. **Lint warnings are a code-health signal, not a blocker.** `pnpm lint` reports 34 warnings, mostly React Compiler memoization and `setState-in-effect` in `DeployDialog.tsx` (e.g. resetting state in an effect at `:288-294`). They do not break the build, but they make the state model harder to reason about, especially around app switching and run streaming.

13. **Deploy is two inconsistent products, and neither names the production site.** The header opens a dialog that can start a production deploy with one click and no hostname ("Deploy chat"); the Pipeline opens a confirmation whose title exposes the raw commit ("Deploy f437c8a?") and a `force` option. For a `replace` app, neither says "this restarts chat.m4marvin.com and it will be briefly down". Unify into one flow: target site, current version, new version, primary "Deploy to production".

14. **"Keep this open — closing is disabled" is false.** It only blocks the dialog's own close ([DeployDialog.tsx:599](web/src/components/app/DeployDialog.tsx#L599)); clicking a sidebar link or Back unmounts it and aborts the follow, while the server deploy continues. Use a router blocker and a persistent "Deploying chat… View progress" banner.

15. **The console can show a clean estate while the API is failing.** If `/api/doctor` is aborted, Doctor renders "0 errors · 0 warnings · clean estate" and "Everything checks out" beside "Failed to fetch" ([doctor.tsx:78](web/src/routes/doctor.tsx#L78)); while loading it also shows 0/clean. Runs says "No runs recorded yet" on error ([runs/index.tsx:268](web/src/routes/runs/index.tsx#L268)). The fleet summary ignores a failed `POST /validate` (`errors = validation?.errors ?? 0`, [index.tsx:489](web/src/routes/index.tsx#L489)), so a broken validate endpoint makes the home page look clean. A read should not be a POST. Never render a clean empty state without a successful response; surface last-known data with an error banner.

16. **Run detail can claim a trace completed when the stream closed.** It always resolves `/resume` and SSE on the **deploy** base ([$runId.tsx:179](web/src/routes/runs/$runId.tsx#L179)), even for build runs on `buildApi`; when the stream ends without a terminal status, it sets `ended = "stream closed"` and renders "trace complete" ([$runId.tsx:206](web/src/routes/runs/$runId.tsx#L206), `:480`). It should use the correct base, fall back to `/resume` polling, and only claim completion on a terminal status. The 60s idle timeout also flags a healthy silent Docker step as "stalled".

17. **A missing/malformed `config.json` silently falls back to same-origin defaults** ([api.ts:286](web/src/api.ts#L286)). On a split-box install that means acting on the wrong server with no warning. Validate the config and fail closed ("This build is not configured") rather than guessing.

18. **Smaller accessibility/state issues.** Search inputs have placeholder text but no accessible name ([kit.tsx:314](web/src/components/kit.tsx#L314)); color-only sidebar dots have no screen-reader status; custom `role="menu"`s do not respond to ArrowDown (focus stays on the trigger); hostname links always use `https://` even for HTTP-only apps; static apps with a served root still say "Not deployed" in the release column ([index.tsx:229](web/src/routes/index.tsx#L229)); several 16–23px text buttons are below a comfortable touch target.

### README claims the implementation contradicts

- "The commit … only appears under technical details" — false: app header, home rows, run rows, and the Pipeline confirmation title show it.
- "Build, transfer and deploy … fall back to `/resume` polling" — only the Pipeline/DeployDialog do; run detail marks the stream over and says "trace complete".
- "Static apps are shown as Static" — the health badge is, but the release column says "Not deployed" when `release` is null, even when the app is served from `root`.
- "Same-origin defaults work both through the Vite dev proxy and when the built app is served from either box" — the shipped `config.json` points at a private IP and a personal `hetzner` target; the same-origin default is only a fallback.
- "Every view is a job, not a dump of fields" — the shell is job-shaped, but Doctor is a raw backend-message dump and Configuration is a registry-field form.

### The 10 highest-leverage UI changes

1. Remove `front`, `slot`, `live on`, strategy, and release SHA from the default header; put them behind an "Advanced" disclosure. Lead with: app name, public URL, status, version label, last deploy, one primary action.
2. Add a first-run "Add your first app" flow: connect Git or point at a Compose file, choose a domain, detect/build, deploy. No CLI command in the empty state.
3. Replace the hardcoded `config.json` with same-origin defaults plus a setup screen; never ship the author's IP or `hetzner`.
4. Make `doctor` deploy-blocking findings errors and give each one a button: "Add healthcheck", "Adopt container", "Fix compose path", "Open Configuration".
5. Use one attention model across overview, sidebar, and Doctor; show "registry findings" and "host findings" separately if they differ.
6. Rename "Latest run" → "Latest deploy"; fix the success percentage denominator; label stale source as "Last fetched".
7. Fix light-theme status contrast and raise the 11px metadata text to 12px where it matters. Add a contrast check to CI.
8. Make Prune/Adopt/Rollback visually destructive and state consequences in the dialog; require a typed app name for adopt.
9. Add auth and a user menu; make the console same-origin with the API; add a session-expired state.
10. Add an empty/loading/error state for every panel, with a "what to do next" action; test at 390px, 200% zoom, and with a 60-character app name.

**Screenshots reviewed:** `.shots/ui-overview-dark.png`, `ui-overview-light.png`, `ui-doctor-dark.png`, `ui-operations-dark.png`, `live-version-first.png`, `config-dirty-3.png`, `new-pipeline.png`, `ui-mobile-dark.png`. The overview and deploy dialog are the best assets for the README/launch; use them.

---

## 6. Ease of setup for a fairly non-technical user

**Persona:** competent self-hoster; can rent a VPS, has used Docker Compose, knows what nginx is but cannot debug a bind mount or explain blue/green; wants "my app online with HTTPS, updates without downtime".

**Verdict: no. This persona cannot adopt swapdock today without an expert.** The unaided path fails in about five minutes, at the first command.

### As experienced

1. **Install (fails immediately).** README says `cargo install --locked swapdock`; crates.io says the crate does not exist. The fallback `sudo bash install/install-swapdock.sh ./examples` requires a `swapdock` binary and `nginx/` assets that `examples/` does not contain. There are no GitHub Releases, no Docker image, no installer download. If they build from source (Rust 1.98 required), they then need to assemble a `swapdock-dist` layout the README never describes.
2. **First registry (looks successful, is not).** `swapdock init blog` writes a template; `swapdock --registry . validate` says `0 error(s), 1 warning(s)` even though `compose_dir = "/srv/example/compose"` does not exist and there is no image, no release, and no healthcheck in the compose file. `doctor` then says `compose-missing ... is not readable`. The novice thought "validate" meant "ready".
3. **Deploy preview (fails).** `swapdock up blog --dry-run` prints `deploying blog release ... via swap`, then `error: health gate failed: no container publishes port 9000; nothing to gate on`. The documented safe preview does not reach the plan. `apply --dry-run` prints `applied /path` even though it wrote nothing.
4. **Existing app (more edits).** The tool hardcodes `docker-compose.yml`; a normal `compose.yaml` is reported missing. The compose file must use `${IMAGE}`, a `${APP_PORT}`-style variable, and a healthcheck. `doctor` checks the healthcheck but not whether `${IMAGE}`/`${APP_PORT}` are wired. The user must edit a live compose file and restart.
5. **Adopt (risky).** `adopt` only works if the legacy container already publishes the registry's `front_port`. A normal app publishes 80/3000, so the user must first change and restart the live app to the tool's port. Then adopt stops and removes the legacy container before the new health gate; if it fails, the site is down.
6. **Online (manual).** There is no TLS or tunnel helper. The user must set up Cloudflare Tunnel or another proxy, point each hostname at a front port, and configure nginx. The README's "optional tunnel" paragraph is not a walkthrough. `probe` is optional, so "deployed" can mean loopback-only.
7. **Console (not installable).** There is no UI installer; `web/public/config.json` points at the author's private IP; the empty state tells the user to go back to the CLI. The API is unauthenticated and, if exposed, root.
8. **Recovery (undocumented).** No uninstall, no upgrade guide, no registry backup/restore, no failed-deploy cleanup, no "console is down" bootstrap. The run log exists, but the novice cannot use it.

**Time to value:** 2–5 hours with expert help; unaided, ~5 minutes to the first hard failure. Abandonment: `cargo install` first; then `./examples/swapdock missing`; then the health gate.

### What would make the 5-minute version real

```bash
# 1. One line, no Rust, checksum-verified.
curl -fsSL https://get.swapdock.dev/install.sh | sh

# 2. In the app directory: detect compose.yaml, service, image, port,
#    healthcheck; allocate ports; print a plan and ask y/N.
swapdock import --domain app.example.com --tls auto

# 3. Human-readable result.
swapdock status
#   app.example.com  https 200  release 9c1f2ab  rollback 4d8e0f1
```

To support that, add: published binaries/install script; `swapdock setup` with preflight, nginx snippets, systemd (non-root API), UI, logrotate, `--dry-run`, `--uninstall`; `import` from Compose with auto-detection and keep-old-until-healthy; ACME or a Cloudflare helper; `swapdock demo`; a UI add-app/deploy/rollback flow with auth; docs built around the one-app path. That is the "non-technical" product. It is a significant build, not a docs fix.

---

## 7. Kamal comparison and positioning

### The competitive set

- **Kamal 2** (Basecamp/37signals, MIT, ~14.7k stars, v2.12.0): `gem install kamal` or Docker; `kamal setup` installs Docker, starts `kamal-proxy` on 80/443 with automatic Let's Encrypt, builds/pushes/pulls, starts the new container, health-checks `/up`, flips, stops the old, prunes. YAML `config/deploy.yml`, one app per config, accessories for DBs, secrets, hooks, aliases, rollback, roles/servers. It supports multiple apps on one server (one config per app, proxy host routing). No UI.
- **Coolify** (Apache-2.0, ~63k stars): one-line install, GUI, 360+ templates, automatic TLS, backups, RBAC, Hetzner one-click, $5/mo cloud. The real competitor for "non-technical self-hoster".
- **Dokploy** (~38k stars, $4.50/server/mo), **Dokku** (~32k, MIT, $849 lifetime Pro), **CapRover** (~15k), **Portainer** (~39k, container management), **Railway/Render/Fly** (managed).
- **Roll-your-own:** Compose + Caddy/Traefik + a runbook; Ansible.

### Honest comparison

| Axis | swapdock | Kamal 2 | Coolify | Dokku |
|---|---|---|---|---|
| Install → first deploy | Binary + nginx + root + sudoers; currently requires source build; **both documented paths fail** | `gem install`/Docker; `kamal setup` 3–5 min | One-line Docker; Hetzner one-click; ~10 min | apt/deb + `git push`; ~15 min |
| Required knowledge | TOML, Compose, nginx, ports, sudo | YAML, Dockerfile, registry, SSH | Almost none | Git, Dockerfile, nginx |
| Many apps / one host | **First-class registry** | Supported with one config per app; proxy routing | First-class | First-class |
| Zero-downtime | Swap on a port pair, health gate, 15s drain | kamal-proxy, health `/up` | Traefik/Caddy | Yes-ish |
| Stateful apps | `replace` = deliberate downtime, **enforced** | Accessories; no zero-downtime | Managed DBs/backups | Datastore plugins |
| Rollback | `rollback` command (but **tag bug**, P0-2) | `kamal rollback`; old containers pruned after days | Retained images; no DB undo | No rollback command |
| Audit | **JSONL run log (argv/exit/duration/redacted), `verify`, `doctor`** | `kamal audit` = recent commands | Audit log, 90-day retention | None |
| UI | React console, fleet/app/pipeline/runs/doctor | **None** | Yes, broad | Pro |
| Config model | TOML per app → generated nginx | YAML per app | UI + env | CLI + git |
| TLS / secrets | TLS external (manual tunnel/nginx); redaction only, no secret store | Let's Encrypt single-host; `.kamal/secrets` | Both built in | Plugin/config vars |
| Multi-host | **No** | Yes (needs LB) | Yes | k3s |
| Community / bus factor | 0 stars, 1 maintainer | 14.7k, 37signals | 63k, company | 32k, 1 lead + community |

**Where swapdock genuinely wins:** the per-deploy append-only receipt; host-wide inventory and cross-app validation; `adopt` for legacy containers; machine-enforced `replace` for stateful apps; no platform/agent/DB/Ruby; a deterministic generated nginx; and a console that shows the whole estate, not one app.

**Where it loses:** TLS, install, secrets, backups, RBAC, multi-host, templates, packaging, docs, community, and support. Do not argue deploy mechanics — Kamal owns that mindshare, and Kamal 2 already does multiple apps on one server. Do not argue ease — Coolify wins. Win on **control, auditability, and the one-box fleet view**.

### Recommended position

> **swapdock is the deploy console for the one server that runs everything.**
>
> One static binary, one TOML per app, nginx generated from the registry, blue/green swaps behind a health gate, one-command rollback, and a receipt for every change — on a box small enough that SSHing in and looking still works.

**Tagline:** *One box. Many apps. No surprises.*

**ICP (narrow):** a solo operator or 2–3 person agency running 5–30 small Docker apps for clients on 1–3 €6–12/mo Hetzner/DigitalOcean boxes, today on Coolify or Compose + Caddy + a Notion runbook. Trigger: a bad deploy with no rollback, a client asking "what changed last Tuesday?", or a platform upgrade that took a site down. Fear: being the single point of failure and adding another platform to maintain/hack.

**Anti-ICP (say it out loud):** >3 hosts or Kubernetes ambitions; teams needing SSO/RBAC/compliance; **truly non-technical self-hosters on their own** (they need DNS, TLS, backups, email, and hand-holding; they will generate unbounded support load for a solo maintainer); git-push-and-forget users; happy Rails/Kamal shops; Windows/macOS hosts.

### Hero copy

> **One box. Many apps. No surprises.**
>
> swapdock is the deploy console for a single Docker host. One TOML per app, nginx generated from the registry, blue/green swaps behind a health gate, one-command rollback, and an append-only log of every change. A single static binary, MIT, no agent, no platform database.
>
> **Swap, don't restart.** The new release starts on the free port, passes its healthcheck, traffic flips, old workers drain for 15 seconds, and the previous image stays tagged for rollback. Apps with a database file get an honest `replace` — refused by the registry, not remembered by you.
>
> **Every change leaves a receipt.** argv, exit code, duration, secrets redacted; `--dry-run` on everything; `verify` reads the nginx access log and tells you the second traffic moved; `doctor` finds the problems before 2 a.m.
>
> **Install in 60 seconds** · **Watch the 90-second demo** · **Read the threat model**

(Do not publish the install line until the artifact exists; today it would be a lie.)

### 90-second demo storyboard

| Time | Visual | Voice / on-screen |
|---|---|---|
| 0–8s | Fleet view, 13 apps, 3 live, 4 attention | "One €6 Hetzner box. Thirteen apps. No Kubernetes." |
| 8–24s | Deploy "Latest from main" → step timeline | "New release on the free port, health gate, nginx flip, 15-second drain. Zero 502s." |
| 24–38s | Traffic panel, 0 5xx, upstream flip timestamp | "This isn't our database. It's the nginx access log." |
| 38–54s | Broken release → red step, old release still Live 200 | "A bad release never gets traffic. It is never promoted." |
| 54–70s | `swapdock rollback` → green in 40s | "Rollback is one command, through the same health gate." |
| 70–90s | Run ledger → expand argv → dry-run → end card | "Every change, with receipts. One box. Many apps. No surprises." |

### 90-day plan (condensed)

**Weeks 1–2 — make it installable and safe.** Auth + strict CORS + TLS + non-root service; publish a signed release, crate, CI, install script; push the current tree/tag 0.2.0; fix README install; SECURITY.md; screenshots. Fix P0-2/3/4/5/6/7 (or remove the false static claims).
**Weeks 3–4 — private beta.** 10–12 operators, 5 screen-share installs, fix top 10 frictions; add `swapdock setup`, `import`, `compose.yaml`, dry-run honesty, `doctor` actionability. Write a Coolify→swapdock migration guide and "swapdock deploys swapdock" case study.
**Weeks 5–6 — launch assets.** Landing page + docs, 90-second video, six GIFs, read-only seeded demo (HN cannot install), honest comparison pages (Kamal/Coolify/Dokku), "One VPS, 10 apps" tutorial, changelog.
**Week 7 — launch.** Show HN Tuesday 09:00 ET; r/selfhosted Thursday with a concrete story ("I rolled back a client site in 40s; here's the log"); Lobsters; X thread; answer every comment for 48 hours.
**Weeks 8–12 — retention.** Ship the top three requests within seven days; publish launch numbers; selfh.st/Console.dev/DevOps'ish pitches; two YouTube collabs; podcasts (Self-Hosted, Ship It); awesome-selfhosted PR; 1.0-rc with auth on by default, audit export, uninstall, upgrade/backup docs.

### Business model

Keep the core MIT, including auth. Free: single-host everything. **Team, $290/yr per 3 hosts (or $29/mo):** RBAC, OIDC, audit export/retention, scheduled backups with restore verification, Slack/webhook alerting, multi-host registry read. **Support:** $400 one-off production install/migration, capped. Sponsors as a tip jar. Avoid a hosted control plane that holds strangers' deploy credentials; avoid open-core that gates security.

Realistic outcome: **1–3k stars, 300–800 active hosts, $5–15k/yr** from Team licences plus install work. A strong open-source project and reputation asset, not a venture-scale company. The company version is the agency console (multi-host, RBAC, per-client isolation, backups) — 6–12 months and real support obligations.

### Top GTM risks

| Risk | Mitigation |
|---|---|
| "Why not Kamal?" (14.7k stars, Rails 8 default, already multi-app) | Never argue deploy mechanics. Win on host inventory + receipts + console. Publish the honest comparison and concede the rest. |
| Coolify's polish/distribution (63k, one-click, templates, $5/mo) | Don't be a worse Coolify. Be for people who refuse a platform on the box: no agent, readable files, works when the UI is down, audit trail. |
| Solo-maintainer trust (0 releases, no CI, root API) | Signed releases, CI, SECURITY.md, threat model, 2FA, disclosure policy, a named second maintainer/succession note. Auth before launch. |
| A security incident (root, permissive CORS, nginx reload) | Fix P0-1 now; outside review; threat model; `swapdock security` self-check; no non-technical marketing before login + TLS. |
| Support swamp from non-technical users (DNS, TLS, Cloudflare, Compose, backups) | Narrow ICP; supported-setups matrix; `doctor` exits non-zero on unsupported hosts; paid support with an explicit boundary. |

**Name:** keep `swapdock` for now. It is search-empty and ownable, but "swap" describes the port-pair mechanism, not the benefit; the tagline must carry the meaning. Nearest collision is Dockswap (a blue-green CLI), so secure `swapdock.dev` and avoid claiming it is unique. Renaming at zero users is cheap, but no candidate (slipway/berth/twoslot/handover) is clearly better; naming is not the bottleneck.

---

## 8. Recommended sequence

### Next 7 days (stop the bleeding)

1. Disable/secure the API: bind loopback, add a bootstrap token, strict CORS, no root service, narrow sudoers, TLS in front. Do not expose the current service.
2. Fix rollback/force tag corruption; add a distinct-image test.
3. Add the estate lock around read→render→apply→commit; make `apply` take it.
4. Implement static deploy or remove the static claims from README/UI/validator.
5. Fix the green Compose project (same project or detect named volumes/deps) or refuse swap for those apps.
6. Make `adopt` keep the legacy container until health passes and auto-rollback on failure.
7. Publish the current tree, tag 0.2.0, add CI and signed release binaries; fix the README install commands; ship a `swapdock setup` script.
8. Replace the hardcoded console config; add auth and a first-run path.

### Next 30 days

- `swapdock import --from-compose` (compose.yaml detection, service/image/port/healthcheck, port allocation, override generation, keep-old-until-healthy).
- TLS/domain helper (ACME or Cloudflare) and a public-URL post-deploy check.
- Crash journal + interrupted status + reconciliation; dry-run honest; `compose.yaml`; validator alignment; replace auto-rollback; `apply` lock; IPv6; durability fsyncs.
- Doctor: deploy-blocking severity, action buttons, a "fix" path for no-healthcheck; counts one source of truth.
- UI: remove internal fields from the default header, add add-app wizard, fix light contrast, rename "Latest run", stale-source warning, destructive-action hierarchy.
- Docs: screenshots, quickstart, migration guide, security/threat model, uninstall/upgrade/backup/restore, supported platforms.

### Before public launch

Auth on by default, TLS, signed binaries + checksums, CI badge, crate, one-line install, read-only demo, 90-second video, honest comparison pages, SECURITY.md, registry backup/restore, uninstall. Without these, a Show HN will find the gaps faster than the feature list.

---

## 9. Evidence and reproduction notes

- Unit tests: `PATH="$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$PATH" CARGO_TARGET_DIR=.target cargo test --lib` → 346 passed, 0 failed.
- Web: `cd web && pnpm build` → success; `pnpm lint` → 34 warnings, 0 errors.
- Install: `curl https://crates.io/api/v1/crates/swapdock` → HTTP 404; GitHub API → 0 releases, 0 stars, `pushed_at` 2026-10-13; local `git status` → 6 commits ahead, 17 untracked paths / 50 changed files.
- Static: `swapdock --registry examples/apps --dry-run up docs --release abcd1234` → `health gate failed: no container publishes port 9004`.
- Dry-run: `swapdock --registry . --dry-run up blog --release abcd1234` → `health gate failed: no container publishes port 9000`; trace shows a real `docker-ps` step.
- Validation: `swapdock init blog` + `validate` → `0 error(s), 1 warning(s)` despite `compose_dir` missing; `doctor` → `compose-missing`.
- API: `GET /api/health` → 200 with `access-control-allow-origin: *`; local unauthenticated `PUT` + `POST /backup` executed a command I supplied (file created). No mutating requests were sent to the live host.
- UI screenshots: the `.shots/` set listed in §5; live console at `http://100.80.96.4:8015/` (GET only).
- Competitor facts: [Kamal proxy docs](https://kamal-deploy.org/docs/configuration/proxy/), [Kamal installation](https://kamal-deploy.org/docs/installation/), [Kamal 2.0](https://dev.37signals.com/kamal-2/), [basecamp/kamal](https://github.com/basecamp/kamal), [Coolify comparison](https://coolify.io/compare/coolify-vs-dokploy-vs-caprover-vs-dokku-vs-kamal), [Coolify pricing](https://coolify.io/pricing/), [Dokploy pricing](https://dokploy.com/pricing), [Dokku Pro](https://pro.dokku.com/), GitHub APIs (2026-10-22).

### What I could not verify

- Real Docker swap/replace/rollback against the live host; the deploy-correctness findings are from code paths and local/temp tests, not a production deploy.
- The exact installed systemd unit on the remote host may differ from `install/systemd/swapdock.service`.
- Compose's precise recreate behavior for named volumes/`container_name` was reasoned from Compose semantics, not measured in a live swap.
- The live console screenshots reflect the author's fleet and may be from the uncommitted UI; the public repo is older.

If any of these matter for a go/no-go decision, the next step is a disposable Docker host and a failure-injection run, not more reading.
