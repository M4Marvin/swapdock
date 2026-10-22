# Release process

**Status: the workflow exists and now signs with Sigstore cosign in keyless
mode; no release has been published yet, so no signature has been produced by
this repository.** This page documents how a release is built and verified.

## Trigger

- Push a tag matching `v*` (for example `v0.2.0`): builds both targets, creates
  a GitHub Release, and runs the optional crates.io publish.
- `workflow_dispatch`: builds both targets and uploads workflow artifacts, but
  the `release` and `publish-crate` jobs are skipped because they require a tag.

Workflow: [.github/workflows/release.yml](../.github/workflows/release.yml).

## Build matrix

| Target | How it is built | Toolchain |
|---|---|---|
| `x86_64-unknown-linux-musl` | `cargo build --release --locked --target ...` on `ubuntu-latest` | Host Rust with `musl-tools` |
| `aarch64-unknown-linux-musl` | `cross build --release --locked --target ...` | Containerized cross toolchain (`cross`) |

Notes:

- The aarch64 job uses a temporary `CROSS_CONFIG` with
  `image = ":edge"` for `aarch64-unknown-linux-musl`. The cross image must have a
  Rust toolchain at least as new as `Cargo.toml`'s `rust-version` (1.98); `:edge`
  tracks current stable. For reproducibility, pin a cross image tag instead.
- No QEMU is used: a build with no run/build-script steps compiles natively in
  the cross container. Cross testing would need QEMU and is not part of this
  workflow.
- The release profile already strips (`strip = true` in `Cargo.toml`); the
  workflow checks and strips explicitly if needed, then reports `file`/`ldd`
  output in the log.
- The checked-out tag must match the `version` in `Cargo.toml`, otherwise the
  build job fails on purpose.

## Artifacts

For each target:

```
swapdock-<version>-<target>.tar.gz
swapdock-<version>-<target>.tar.gz.sigstore.json
swapdock-<version>-<target>.spdx.json
swapdock-<version>-<target>.spdx.json.sigstore.json
```

The archive contains the `swapdock-dist/` tree: the static binary, the app
template placeholder, the nginx files, logrotate, the installer and the
uninstaller. A release-wide `SHA256SUMS.txt` is generated in the `release` job
after both archives are downloaded, verified with `sha256sum -c`, and attached
to the release.

## Verify a release

```bash
# 1. Download the archive and SHA256SUMS.txt from the GitHub Release.
sha256sum -c SHA256SUMS.txt

# 2. Inspect the archive before extracting.
tar -tzf swapdock-0.2.0-x86_64-unknown-linux-musl.tar.gz

# 3. Extract and inspect the dist tree.
tar -xzf swapdock-0.2.0-x86_64-unknown-linux-musl.tar.gz
ls -R swapdock-dist

# 4. Preview the installer, then run it as root on a disposable host.
bash swapdock-dist/install-swapdock.sh --dry-run swapdock-dist
sudo bash swapdock-dist/install-swapdock.sh swapdock-dist
```

A checksum proves the download matches the manifest. It does **not** prove who
produced the archive. Verify the Sigstore bundle for the identity and issuer:

```bash
cosign verify-blob \
  --bundle swapdock-<version>-<target>.tar.gz.sigstore.json \
  --certificate-identity-regexp \
    'https://github.com/M4Marvin/swapdock/.github/workflows/release.yml@refs/tags/.*' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  swapdock-<version>-<target>.tar.gz
```

The workflow uses GitHub Actions OIDC, so there is no long-lived signing key in
the repository. A bundle that verifies proves the archive was signed by that
workflow identity; it is not a statement that the code is secure.

## Signing and SBOM

Each archive and SPDX SBOM is signed with `cosign sign-blob` in keyless mode
using the GitHub Actions OIDC identity (`id-token: write`). No long-lived signing
key or `MINISIGN_SECRET_KEY` is used. The signatures are attached as
`*.sigstore.json` next to the artifacts.

The workflow has not run on GitHub yet, so the first real release must verify
the bundle on a clean machine before the artifact is described as signed. If
the keyless step fails, the build job fails; it does not silently publish an
unsigned archive.

## Optional crates.io publish

The `publish-crate` job runs on a `v*` tag:

- if `CARGO_REGISTRY_TOKEN` is set, it runs `cargo publish --locked`;
- if the secret is absent, it runs `cargo publish --locked --dry-run` and logs a
  notice.

The crate is **not published today**; the README install path is the source
build. Do not tell users `cargo install swapdock` works until this job has
actually published and the crate page is live.

## Release checklist

- [ ] All gates required for the release are green; no open P0/P1 that affects
      the release.
- [ ] `Cargo.toml` version and the tag agree (`v<version>`).
- [ ] `CHANGELOG.md` updated; migration notes if a registry flag/schema changed.
- [ ] CI green on the tagged commit (fmt, clippy, tests, web, audit, scripts,
      docs links).
- [ ] The aarch64 cross build succeeded (not just the x86_64 job).
- [ ] `SHA256SUMS.txt` verifies and is attached.
- [ ] The release notes state that artifacts are **signed with keyless Sigstore**
      and show the verification command.
- [ ] Install and uninstall on a fresh supported host, with `--dry-run` first.
- [ ] Upgrade from the previous release, if one exists, on a disposable host.
- [ ] No secrets, personal IPs or private-fleet screenshots in the archive,
      repo or release notes.

## Still missing

| Gap | Tracking |
|---|---|
| A first keyless Sigstore-signed release actually run and verified | G1.7 / G4.4 |
| A published SBOM for a real release | G4.4 |
| Reproducible-build notes (pinned cross image, toolchain, dates) | G4.4 |
| A Docker image | G1.7 |
| A tested upgrade path from a previous release | G1.4 |
| An actual first release | G1.7 / DIST-01 |

Until those exist, the release workflow is build automation with checksums,
keyless-signing code and an SBOM step — not yet a verified trust story.
