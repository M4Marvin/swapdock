export interface App {
  name: string
  kind: string
  strategy: string
  hostnames: string[]
  listen: string[]
  front_port: number
  slot: number
  live_port: number | null
  old_port: number | null
  writes_state: boolean
  image_repo: string | null
  registry: string | null
  build_host: string | null
  release: string | null
  old_release: string | null
  health_url: string | null
  compose_dir: string | null
  compose_svc: string | null
  env_name: string | null
  git_remote: string | null
  branch: string | null
  repo: string | null
  root: string | null
  build_repo: string | null
}

export interface Problem {
  severity: 'warning' | 'error'
  code: string
  message: string
  app: string | null
}

export interface AppEntry {
  app: App
  problems: Problem[]
}

export interface RunSummary {
  run_id: string
  started: string
  status: string
  steps: number
  non_ok: number
  /** App the run was scoped to, or null for a registry-wide run. */
  app?: string | null
}

export interface TraceEvent {
  event: 'run_start' | 'step' | 'run_end'
  run_id: string
  ts: string
  /** App this event belongs to, when the run was scoped to one. */
  app?: string | null
  mode?: string
  status?: string
  seq?: number
  step?: string
  argv?: string[]
  exit_code?: number | null
  duration_ms?: number | null
  detail?: unknown
  error?: string | null
  steps?: number
}

/** Live health probe verdict for an app, from GET /api/apps/{name}/status. */
export interface AppStatus {
  /** True when the live port answered 2xx. */
  up?: boolean
  /** HTTP status code, or null when the app did not answer at all. */
  status?: number | null
  /** Port probed, or null when the app has no live port. */
  live_port?: number | null
}

/** Release that a deploy would land on, from GET /api/apps/{name}/latest. */
export interface AppLatest {
  release?: string | null
  /** Which ref answered: "origin/main" or "HEAD". */
  source?: string
}

/** Source state of an app's checkout, from GET /build/apps/{name}/git. */
export interface AppGit {
  repo: string
  branch: string
  remote_sha: string | null
  head_sha: string | null
  dirty: boolean | null
  error: string | null
}

/** One local image tag, from GET /build/apps/{name}/images. */
export interface AppImage {
  ref: string
  tag: string
  size: string | null
  created: string | null
}

/** What one upstream served during a verify window. */
export interface VerifyUpstream {
  first_ms: number
  last_ms: number
  requests: number
  errors_5xx: number
}

/** The instant traffic moved from one upstream to another. */
export interface VerifyFlip {
  at_ms: number
  from: string
  to: string
}

/** Access-log verdict from GET /api/apps/{name}/verify. */
export interface VerifyReport {
  /** Lines read, including ones for other hosts. */
  lines: number
  /** Lines that did not parse. */
  malformed: number
  /** Requests to this app's hostnames since `since`. */
  requests: number
  errors_5xx: number
  by_status: Record<string, number>
  upstreams: Record<string, VerifyUpstream>
  flip: VerifyFlip | null
}

/** Where a run had got to, from GET /api/runs/{id}/resume. */
export interface ResumeInfo {
  run_id: string
  last_seq: number
  last_step: string
  last_status: string
  non_ok_steps: number
  /** Terminal status, when the run wrote its `run_end` record. */
  ended: string | null
}

/**
 * Normalizes a run or step status for display.
 *
 * The API serializes statuses as snake_case ("ok", "failed", "dry_run"), but
 * `/resume` renders them with Rust's `Debug` spelling ("Ok", "DryRun"). Fold
 * both onto one label: capitalize the first letter, and spell out "dry run".
 */
export function formatStatus(status: string): string {
  const key = status.toLowerCase().replace(/[\s-]+/g, '_')
  if (key === 'dry_run' || key === 'dryrun') return 'dry run'
  return key.charAt(0).toUpperCase() + key.slice(1)
}

/** Coarse outcome, folding the API's statuses onto one small set. */
export type StatusOutcome = 'ok' | 'failed' | 'dry' | 'interrupted' | 'unknown'

