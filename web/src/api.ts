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

/** Transport errors are retried this many times before giving up. */
const MAX_RETRIES = 3
/** Fixed delay between transport reconnect attempts. */
const RETRY_DELAY_MS = 1000

/**
 * Opens the SSE event stream for a run under `/api/events`.
 *
 * The server sends the trace as unnamed `message` events, an explicit `done`
 * event when the run writes its `run_end` record, and a named `error` event
 * when it cannot read the log. A transport failure is retried up to three
 * times; if it keeps failing, `onError` reports it and the stream ends. `onEnd`
 * fires exactly once — with the run_end status when known, otherwise null.
 */
export function openRunEvents(id: string, handlers: RunStreamHandlers): RunStream {
  const { onEvent, onError, onStatus, onEnd } = handlers
  let closed = false
  let ended = false
  let serverError = false
  let retries = 0
  let es: EventSource | null = null
  let retryTimer: ReturnType<typeof setTimeout> | null = null

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
    if (closed) return
    es = new EventSource(`/api/events?run=${encodeURIComponent(id)}`)

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

  connect()

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

export const api = {
  health: (signal?: AbortSignal) =>
    get<{ status: string }>('/api/health', signal),
  apps: (signal?: AbortSignal) => get<AppEntry[]>('/api/apps', signal),
  app: (name: string, signal?: AbortSignal) =>
    get<AppEntry>(`/api/apps/${encodeURIComponent(name)}`, signal),
  validate: (signal?: AbortSignal) =>
    fetch('/api/validate', { method: 'POST', signal }).then(async (r) => ({
      status: r.status,
      body: await r.json(),
    })),
  render: (app?: string, signal?: AbortSignal) =>
    fetch(`/api/render${app ? `?app=${encodeURIComponent(app)}` : ''}`, {
      signal,
    }).then((r) => r.text()),
  deploy: (name: string, release?: string, signal?: AbortSignal) =>
    post<{ run_id: string }>(
      `/api/apps/${encodeURIComponent(name)}/deploys`,
      { release },
      signal,
    ),
  rollback: (name: string, signal?: AbortSignal) =>
    post<{ run_id: string }>(
      `/api/apps/${encodeURIComponent(name)}/rollback`,
      {},
      signal,
    ),
  build: (name: string, release?: string, signal?: AbortSignal) =>
    post<{ run_id: string }>(
      `/api/apps/${encodeURIComponent(name)}/build`,
      { release },
      signal,
    ),
  sync: (name: string, signal?: AbortSignal) =>
    post<{ run_id: string }>(
      `/api/apps/${encodeURIComponent(name)}/sync`,
      {},
      signal,
    ),
  verify: (name: string, since: string, signal?: AbortSignal) =>
    get<VerifyReport>(
      `/api/apps/${encodeURIComponent(name)}/verify?since=${encodeURIComponent(since)}`,
      signal,
    ),
  runs: (limit = 50, signal?: AbortSignal) =>
    get<RunSummary[]>(`/api/runs?limit=${limit}`, signal),
  run: (id: string, signal?: AbortSignal) =>
    get<TraceEvent[]>(`/api/runs/${encodeURIComponent(id)}`, signal),
  appStatus: (name: string, signal?: AbortSignal) =>
    get<AppStatus>(`/api/apps/${encodeURIComponent(name)}/status`, signal),
  appLatest: (name: string, signal?: AbortSignal) =>
    get<AppLatest>(`/api/apps/${encodeURIComponent(name)}/latest`, signal),
  resume: (id: string, signal?: AbortSignal) =>
    get<ResumeInfo>(`/api/runs/${encodeURIComponent(id)}/resume`, signal),
}
