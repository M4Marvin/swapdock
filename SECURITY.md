# Security policy

swapdock is a root-capable deploy tool. Security reports are taken seriously,
but this is a pre-launch, solo-maintained project: the response targets below
are best-effort, not a commercial SLA.

## Supported versions

No version has been released yet. Until the first release, security fixes land
on `main` only; there are no backports.

| Version | Supported |
|---|---|
| `main` (pre-launch) | Best-effort fixes only |
| Any tag or release | Not supported yet |

## Reporting a vulnerability

**Do not open a public issue, discussion, or pull request for a vulnerability.**

Email **security@swapdock.dev** (placeholder address until the domain mailbox
is live) with:

- the affected version/commit and install method;
- the host setup (OS, Docker/nginx version, loopback or exposed, token or
  `--allow-unauthenticated`);
- reproduction steps or a proof of concept;
- impact: what an attacker can read, change, or deploy;
- any suggested fix or mitigation.

If email is unavailable, use GitHub's private vulnerability reporting at
`https://github.com/M4Marvin/swapdock/security/advisories/new` — not a public
issue.

**Never include live tokens, customer data, private hostnames, or third-party
secrets.** Redact logs and registry files. We can ask for more detail over a
private channel.

## What to expect

| Stage | Target |
|---|---|
| Acknowledgement | Within 48 hours |
| Initial triage and severity | Within 5 business days |
| Fix or mitigation plan | Agreed with the reporter, based on severity |
| Coordinated disclosure | By agreement; we aim for 90 days after the fix, sooner if a report is already public |

We will credit reporters who want credit. We will tell you if the report is out
of scope or a duplicate, with a reason.

## No bug bounty

There is no paid bug bounty. We do not offer money for reports, and we will not
ask a reporter to pay for disclosure. Good-faith research is welcome under the
scope below.

## Scope

In scope:

- authentication or authorization bypass of the HTTP API, including the
  `?token=` path and the `/api` mount;
- command injection, argument injection, arbitrary argv, or escaping the
  single-spawn-point (`src/exec.rs`) guarantees;
- path traversal, symlink attacks, or writes outside the intended state/nginx
  paths;
- SSRF through probes or public-URL checks;
- secret leakage into the run log, argv, error text, notifications, or the
  console;
- privilege escalation through sudoers, the installer, systemd unit, or
  file permissions;
- deploy-integrity bugs that put the wrong image/code live (rollback tags,
  concurrency, crash recovery);
- supply-chain issues in the build/release workflow.

Out of scope:

- an attacker who already has root, the Docker socket, or the deploy user's
  shell (the tool cannot defend a host from its own administrator);
- misconfiguration that the documentation explicitly warns against (for
  example, exposing the API publicly without TLS/proxy auth);
- missing rate limiting, hardening headers, or TLS **as standalone reports**
  when they are already documented as open Gate 0 items;
- social engineering, physical access, and denial of service from a valid
  token on an already-trusted host;
- vulnerable dependencies without a reachable path (send it anyway if
  uncertain; `cargo audit` is in CI).

The current known risks are documented honestly in
[docs/security.md](docs/security.md). A report that repeats a documented
pre-launch gap may be closed as known, but we still want the details if the
impact is worse than documented.

## Safe harbor

We will not pursue legal action or ask a host to do so for good-faith research
that stays within this policy, avoids privacy violations and data destruction,
and gives us reasonable time to fix before disclosure. This is not a legal
contract or a promise of indemnity; if you need one, contact us before testing.

## Security work in the launch plan

The Gate 0 items in [LAUNCH.md](LAUNCH.md) are the security roadmap: API
sessions and CSRF, TLS, non-root service, argv/path/listen restrictions, secret
redaction, request limits, SSRF protection, and an independent retest. Reports
that map to those items are still useful as evidence.