/**
 * Classifies a run or step status.
 *
 * The API serializes statuses as snake_case (`ok`, `failed`, `dry_run`,
 * `interrupted`) but `/resume` renders them with Rust's `Debug` spelling
 * (`Ok`, `DryRun`). Match a closed set rather than a substring, so `revoked`
 * or `not ok` can never read as success. Step-only statuses (`error`,
 * `timeout`) fold onto `failed`.
 */
export function classifyStatus(status: string | null | undefined): StatusOutcome {
  if (status == null) return 'unknown'
  switch (status.trim().toLowerCase().replace(/[\s-]+/g, '_')) {
    case 'ok':
    case 'success':
    case 'succeeded':
      return 'ok'
    case 'failed':
    case 'failure':
    case 'error':
    case 'timeout':
      return 'failed'
    case 'dry_run':
    case 'dryrun':
      return 'dry'
    case 'interrupted':
      return 'interrupted'
    default:
      return 'unknown'
  }
}

// ---------------------------------------------------------------------------
// configuration
// ---------------------------------------------------------------------------

/**
 * Served as a static `/config.json` next to the SPA.
 *
 * Same-origin defaults work both through the Vite dev proxy and when the built
 * app is served from either box. To split the build server from the deploy
 * server, point one base at the other box, e.g.
 *   { "buildApi": "/api", "deployApi": "http://100.80.96.4:8088/api" }
 */
export interface WebConfig {
  /** Base for the build-server surface (git, images, builds, transfers). */
  buildApi: string
  /** Base for the deploy surface (registry, deploys, runs, verify). */
  deployApi: string
  /** Default `target` for a transfer (`ssh`/`taildrop` destination). */
  transferTarget: string
}

const DEFAULT_CONFIG: WebConfig = {
  buildApi: '/api',
  deployApi: '/api',
  transferTarget: 'hetzner',
}

let configPromise: Promise<WebConfig> | null = null

/**
 * Fetches `/config.json` once and caches the result.
 *
 * A missing or malformed file falls back to same-origin defaults rather than
 * failing the app: the defaults are correct in the common single-box case.
 */
export function cfg(): Promise<WebConfig> {
  if (!configPromise) {
    configPromise = fetch('/config.json', { cache: 'no-cache' })
      .then(async (r) =>
        r.ok ? ((await r.json()) as Partial<WebConfig>) : {},
      )
      .catch((): Partial<WebConfig> => ({}))
      .then((c) => ({ ...DEFAULT_CONFIG, ...c }))
  }
  return configPromise
}

/** Joins a configured base with a relative path, collapsing the slash. */
function joinBase(base: string, path: string): string {
  return `${base.replace(/\/+$/, '')}/${path.replace(/^\/+/, '')}`
}

/** Resolves a build-server path under the configured `buildApi` base. */
export async function build(path: string): Promise<string> {
  return joinBase((await cfg()).buildApi, path)
}

/** Resolves a deploy-server path under the configured `deployApi` base. */
export async function deploy(path: string): Promise<string> {
  return joinBase((await cfg()).deployApi, path)
}

// ---------------------------------------------------------------------------
// run streaming
// ---------------------------------------------------------------------------

/** A live tail of one run's trace. */
export interface RunStream {
  close(): void
}

export interface RunStreamHandlers {
  /** Each trace event: run_start, step, or run_end. */
  onEvent: (e: TraceEvent) => void
  /** A server-sent named `error` event, or a terminal transport failure. */
  onError?: (message: string) => void
  /** Connection phase changes, for a "reconnecting…" indicator. */
  onStatus?: (status: 'open' | 'reconnecting') => void
  /** Called once when the stream is over: the run_end status, the server's
   * `done` event, a server error, or a give-up after transport retries. */
  onEnd?: (status: string | null) => void
}

/** Which configured base a run's events live on. Defaults to the deploy box. */
export type EventsApi = 'deploy' | 'build'

/** Transport errors are retried this many times before giving up. */
const MAX_RETRIES = 3
/** Fixed delay between transport reconnect attempts. */
const RETRY_DELAY_MS = 1000

