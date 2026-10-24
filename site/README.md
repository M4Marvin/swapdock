# swapdock site

The public marketing site for **swapdock**. A static, multipage site — every
route is prerendered to its own HTML file at build time, not an SPA shell.

- **TanStack Start** (TanStack Router + Vite) with `prerender` enabled
- **Tailwind CSS v4** with the console's design tokens, extended for editorial scale
- **shadcn/ui** installed from the registry (`base-nova` style, Base UI primitives)
- **Geist Sans** (fontsource) and **Geist Mono** (vendored in `public/fonts`)

## Develop

```sh
pnpm install
pnpm dev        # http://localhost:3001
```

## Build and check

```sh
pnpm build      # vite build + typecheck + sitemap/robots
pnpm check      # asserts every route has HTML, title, description, h1, canonical
```

The static output is `dist/client/`. Every route has its own `index.html`
(for example `dist/client/path/index.html`). Deploy it behind nginx with
`try_files $uri $uri/ =404;` and `error_page 404 /404/index.html;`.

## Routes

| Route | Page |
|---|---|
| `/` | Home: outcome, problem, path preview, features, proof |
| `/why` | The single-box problem and what changes |
| `/path` | The seven-stage route from `compose up` to boring deploys |
| `/how-it-works` | The 20-step loop, strategies, health gate, failure matrix |
| `/features` | Feature index |
| `/features/zero-downtime` | Blue/green cutover |
| `/features/stateful-apps` | Replace, SQLite, backups |
| `/features/recovery` | Journal, reconcile, locks |
| `/features/operations` | verify, monitor, prune, doctor |
| `/docs` | Install, quickstart, docs hub |
| `/security` | Privilege model, auth, limits |
| `/faq` | Objections, including production readiness |
| `/404` | Static not-found page |

## Configuration

`VITE_SITE_URL` sets the canonical origin for metadata, Open Graph, and the
sitemap. It defaults to `https://swapdock.dev`; set it when the real domain is
known.

```sh
VITE_SITE_URL=https://example.com pnpm build
```

## Design notes

- Tokens live in `src/styles.css` (`@theme inline` + light/dark variables).
- Dark is the default; the theme toggle persists under `swapdock-site-theme`.
- Content lives in typed modules under `src/content/`, so copy is reviewed like code.
- No analytics, no third-party scripts, no external fonts.

## Deploy on this host

The public site is a **static app in swapdock**, served by nginx and fronted by
the Cloudflare Tunnel:

| | |
|---|---|
| Hostname | `swapdock.m4marvin.com` |
| Front port | `8016` (loopback, nginx) |
| Slot | `10` |
| Root (live) | `/srv/www/swapdock-ui/swapdock-site/current` |
| Releases | `/srv/www/swapdock-site/releases/<release>` |
| Registry | `/srv/swapdock/apps/swapdock-site.toml` |
| Tunnel | `swapdock-site` → `http://localhost:8016`, user service |

The live root is under the console directory because `/srv/www` is root-owned
and only the swapdock binary has passwordless sudo on this host. The script
accepts a root override, so a normal redeploy does not need a password:

```sh
SITE_ROOT=/srv/www/swapdock-ui/swapdock-site ./deploy.sh --dry-run
SITE_ROOT=/srv/www/swapdock-ui/swapdock-site ./deploy.sh
```

Running `./deploy.sh` without the override targets `/srv/www/swapdock-site` and
asks for sudo once to create that root; that is the migration path to the
clean layout.

`deploy.sh` does the whole sequence:

1. Builds with `VITE_SITE_URL=https://<hostname>` and runs the build checks.
2. Publishes the release under `<root>/releases/<release>/`.
3. Adds the hostname to the dedicated tunnel ingress (backed up first) and validates it.
4. Installs the registry entry and validates the whole estate.
5. Runs `swapdock up swapdock-site --release <release> --force`: it verifies the
   release directory, flips the `current` symlink, renders and applies nginx,
   probes the front door, then commits the registry.
6. Restarts the user tunnel connector, creates the proxied CNAME, and verifies
   local + public.

Requirements: `pnpm`, `swapdock` (passwordless sudo), `cloudflared`, `rsync`,
`curl`, and `git`. The live root needs no password; creating a different root
under `/srv/www` does.

Override the defaults with environment variables:

```sh
SITE_HOSTNAME=swapdock.example.com \
SITE_FRONT_PORT=8016 \
SITE_SLOT=10 \
./deploy.sh
```

A different hostname needs a matching DNS record; the script creates a proxied
CNAME to the tunnel with `cloudflared tunnel route dns`. Use `--skip-dns` if you
manage DNS elsewhere.

Rollback and status:

```sh
sudo swapdock --registry /srv/swapdock/apps rollback swapdock-site
swapdock --registry /srv/swapdock/apps status swapdock-site
```

Notes:

- swapdock owns the cutover. The release files are produced outside it, which is
  what the static strategy expects.
- The generated static block uses `try_files $uri $uri/ =404`. `/404/` is
  deployed, but nginx serves its default 404 for unknown paths until the
  renderer grows a per-app error page.
- Internal links and canonical URLs use trailing slashes to match nginx's
  directory behaviour.
- The live edge is a dedicated tunnel (`swapdock-site`) run as the user service
  `swapdock-site-tunnel.service`. It does not touch the system `cloudflared`
  service or the main `vps` tunnel. Restarting it only affects this hostname.
- A normal content deploy does not need the tunnel restart; the connector only
  restarts when the ingress changes.
- The registry template is `deploy/swapdock-site.toml`; the script rewrites the
  hostname, front port, and slot from the environment before installing it.
