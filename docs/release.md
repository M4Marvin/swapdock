# Release process

**Status: no release has been published yet.** Builds, checks and packaging run
locally or on the target host. GitHub is only a git remote: this tree has no
GitHub Actions, GitHub Release, Packages or Pages automation, and none should
be added.

## Build the dist tree

Prerequisites: Rust 1.98+, `musl-tools` for a static x86_64 build, and `cross`
for aarch64. The release profile already strips and uses thin LTO.

```bash
# Native build for this host.
bash scripts/package-dist.sh

# Static x86_64.
rustup target add x86_64-unknown-linux-musl
bash scripts/package-dist.sh --target x86_64-unknown-linux-musl

# aarch64 on a host with cross installed.
bash scripts/package-dist.sh --target aarch64-unknown-linux-musl
```

The script writes `swapdock-dist/` with the binary, the built console, the
app-template placeholder, nginx files, systemd examples, logrotate, the
installer and the uninstaller. `--no-build --binary PATH` packages an existing
binary.

## Run the checks

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --locked --all-targets
(cd web && pnpm build && pnpm lint)
bash -n scripts/*.sh install/*.sh
shellcheck scripts/*.sh install/*.sh   # if shellcheck is installed
python3 scripts/check-docs-links.py
bash swapdock-dist/install-swapdock.sh --dry-run swapdock-dist
```

Docker-backed tests self-skip on a host without a daemon; say so when reporting
the result. Do not describe a release as tested on a clean host until it was.

## Verify the artifact

```bash
sha256sum swapdock-dist/swapdock
file swapdock-dist/swapdock
ldd swapdock-dist/swapdock

# Optional archive for transfer to the target host.
tar -C . -czf swapdock-0.2.0-x86_64-unknown-linux-musl.tar.gz swapdock-dist
sha256sum swapdock-0.2.0-x86_64-unknown-linux-musl.tar.gz
```

A checksum only proves the file matches the manifest. It does not prove who
produced it. This tree does not produce signed artifacts or an SBOM; do not
claim either.

## Install on the target host

```bash
bash swapdock-dist/install-swapdock.sh --dry-run swapdock-dist
sudo bash swapdock-dist/install-swapdock.sh swapdock-dist
```

The installer does not reload nginx, start the API, restart an app or move
traffic. After it finishes, start or restart the API yourself and verify:

```bash
curl -fsS http://127.0.0.1:8088/health
```

## Release checklist

- [ ] The gates required for the release are green.
- [ ] `Cargo.toml` version and the artifact name agree.
- [ ] `CHANGELOG.md` updated; migration notes if a registry flag/schema changed.
- [ ] Local checks above pass, including the web build/lint and docs links.
- [ ] The aarch64 build succeeded if aarch64 is claimed.
- [ ] The checksum is recorded next to the artifact.
- [ ] Install and uninstall on a fresh supported host, with `--dry-run` first.
- [ ] Upgrade from the previous release, if one exists, on a disposable host.
- [ ] No secrets, personal IPs or private-fleet screenshots in the dist tree,
      repo or release notes.

## Still missing

| Gap | Tracking |
|---|---|
| A first tagged, repeatable artifact | G1.7 / DIST-01 |
| Reproducible-build notes (pinned toolchain, cross image, dates) | G4.4 |
| A Docker image | G1.7 |
| A tested upgrade path from a previous release | G1.4 |
| Hosted artifact storage owned by the user | G1.7 |

The release is not a verified trust story until those exist.