/**
 * Opens the SSE event stream for a run under `<base>/events`.
 *
 * The events endpoint is resolved through the configured base first, because a
 * build-server run's log lives on the build box and a deploy run's on the deploy
 * box. The server sends the trace as unnamed `message` events, an explicit
 * `done` event when the run writes its `run_end` record, and a named `error`
 * event when it cannot read the log. A transport failure is retried up to three
 * times; if it keeps failing, `onError` reports it and the stream ends. `onEnd`
 * fires exactly once — with the run_end status when known, otherwise null.
 */
export function openRunEvents(
  id: string,
  handlers: RunStreamHandlers,
  apiBase: EventsApi = 'deploy',
): RunStream {
  const { onEvent, onError, onStatus, onEnd } = handlers
  let closed = false
  let ended = false
  let serverError = false
  let retries = 0
  let es: EventSource | null = null
  let retryTimer: ReturnType<typeof setTimeout> | null = null
  let eventsUrl: string | null = null

  const finish = (status: string | null) => {
    if (ended) return
    ended = true
    closed = true
    if (retryTimer) {
      clearTimeout(retryTimer)
      retryTimer = null
    }
    es?.close()
    es = null
    onEnd?.(status)
  }

  const connect = () => {
    if (closed || eventsUrl == null) return
    es = new EventSource(eventsUrl)

    es.onopen = () => {
      retries = 0
      onStatus?.('open')
    }

    // A server-sent named "error" event is a MessageEvent carrying a message
    // in `data`; a transport failure is a bare Event with no data. This
    // listener only handles the former, leaving transport to es.onerror.
    es.addEventListener('error', (ev) => {
      const data = (ev as MessageEvent).data
      if (typeof data === 'string' && data.length > 0) {
        serverError = true
        onError?.(data)
        finish(null)
      }
    })

    es.addEventListener('done', () => finish(null))

    es.onmessage = (msg) => {
      let ev: TraceEvent
      try {
        ev = JSON.parse(msg.data) as TraceEvent
      } catch {
        return
      }
      onEvent(ev)
      if (ev.event === 'run_end') finish(ev.status ?? 'ended')
    }

    es.onerror = () => {
      if (closed || ended || serverError) return
      // A named error leaves the connection OPEN; a transport error does not.
      if (es?.readyState === EventSource.OPEN) return
      es?.close()
      es = null
      if (retries >= MAX_RETRIES) {
        onError?.('connection lost')
        finish(null)
        return
      }
      retries += 1
      onStatus?.('reconnecting')
      retryTimer = setTimeout(connect, RETRY_DELAY_MS)
    }
  }

  // Resolve the events URL through the configured base before connecting; a
  // close() during resolution cancels the pending connection.
  const resolve = apiBase === 'build' ? build : deploy
  resolve(`events?run=${encodeURIComponent(id)}`).then(
    (url) => {
      if (closed) return
      eventsUrl = url
      connect()
    },
    () => {
      if (closed) return
      onError?.('could not resolve the events endpoint')
      finish(null)
    },
  )

  return {
    close() {
      closed = true
      if (retryTimer) {
        clearTimeout(retryTimer)
        retryTimer = null
      }
      es?.close()
      es = null
    },
  }
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

/** An HTTP response that was not ok, carrying the server's status and message. */
export class ApiError extends Error {
  readonly status: number

  constructor(status: number, message: string) {
    super(message)
    this.name = 'ApiError'
    this.status = status
  }
}

async function get<T>(url: string, signal?: AbortSignal): Promise<T> {
  const res = await fetch(url, { signal })
  const json = await res.json().catch(() => ({}))
  if (!res.ok) {
    throw new ApiError(
      res.status,
      (json as { error?: string }).error ?? `${res.status} ${res.statusText}`,
    )
  }
  return json as T
}

async function post<T>(
  url: string,
  body?: unknown,
  signal?: AbortSignal,
): Promise<T> {
  const res = await fetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal,
  })
  const json = await res.json().catch(() => ({}))
  if (!res.ok) {
    throw new ApiError(
      res.status,
      (json as { error?: string }).error ?? `${res.status}`,
    )
  }
  return json as T
}

