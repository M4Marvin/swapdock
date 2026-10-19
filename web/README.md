# swapdock web

React + TypeScript + Vite SPA for the swapdock API, with TanStack Router
file-based routes under `src/routes/` and shadcn/ui components in
`src/components/ui/`.

## Configuration

Runtime-only settings live in the static `public/config.json`, fetched once by
`src/api.ts`:

```json
{ "buildApi": "/api", "deployApi": "/api", "transferTarget": "hetzner" }
```

`buildApi` is the base for the build-server surface (git, images, builds,
transfers); `deployApi` is the base for everything else (registry, deploys,
rollback, runs, status, latest, verify). Same-origin defaults work through the
Vite dev proxy and when the app is served from either box.

### Split-box deploys

To run the SPA against a build server and a deploy server on different
machines, edit `config.json` and point one base at the other box — no rebuild
required:

```json
{
  "buildApi": "/api",
  "deployApi": "http://100.80.96.4:8088/api",
  "transferTarget": "hetzner"
}
```

Run events (`EventSource`) are opened on whichever base owns the run: build and
transfer runs stream from `buildApi`, deploy runs from `deployApi`.

## Pipeline

The app page shows a Build → Transfer → Deploy pipeline. Each stage POSTs its
run and tails it over SSE, revealing the next stage when its predecessor ends
`ok`. A transfer is also offered when the target image is already present in
the build server's image list.

When `buildApi` and `deployApi` resolve to the same origin, the build already
puts the image on the deploy host, so the Transfer stage is omitted and Deploy
unlocks after a successful Build (or when the image already exists).

## Operations

The app page has an **Operations** card driven by the `0.2.0` API:

- **Doctor** — host/compose checks for this app; shows the findings inline.
- **Backup** — runs the registry's `[backup]` command as a deploy run.
- **Probe** — HTTP status from `public_url` or an explicit URL.
- **Prune** — removes old local image tags, keeping the release chain.
- **Inspect** — registry entry, live verdict, image, health and actual mounts.
- **Adopt…** — shown only when `live_port` is unset; turns a legacy
  front-port container into a swapdock-managed app.

The **Doctor** page in the header runs the same checks across the whole
estate, with an optional public-URL probe. The registry editor now edits
`compose_override`, `public_url` and the `[backup]` block
(`command`, `dir`, `before_deploy`).

## Commands

```sh
pnpm install
pnpm dev       # Vite dev server, proxies /api to the build API
pnpm exec tsc -b
pnpm build
```

## Deploying

`./deploy.sh [release-id]` builds the app, ships `dist/` to
`hetzner:/srv/www/swapdock-ui/releases/<id>`, flips the `current` symlink
atomically, and checks the URL. nginx needs no change: the `swapdock-ui`
registry entry serves the symlink as its `root`.

The default release id is the short commit (plus `-dirty<timestamp>` when the
tree is dirty). Override the host and URL with `SWAPDOCK_WEB_HOST` and
`SWAPDOCK_WEB_URL`.

## Backend gaps

- **Registry editing** has no `PUT /api/apps/{name}` endpoint. The registry
  editor validates the draft client-side and offers Revert (re-read) and
  Copy as TOML (paste into the app's `.toml`); it never writes to the server.
- **Deploy-side image check**: there is no deploy-server images endpoint, so
  the Deploy stage is enabled when the transfer run ends `ok` (the server has
  already verified the image with `docker inspect` on the target).
