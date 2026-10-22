# Security

**Read this before exposing swapdock to anything.** The current build is a
pre-launch, root-capable control plane. The API requires a token by default;
the console exchanges it for an in-memory HttpOnly session and sends a CSRF
header on mutations. There is **no automatic TLS and no multi-user account
system**. The API can terminate TLS with an operator-supplied certificate; no
ACME client is built in. It is not safe on a public or shared host.

The reporting process, scope and 48-hour acknowledgement target are in
[SECURITY.md](../SECURITY.md). This page describes the engineering posture.

## Current posture (honest)

| Control | Status | Evidence / tracking |
|---|---|---|
| API authentication | **Single shared token**, required by default (`SWAPDOCK_API_TOKEN`, 16+ chars). The console exchanges it for an in-memory HttpOnly session cookie (12h); bearer remains for CLI/automation. `--allow-unauthenticated` is a loopback development escape hatch. | G0.2 / SEC-01 partly done; no users, OIDC or rotation. |
| API surface | Mutations mounted under `/api`; the health endpoints and `POST /api/login` are the only unauthenticated routes. | G0.3 partly done; unprefixed mutations removed. |
| TLS / ACME | Optional API TLS (`--tls-cert/--tls-key`); no ACME, redirect or HSTS; console TLS via proxy. | G0.4 / SEC-02 partial. |
| CORS | Optional allowlist via `SWAPDOCK_ALLOWED_ORIGINS` (empty means no CORS layer) | G0.3; not a substitute for auth. |
| CSRF / Origin checks | Cookie sessions require `X-CSRF-Token` on POST/PUT/PATCH/DELETE; bearer clients are exempt. `SWAPDOCK_ALLOWED_HOSTS` enforces a Host allowlist; same-origin and explicit Origin allowlists are checked. Login has a per-key 5-failure/5-minute throttle; general request rate limiting is still open. | G0.3 partial. |
| API bind | Defaults to `127.0.0.1:8088`; public bind is possible with `--bind` | G0.1. Do not pass a public address. |
| Service privilege | Opt-in `--non-root` mode runs the API as the deploy user; a root broker handles nginx and an allowlisted Docker argv (`ps`, `inspect`, `pull`, `tag`, `stop`, `rm`, compose pull/up/stop/rm/config, build `-t image .`, push). | G0.5 materially closed; broker root remains high-value. |
| Arbitrary commands over HTTP | Authenticated registry writes can choose an allowlisted backup program and its argv; a positive allowlist plus argv checks rejects the known `tar`/`rsync`/`sqlite3` exec payloads. No HTTP-reachable unsafe escape hatch. | G0.6 partial; non-root broker still open. |
| Registry path/listen restrictions | `root` rejects system paths/traversal/symlink targets; `listen` is restricted to loopback/private/CGNAT/docs ranges. An allowed-root realpath policy is still open. | G0.7 partial. |
| Secret redaction | Incomplete | G0.8 / SEC-05. |
| Rate limits / body limits | Partial or absent | G0.9 / SEC-05. |
| SSRF protection on probes | Host allowlist (`public_url`/app hostnames); DNS rebinding and redirect behavior still need a live test. | G0.10 partial. |
| Audit actor | `run_start` records an actor (`api` for API jobs, `USER`/`SWAPDOCK_ACTOR` for CLI); no per-user identity or source IP. | G0.12 partial. |
| Log permissions | `0640`; new files use it and an inherited 0664 file is tightened on open. Owner deploy user, group deploy group. | G0.8 partially done; redaction still open. |
| Telemetry | None | D-011. |

This is the reason for the launch rule: **no public URL, no tunnel, no shared
host until Gate 0 is green.**

## Exposure rules

If you run the API at all:

1. **Bind loopback only.** The `serve` default is `127.0.0.1:8088`; keep it.
   Do not use `0.0.0.0`, a tailnet address, or a public interface.
2. **Set a token and keep it out of argv.** `serve` refuses to start without
   `SWAPDOCK_API_TOKEN` (16+ chars). Use an environment file with mode `0600`
   (the `--with-service` installer creates `/etc/swapdock/api.env`); never pass
   the token as a command-line argument, where `ps` can see it. Rotate it if it
   may have leaked.
3. **Do not port-forward 8088.** Do not put it behind a public reverse proxy
   without an authenticated TLS layer.
4. **If you use a TLS proxy**, terminate TLS there, require authentication
   there, and still treat the API token as the real API credential. The console
   uses a single-admin cookie session, so TLS is still the missing layer for
   remote UI access.
5. **Firewall the host.** Only 80/443 (or your tunnel) should be reachable from
   the internet; SSH should be restricted.
