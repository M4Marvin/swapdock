# Non-root API and the apply broker

**Status: implemented as an opt-in install mode and exercised under systemd in
a disposable container.** The API ran as `swapdock`, the broker as root, and an
apply through the API produced a root-owned nginx config. It has not been run on
the live host. The API can run as the deploy user; a small root service
(`swapdock-broker`) owns the one operation that needs root for nginx: install a
rendered config and reload.

## What runs as what

| Process | User | Can write |
|---|---|---|
| `swapdock serve` (API) | deploy user | `/srv/swapdock/apps`, `overrides`, `backups`, `green`, `/var/log/swapdock` |
| `swapdock-broker` | root | `/etc/nginx/conf.d`, `/etc/nginx/snippets`, run log, allowlisted Docker commands |
| Docker | through the broker | containers/images |

The API does **not** join the `docker` group. `docker` commands are sent to the
broker, which runs a strict allowlist: ps/inspect/image/pull/tag/rmi/stop/rm,
compose pull/up/stop/rm/config, build `-t <image> .`, and push.

The API reaches the broker through `/run/swapdock/broker.sock`. The socket is
mode `0666` because the installer does not assume a shared group; requests carry
the same `SWAPDOCK_API_TOKEN` the API already requires. Keep `/run/swapdock`
root-owned and not world-writable.

## Install

```bash
sudo bash swapdock-dist/install-swapdock.sh --with-service --non-root swapdock-dist
```

This:

- adds the deploy user to the `adm` group (not `docker`) and installs **no**
  passwordless sudo entry; privileged work goes through the broker units;
- makes `/srv/swapdock/green` and `/srv/swapdock/locks` owned by that user;
- installs `swapdock-broker.service` (root) and `swapdock.service` (deploy user,
  `Requires=swapdock-broker.service`);
- generates `/etc/swapdock/api.env` once (`SWAPDOCK_API_TOKEN`);
- does **not** enable or start either unit.

```bash
sudo systemctl enable --now swapdock-broker.service
sudo systemctl enable --now swapdock.service
```

## Residual risk

Docker access is brokered: the API can issue only the allowlisted Docker
operations above. The installer generates separate API and broker tokens so the
console credential is not the broker credential. The broker is still root and `docker build`/`compose up` can
run a Dockerfile or mount host paths chosen by an operator-controlled registry;
that is a deliberate capability, not a sandbox. A compromised API can only use
the allowlist. G0.5 is materially closed but the broker remains a high-value
root process.

The broker's trace and the API's trace are both appended to the shared run log.
The broker never accepts an arbitrary command; it accepts one rendered nginx
config or one allowlisted Docker argv, caps the config at 2 MiB, and runs the
normal `apply` sequence (stage, backup, rename, `nginx -t`, reload, pid check).

## Verify before enabling

```bash
sudo swapdock --registry /srv/swapdock/apps validate
sudo nginx -t
sudo swapdock broker --help
sudo systemctl cat swapdock-broker.service
```

Run a disposable-host deploy and rollback before trusting the mode.