async function put<T>(
  url: string,
  body?: unknown,
  signal?: AbortSignal,
): Promise<T> {
  const res = await fetch(url, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal,
  })
  const json = await res.json().catch(() => ({}))
  if (!res.ok) {
    const problems = (json as { problems?: { code?: string }[] }).problems
    const suffix =
      problems && problems.length > 0
        ? `: ${problems.map((p) => p.code ?? '?').join(', ')}`
        : ''
    throw new ApiError(
      res.status,
      ((json as { error?: string }).error ?? `${res.status}`) + suffix,
    )
  }
  return json as T
}

const enc = encodeURIComponent

export interface UpdateAppResult {
  ok: boolean
  backup: string
}

export const api = {
  health: async (signal?: AbortSignal) =>
    get<{ status: string }>(await deploy('health'), signal),
  apps: async (signal?: AbortSignal) =>
    get<AppEntry[]>(await deploy('apps'), signal),
  app: async (name: string, signal?: AbortSignal) =>
    get<AppEntry>(await deploy(`apps/${enc(name)}`), signal),
  validate: async (signal?: AbortSignal) => {
    const res = await fetch(await deploy('validate'), { method: 'POST', signal })
    return { status: res.status, body: await res.json() }
  },
  render: async (app?: string, signal?: AbortSignal) =>
    (
      await fetch(
        await deploy(`render${app ? `?app=${enc(app)}` : ''}`),
        { signal },
      )
    ).text(),
  deploy: async (name: string, release?: string, signal?: AbortSignal) =>
    post<{ run_id: string }>(
      await deploy(`apps/${enc(name)}/deploys`),
      { release },
      signal,
    ),
  rollback: async (name: string, signal?: AbortSignal) =>
    post<{ run_id: string }>(
      await deploy(`apps/${enc(name)}/rollback`),
      {},
      signal,
    ),
  // Build actions go to the build server: this is the same handler as the
  // legacy `/apps/{name}/build`, surfaced at the build-server path.
  build: async (name: string, release?: string, signal?: AbortSignal) =>
    post<{ run_id: string }>(
      await build(`build/apps/${enc(name)}/builds`),
      { release },
      signal,
    ),
  sync: async (name: string, signal?: AbortSignal) =>
    post<{ run_id: string }>(
      await deploy(`apps/${enc(name)}/sync`),
      {},
      signal,
    ),
  verify: async (name: string, since: string, signal?: AbortSignal) =>
    get<VerifyReport>(
      await deploy(
        `apps/${enc(name)}/verify?since=${enc(since)}`,
      ),
      signal,
    ),
  runs: async (limit = 50, signal?: AbortSignal) =>
    get<RunSummary[]>(await deploy(`runs?limit=${limit}`), signal),
  run: async (id: string, signal?: AbortSignal) =>
    get<TraceEvent[]>(await deploy(`runs/${enc(id)}`), signal),
  resume: async (id: string, signal?: AbortSignal) =>
    get<ResumeInfo>(await deploy(`runs/${enc(id)}/resume`), signal),
  appStatus: async (name: string, signal?: AbortSignal) =>
    get<AppStatus>(await deploy(`apps/${enc(name)}/status`), signal),
  appLatest: async (name: string, signal?: AbortSignal) =>
    get<AppLatest>(await deploy(`apps/${enc(name)}/latest`), signal),
  // Build-server reads.
  appGit: async (name: string, signal?: AbortSignal) =>
    get<AppGit>(await build(`build/apps/${enc(name)}/git`), signal),
  appImages: async (name: string, signal?: AbortSignal) =>
    get<{ images: AppImage[] }>(
      await build(`build/apps/${enc(name)}/images`),
      signal,
    ),
  transfer: async (
    name: string,
    release: string,
    target: string,
    signal?: AbortSignal,
  ) =>
    post<{ run_id: string }>(
      await build(`build/apps/${enc(name)}/transfers`),
      { release, target },
      signal,
    ),
  updateApp: async (name: string, app: App, signal?: AbortSignal) =>
    put<UpdateAppResult>(
      await deploy(`apps/${enc(name)}`),
      app,
      signal,
    ),
}