6. **Do not run the example systemd unit on a shared host.** It is documentation
   only and is not installed unless you pass `--with-service`; even then it is
   not started and is not a security boundary. It binds loopback.
7. **The console is not distributable.** It has a token-entry screen and
   `web/public/config.json` defaults to same-origin `/api`, but there is no user
   login/session and the installer does not install the UI. Serve it only
   locally against a loopback API or behind TLS (DIST-03/G0.2 planned).

## Threat model (current)

**Trusted:** the local operator with sudo, the Docker daemon, the host kernel,
nginx, and the app images they choose to run.

**In scope:**

- a network client reaching the API without a valid token (or any client when
  `--allow-unauthenticated` is set);
- a malicious or compromised app that can influence registry data or backup
  commands;
- a compromised or careless privileged operation (nginx write, compose up,
  backup argv);
- secrets reaching the run log, argv, or exception text;
- supply-chain compromise of a dependency or build artifact.

**Out of scope for v1:** multi-tenant isolation, RBAC/SSO, SOC 2, Kubernetes
multi-host, and protecting a host where the attacker already has root or the
Docker socket.

## Secrets and the run log

- Do **not** put secrets in a registry TOML. The registry is a file the API and
  the operator both write.
- `SWAPDOCK_API_TOKEN` belongs in the environment or `/etc/swapdock/api.env`
  (`0600`), never in argv, a URL, a screenshot, or an issue. The API also
  accepts `?token=` only on `/api/events` for `EventSource`, which puts the
  token in the URL and therefore in access logs; use it only over loopback/TLS
  and rotate if it leaks.
- Backup commands run with their argv recorded. A secret passed as an argument
  can reach the JSONL run log; redaction of structured `detail` is improved but
  still incomplete (G0.8).
- The installer creates `/var/log/swapdock` as `2770 root:<deploy group>` and
  the log as `0640`, owned by the deploy user. Group members can read the log;
  treat it as sensitive. The planned hardening is redaction plus `0640` with a
  narrower group (G0.8); do not assume the current mode is the final one.
- Treat `/srv/swapdock` and `/var/log/swapdock` as sensitive. They can reveal
  internal paths, image names and command lines.

## Deploy path risks (Gate 2 still open)

Security is not only the API. Gate 2 (deploy correctness) is still listed as red
in [LAUNCH.md](../LAUNCH.md). It tracks the risks that can put the wrong code
live or lose state: rollback image integrity, concurrent deploys, the crash
journal between the nginx flip and the registry commit, static and volume
handling, and adopt safety.

The current tree contains fixes and tests for several of those paths, but the
gate is evidence-based and has not been marked green. Until it is:

- deploy only to a host and apps you can restore from a backup;
- record the running image ID and release before a deploy;
- do not run two deploys at once;
- do not use `adopt` on a production app without a manual fallback.

Read [LAUNCH.md](../LAUNCH.md) and the run log before trusting a deploy on a
real app.

## Planned (Gate 0)

| Work | ID |
|---|---|
| User directory/OIDC, token rotation, rate limiting, full Host/Origin audit (single-admin session + CSRF + allowlists exist) | SEC-01 |
| TLS/ACME and a guided domain flow | SEC-02 |
| Non-root service and a narrow privileged broker | SEC-03 |
| Backup allowlist, root/listen restrictions, no HTTP argv | SEC-04 |
| Secret redaction, log permissions, limits, SSRF | SEC-05 |
| Threat model review and independent retest | SEC-06 |
| Actor attribution in the run log | TEAM-01 |

Until those land, the honest marketing line is: **a deploy console for
operators who already run Docker and nginx, on a host they control, not exposed
to the internet.**

## Reporting a vulnerability

Do **not** open a public issue. See [SECURITY.md](../SECURITY.md):
`security@swapdock.dev` (placeholder), 48-hour acknowledgement target,
coordinated disclosure, no bug bounty. Include the version/commit, host setup,
reproduction steps, and impact. Never include live secrets or customer data.

## If you exposed it by mistake

1. Stop the service and remove the public route or tunnel immediately.
2. Rotate `SWAPDOCK_API_TOKEN` (regenerate `/etc/swapdock/api.env`; any client
   using `?token=` may have leaked it into an access log).
3. Assume the API was reachable and review `/var/log/swapdock/swapdock.jsonl`
   for unexpected commands and releases.
4. Rotate any credentials that were present in the environment, registry or
   backup command lines.
5. Rebuild/redeploy from a known-good image and check `docker ps`,
   `/etc/nginx/conf.d/front-door.conf`, and sudoers for changes.
6. Report it via the security contact.
