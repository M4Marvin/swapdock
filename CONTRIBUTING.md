# Contributing

Thanks for considering a contribution. swapdock is pre-launch: correctness,
security and honest documentation matter more than new surface area.

## Before you start

- Read [LAUNCH.md](LAUNCH.md) (scope, gates, decisions) and
  [docs/security.md](docs/security.md) (current posture). Do not build a feature
  that a gate deliberately defers.
- **No large scope-expanding PRs before v1.** New commands, new providers, a
  plugin system, a hosted control plane, multi-host, RBAC/SSO, or a rewrite of
  the deploy engine need a discussion and a decision entry first. Small fixes,
  tests, docs, and Gate 0/1/2 work are welcome.
- Security issues go to [SECURITY.md](SECURITY.md), not a public issue or PR.
- If your change touches a launch gate or defect, link its ID (`SEC-01`,
  `DEP-03`, `DIST-02`, …) in the PR. The maintainer updates the status row in
  `LAUNCH.md`; do not edit `LAUNCH.md` or the audit in a drive-by PR.

## Development setup

Requirements:

- Rust 1.98+ (`rustup toolchain install stable`), `rustfmt`, `clippy`;
- Docker (for e2e tests; they skip themselves without a daemon);
- nginx (some installer tests skip without it);
- Node 22+ and pnpm 10 for `web/`;
- `shellcheck` and Python 3 for the script/docs checks.

```bash
rustup component add rustfmt clippy
cd web && pnpm install && cd ..
```

## Build and test

Run these before opening a PR:

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --locked --all-targets
cargo test --locked --doc

cd web
pnpm build
pnpm lint
cd ..

bash -n scripts/*.sh install/*.sh
shellcheck scripts/*.sh install/*.sh
python3 scripts/check-docs-links.py
```

Build a dist tree and exercise the install plan (never as root):

```bash
bash scripts/package-dist.sh
bash swapdock-dist/install-swapdock.sh --dry-run swapdock-dist
bash swapdock-dist/uninstall-swapdock.sh --dry-run
```

The Docker e2e tests are real but disposable: they create a temp estate and
clean up their containers. The installer tests run as an unprivileged user and
must never require root.

## The single-spawn-point rule

**All subprocesses go through `src/exec.rs`.** Do not call `Command::new`,
`std::process::Command`, or a shell from any other module. A test fails the
build when a second spawn point appears. If you need a new capability, extend
the chokepoint and add a test; do not bypass it.

Related rules:

- Never build a shell command string from registry/user input.
- Keep argv as an array; no `sh -c`.
- Redact secrets before anything reaches the run log, error text, or console.
- Do not weaken the installer's root/`SUDO_USER` checks, the sudoers validation,
  the `nginx -t` gate, or the atomic apply path to make a test pass.
- Do not add a Rust dependency without explaining why the standard library or an
  existing dependency is insufficient.

## Tests

- A bug fix needs a test that fails before the fix.
- Deploy/rollback/concurrency changes need a failure-injection test where
  practical, not only a happy path.
- Installer changes must keep `tests/install_files.rs` green and stay
  `shellcheck`-clean.
- Docs changes must keep `python3 scripts/check-docs-links.py` green.
- Web changes must build and lint. Do not add screenshots of a real private
  fleet.

## Commit and PR style

- Keep PRs focused. Split unrelated refactors out.
- Use a clear imperative subject; reference the gate/defect ID where relevant:
  `fix(SEC-01): require a token on every API route`.
- **DCO:** sign off every commit (`git commit -s`). There is no CLA. The
  sign-off certifies you wrote the change or have the right to submit it. See
  <https://developercertificate.org/>.
- Fill in the PR template, including the test plan and the safety checklist.
- Do not commit secrets, personal IPs/hostnames/emails, private screenshots,
  `.shots/`, `hetzner/`, or local agent artifacts.
- Do not edit `LAUNCH.md` or `swapdock-pre-launch-audit.md` in a normal PR; the
  maintainer keeps the status/evidence current.
- No force-push to `main`; no rewriting history of a released tag.

## Documentation

- Update the relevant file under `docs/` and the README link table when you add
  or change a user-facing command.
- Be honest: mark unfinished features **planned** and link the gate/defect.
  Do not document a command that does not exist, and do not copy a vendor's
  feature list.
- Use the glossary in `LAUNCH.md` §2 consistently (front port, back port, slot,
  swap, replace, registry, run log, doctor, verify).

## Review and merge

- CI must be green. A maintainer may ask for a narrower change or a design note
  before review.
- Security-sensitive changes need an explicit review and a retest note.
- User-facing UI changes need a non-personal screenshot/GIF and an accessibility
  check where relevant.

By contributing, you agree that your contribution is licensed under the MIT
license in [LICENSE-MIT](LICENSE-MIT).
