/**
 * Central registry of TanStack Query keys.
 *
 * Every cached response gets its key from here so invalidation after a
 * mutation can never drift from the key a query was stored under. Keys are
 * arrays; the leading element names the kind of data, the rest narrow it.
 */
export const queryKeys = {
  /** The whole registry: `GET /api/apps`. */
  apps: ['apps'] as const,
  /** One app entry (app + problems): `GET /api/apps/{name}`. */
  app: (name: string) => ['app', name] as const,
  /** Live health probe: `GET /api/apps/{name}/status`. */
  appStatus: (name: string) => ['app-status', name] as const,
  /** Registry-wide validation summary: `POST /api/validate`. */
  validate: ['validate'] as const,
  /** Recent runs: `GET /api/runs`. */
  runs: ['runs'] as const,
  /** One run's recorded trace / resume state: `GET /api/runs/{id}`. */
  run: (id: string) => ['run', id] as const,
  /** Access-log verdict for a window: `GET /api/apps/{name}/verify`. */
  verify: (name: string, since: string) => ['verify', name, since] as const,
  /** What an app's upstream ref resolves to: `GET /api/apps/{name}/latest`. */
  latest: (name: string) => ['latest', name] as const,
  /** Source state of an app's checkout: `GET /build/apps/{name}/git`. */
  git: (name: string) => ['git', name] as const,
  /** Local image tags: `GET /build/apps/{name}/images`. */
  images: (name: string) => ['images', name] as const,
  /** Runtime web config: `/config.json`. */
  config: ['config'] as const,
} as const
