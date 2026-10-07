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
}

export interface TraceEvent {
  event: 'run_start' | 'step' | 'run_end'
  run_id: string
  ts: string
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

async function get<T>(url: string): Promise<T> {
  const res = await fetch(url)
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`)
  return res.json() as Promise<T>
}

async function post<T>(url: string, body?: unknown): Promise<T> {
  const res = await fetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  })
  const json = await res.json().catch(() => ({}))
  if (!res.ok) throw new Error((json as { error?: string }).error ?? `${res.status}`)
  return json as T
}

export const api = {
  health: () => get<{ status: string }>('/health'),
  apps: () => get<AppEntry[]>('/apps'),
  app: (name: string) => get<AppEntry>(`/apps/${encodeURIComponent(name)}`),
  validate: () =>
    fetch('/validate', { method: 'POST' }).then(async (r) => ({
      status: r.status,
      body: await r.json(),
    })),
  render: (app?: string) =>
    fetch(`/render${app ? `?app=${encodeURIComponent(app)}` : ''}`).then((r) => r.text()),
  deploy: (name: string, release?: string) =>
    post<{ run_id: string }>(`/apps/${encodeURIComponent(name)}/deploys`, { release }),
  rollback: (name: string) =>
    post<{ run_id: string }>(`/apps/${encodeURIComponent(name)}/rollback`, {}),
  build: (name: string, release?: string) =>
    post<{ run_id: string }>(`/apps/${encodeURIComponent(name)}/build`, { release }),
  sync: (name: string) =>
    post<{ run_id: string }>(`/apps/${encodeURIComponent(name)}/sync`, {}),
  verify: (name: string, since: string) =>
    get<Record<string, unknown>>(
      `/apps/${encodeURIComponent(name)}/verify?since=${encodeURIComponent(since)}`,
    ),
  runs: (limit = 50) => get<RunSummary[]>(`/runs?limit=${limit}`),
  run: (id: string) => get<TraceEvent[]>(`/runs/${id}`),
  resume: (id: string) =>
    get<{ last_seq: number; last_step: string; ended: string | null; non_ok_steps: number }>(
      `/runs/${id}/resume`,
    ),
}
