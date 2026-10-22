# swapdock web

The operator console for **swapdock**: a fleet of blue/green Docker apps behind
one nginx front door.

React 19 + TypeScript + Vite, TanStack Router for file routes, TanStack Query
for cache, TanStack Form for the registry editor, and shadcn/Base UI primitives
re-skinned around one small design system.

## The shape of the UI

```
sidebar                                 top bar
  fleet status + app list               breadcrumb · search · refresh · theme
────────────────────────────────────────────────────────────────────────────
fleet overview                          app detail
  healthy / attention / runs / latest     Deployment | Operations | Configuration
  app list · host checks · activity       topology · traffic · source · runs
────────────────────────────────────────────────────────────────────────────
runs ledger                             doctor
  filters · day groups · live detail      severity summary · grouped findings
```

Every view is a job, not a dump of fields: **know the fleet**, **act on one
app**, **audit a run**, **fix a finding**. The command palette (`⌘K` / `Ctrl K`)
is the shortest path between them.

### Routes

| Route | Purpose |
|---|---|
| `/` | Fleet overview: health, attention, registry filters, host checks, recent runs |
| `/apps/$appName?tab=overview` | Blue/green topology, nginx traffic window, source/images, recent runs |
| `/apps/$appName?tab=pipeline` | Mode-aware Deployment: source build, image versions, first-run takeover, static release |
| `/apps/$appName?tab=operations` | Doctor, Inspect, Probe, Backup, Prune, takeover, generated nginx |
| `/apps/$appName?tab=configuration` | Registry editor, grouped fields, advanced/danger fields, draft TOML |
| `/runs?app=…` | Run ledger, filterable by app, status and text, grouped by day |
| `/runs/$runId` | Live/finished run with a step timeline and expandable argv/detail |
| `/doctor` | Estate-wide findings, grouped by app, optional public-URL probes |

## Design system

- **Source of truth:** `src/index.css`. Tokens are OKLCH/hex values mapped into
  Tailwind v4's `@theme`; there is no separate colour file to drift.
- **Surfaces:** near-black `#0a0a0b` and warm-white `#f8f8f9`, one-pixel
  borders, a 12px card radius, and a single inset highlight on dark cards.
- **Type:** Geist Sans for UI, Geist Mono for identifiers and data. The mono
  face is vendored in `public/fonts/` so the build has no external font request.
- **Colour carries meaning:** success, warning, danger and info are reserved for
  state. The accent (`#4b5cf5` light, `#7b87ff` dark) is for the one action that
  matters and for focus.
- **Motion:** 120–220ms, custom `cubic-bezier(0.23, 1, 0.32, 1)` easing, press
  feedback on buttons, no entrance from `scale(0)`, and one shared 15s clock for
  relative timestamps. `prefers-reduced-motion` collapses movement.
- **Keyboard:** `⌘K`/`Ctrl K` palette, arrow/enter navigation, `Escape` closes;
  tab switches reset scroll; back/forward restores scroll through
  `ScrollRestoration`.
- **Themes:** system, light and dark, applied before first paint by the boot
  script in `index.html` and persisted under `swapdock-theme`.

## Deployment origins and lifecycle

The old UI assumed every container was built from a git checkout. That is not
the estate: some apps are source builds, some pull a prebuilt image, and some
are still running as legacy containers that nginx has never owned. The
Deployment tab now starts from **where a version comes from**:

- **Source** — a `repo` or `git_remote` exists. The real build pipeline is shown
  and versions are resolved from the checkout.
- **Image** — an `image_repo` exists but no source checkout. There is no build
  stage. Versions are local image tags, and a deploy only needs one of those.
- **Compose** — no source and no image repository. The tab says so and points at
  Configuration; it does not pretend a build can run.
- **Static** — files served from a root symlink. There is no container build or
  health gate, so the tab shows the release/root facts only.

Lifecycle is separate from origin. A `replace` app with no `live_port` cannot
be deployed normally yet: the backend needs a live slot to replace. For those
apps the primary action is **Take over app**, not "resolve a version". Adoption
takes over the container already publishing the front port.

### First-run adoption

- The takeover dialog detects the running container and its image.
- It generates a **snapshot release** automatically, so no commit hash is
  required. swapdock tags the running image with that label and records it as
  the adopted release. The label is a snapshot identity, not a claim that the
  hash is a commit.
- A specific commit remains available under an advanced disclosure.
- If the app has no image repository, the dialog can save the repository
  detected from the running image, or link to Configuration.
- The Operations tab's takeover card opens the same dialog; Overview and the
  Deployment tab lead with it until adoption completes.
- After adoption the app is managed: source apps get the build pipeline, image
  apps get the image-version deployment view.

Image apps update their upstream tag outside swapdock; once a version is in the
local Docker daemon, it appears in the deploy picker. This UI does not pretend
to pull or retag arbitrary image tags.

## Version-first deploys (source apps)

A commit hash is an implementation detail, not something an operator should
have to know. For a source-built app, the UI resolves one from the app's source
state and speaks in versions:

