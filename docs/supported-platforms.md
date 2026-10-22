# Supported platforms

The v1 target is deliberately narrow. It follows [LAUNCH.md](../LAUNCH.md) §8.
“Supported” here means the design target; **no release has been published yet**,
so nothing has been validated across the whole matrix. The release workflow
builds for x86_64 and aarch64 musl when a tag is pushed (see
[release.md](release.md)), but treat that as build coverage, not a support
promise.

## v1 matrix

| Dimension | Supported | Notes |
|---|---|---|
| OS | Ubuntu 24.04 LTS, Ubuntu 26.04 LTS, Debian 12, Debian 13 | Other systemd Linux distributions may work; they are untested. |
| Architecture | x86_64, aarch64 | Static musl release binaries are built for both. |
| Init/systemd | systemd | The example unit is optional; the CLI does not require systemd. |
| Docker | Docker Engine 27+ | With the Compose v2 plugin (`docker compose`). |
| nginx | 1.24+ | `nginx -t`, atomic file replace and `nginx -s reload` are used. |
| Filesystem | Local ext4/xfs | No network filesystems for `/srv/swapdock` or nginx. |
| Network | One host; a domain and DNS control | Cloudflare Tunnel optional; TLS is not automatic. |
| Console browsers | Last two versions of Chrome, Firefox, Safari, Edge (target) | The console is not distributable yet (single-admin session, no multi-user login); this is a target, not a tested claim. |
| Accessibility target | WCAG 2.1 AA | Planned for Gate 4; not verified. |

## Explicitly unsupported in v1

- **Windows or macOS hosts.** No plans to support them.
- **Rootless Docker.** Not tested; the privileged operations assume the normal
  Docker daemon and root.
- **Multi-host, Kubernetes, Swarm or a control plane.** Single host only.
- **Non-systemd init.** No tested service integration.
- **Shared/tenant hosts.** One admin, one host; no RBAC or user isolation.
- **NFS/CIFS state directories.** Atomic rename/fsync assumptions do not hold.
- **Static app deploys.** Gated by DEP-04/G2.4; the tree has a static path, but
  it is not supported until the gate is green, independent of the host.
- **Docker Desktop / Podman.** Not tested.

## What `doctor` does and does not prove

`swapdock doctor` checks the host side: compose paths, bind mounts, ports,
images, healthchecks and optional public URLs. It helps catch a host that
cannot deploy, but:

- it is **not** a supported-platform gate today; G4.8 plans an explicit
  unsupported-host failure;
- it does not prove TLS, DNS, tunnel or firewall correctness;
- it does not replace the deploy P0 work in Gate 2.

The installer refuses to run without `nginx`, `docker` and `visudo`, but it does
not check the OS version, Docker version or nginx version.

## Compose file names

The deploy path resolves `compose.yaml`, `compose.yml`, then
`docker-compose.yml` (G2.9). `doctor` in this tree still checks only
`docker-compose.yml`, so it can report a false `compose-missing` for the newer
names until that check is updated. Keep the app's real file in place; do not
rename a live app just to satisfy a preflight check.

## Getting a build for an unsupported host

There is no official artifact for unsupported platforms. You can try a source
build, but that is not a support claim, and the privileged deploy path is only
tested on the matrix above. Do not report it as a supported configuration.

## Planned

| Item | Tracking |
|---|---|
| Supported-platform matrix with `doctor` refusing unsupported hosts | G4.8 |
| Release binaries + checksums + signed artifacts | G1.7 / G4.4 |
| Documented, tested upgrade path | G1.4 |
| Console install and same-origin serving | G1.8 / DIST-03 |