- **Latest from main** is the default. If its image is not built yet, the
  primary action becomes **Build & deploy**: the flow builds the image, sends
  it when the build and deploy boxes are split, then runs the normal health
  gate. The dialog follows every stage over SSE and falls back to `/resume`
  polling for long builds.
- **Newest built version** offers the latest image that already exists, by
  build time. **Current live version** is there for a deliberate redeploy.
- The SHA is only visible under **Technical details**, and an optional
  **Advanced** entry allows a specific commit for the rare case that needs one.
- The Deployment tab's target is a friendly **Latest from main** control;
  editing the raw reference is an advanced disclosure, not the default path.

This keeps the immutable commit chain the deploy tool relies on without making
the operator translate a hash before every release.

## Data flow

- Every cache key lives in `src/lib/query-keys.ts`.
- Polling is deliberately quiet: apps every 60s, runs every 30s, app health every
  20s. Doctor results are reused between the overview and the Doctor page when
  the public-probe toggle matches. Latest-release and image queries are cached
  for a minute.
- Build/transfer/deploy runs and the run detail page tail
  `GET /api/events?run=<id>&token=<session token>` over SSE. When a long step writes nothing (a Docker
  build), the client falls back to `GET /api/runs/{id}/resume` polling.
- A run that has only written `run_start` reports its mode (`live`) as its
  status. `classifyStatus` folds that onto **Running**, so in-progress work never
  reads as an unknown outcome.
- The shell reads `GET /api/health` for the connection indicator; registry
  failures show a retry banner without taking the shell down.
- Static apps have no live port or container health probe. They are shown as
  **Static**, served from `root`, and traffic verification is enabled when a
  hostname exists.
- Fleet health distinguishes **Unmanaged** from idle: an app that still runs
  outside swapdock is actionable, so it gets its own filter and takeover path
  rather than a misleading green/neutral "idle" state.

## API access token

`swapdock serve` requires a token (`SWAPDOCK_API_TOKEN`) unless it was started
with `--allow-unauthenticated`. The console opens on an **Access token** gate
when this tab has no session, and returns to it on any 401 or after **Sign out**
in the sidebar.

- The token is exchanged once at `POST /api/login`; the API sets an HttpOnly,
  SameSite=Strict session cookie and returns a per-session CSRF token.
- The token is not stored after a successful login. The CSRF token is kept in
  `sessionStorage`, never `localStorage`.
- Every state-changing request sends `X-CSRF-Token`; bearer clients use
  `Authorization: Bearer <token>` instead.
- SSE (`/api/events?run=…`) uses the session cookie; the older `?token=` form
  is accepted only on that route.
- Against an older API without `/api/login`, the gate falls back to the bearer
  token flow.
- Sessions are in memory, expire after 12 hours, and are lost on restart.

## Configuration

Runtime-only settings live in `public/config.json`, fetched once by
`src/api.ts`:

```json
{ "buildApi": "/api", "deployApi": "/api", "transferTarget": "" }
```

`buildApi` is the base for the build-server surface (git, images, builds,
transfers); `deployApi` is the base for everything else (registry, deploys,
rollback, runs, status, latest, verify). Same-origin defaults work through the
Vite dev proxy and when the app is served from either box. An empty
`transferTarget` is valid on a single host; a split install must set it or the
transfer stage refuses to run. If `/config.json` is missing or malformed the
shell warns and falls back to same-origin only.

### Split-box deploys

Point one base at the other box — no rebuild required:

```json
{
  "buildApi": "/api",
  "deployApi": "https://deploy.example.com/api",
  "transferTarget": "deploy-host"
}
```

Run events are opened on whichever base owns the run: build and transfer runs
stream from `buildApi`, deploy runs from `deployApi`.

## Development

```sh
pnpm install
pnpm dev       # Vite dev server; /api proxies to SWAPDOCK_API or the default box
pnpm exec tsc -b
pnpm lint
pnpm build
```

The dev proxy target can be changed without editing config:

```sh
SWAPDOCK_API=http://127.0.0.1:8088 pnpm dev
```

## Deploying

`./deploy.sh [release-id]` builds the app, ships `dist/` to the SSH host
under `/srv/www/swapdock-ui/releases/<id>`, flips the `current` symlink
atomically, and checks the URL. Both `SWAPDOCK_WEB_HOST` and
`SWAPDOCK_WEB_URL` are **required** — the script has no personal defaults:

```sh
SWAPDOCK_WEB_HOST=deploy@example.com \
SWAPDOCK_WEB_URL=https://console.example.com/ ./deploy.sh
```

nginx needs no change: the console's registry entry serves the symlink as its
`root`. The default release id is the short commit (plus `-dirty<timestamp>`
when the tree is dirty).

## Backend gaps the UI works around

- **Deploy-side image check:** there is no deploy-server images endpoint, so in
  a split-box setup the Deploy stage unlocks after the transfer run ends `ok`
  (the server has already verified the image on the target).
- **Static deploys:** the API does not expose a static-app health probe, so the
  UI treats `root` as served and does not invent a health status.
- **Run summaries:** `GET /api/runs` does not report a terminal status for an
  in-progress run, so the client reads `live` as **Running** (see above).
