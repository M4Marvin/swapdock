import { createFileRoute, Link } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import {
  api,
  formatStatus,
  type AppEntry,
  type AppLatest,
  type AppStatus,
  type RunSummary,
  type VerifyReport,
} from '../api'
import { Badge } from '@/components/ui/badge'
import { Skeleton } from '@/components/ui/skeleton'
import {
  Card,
  CardAction,
  CardContent,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'

export const Route = createFileRoute('/')({
  component: Index,
})

type StatusEntry = AppStatus | 'error'
type LatestEntry = AppLatest | 'error'
type VerifyEntry = VerifyReport | 'error'

/** How long a health-probe verdict is reused before it is fetched again. */
const STATUS_TTL_MS = 15_000
/** Cross-mount cache, so revisiting the page does not re-probe every app. */
const statusCache = new Map<string, { at: number; status: StatusEntry }>()

/** Formats an RFC3339 timestamp for display, falling back to the raw value. */
function formatTime(iso: string): string {
  const d = new Date(iso)
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString()
}

function formatMs(ms: number): string {
  const d = new Date(ms)
  return Number.isNaN(d.getTime()) ? String(ms) : d.toLocaleTimeString()
}

/** The API computes `up`, but fall back to the 2xx code when it is absent. */
function isUp(status: AppStatus): boolean {
  if (status.up === true) return true
  return (
    typeof status.status === 'number' &&
    status.status >= 200 &&
    status.status < 300
  )
}

function StatusBadge({
  status,
  livePort,
}: {
  status: StatusEntry | undefined
  livePort: number | null
}) {
  // No live port means nothing was ever deployed: not the same as "down".
  if (livePort == null) {
    return (
      <Badge variant="outline" className="text-muted-foreground">
        not deployed
      </Badge>
    )
  }
  if (status === undefined) return <Skeleton className="h-5 w-12 rounded-4xl" />
  if (status === 'error') return <Badge variant="outline">unknown</Badge>
  return isUp(status) ? (
    <Badge
      variant="outline"
      className="border-emerald-500/40 bg-emerald-500/10 text-emerald-500"
    >
      up
    </Badge>
  ) : (
    <Badge variant="destructive">down</Badge>
  )
}

/** Compares the deployed release with what `origin/<branch>` resolves to. */
function UpdateChip({
  release,
  latest,
  livePort,
}: {
  release: string | null
  latest: LatestEntry | undefined
  livePort: number | null
}) {
  if (livePort == null) {
    return (
      <Badge variant="outline" className="text-muted-foreground">
        not deployed
      </Badge>
    )
  }
  if (latest === undefined)
    return <Skeleton className="h-5 w-24 rounded-4xl" />
  if (latest === 'error')
    return <span className="text-xs text-muted-foreground">—</span>
  const remote = latest.release
  if (!remote) {
    return (
      <Badge variant="outline" className="text-muted-foreground">
        no ref
      </Badge>
    )
  }
  if (release != null && release === remote) {
    return (
      <Badge
        variant="outline"
        className="border-emerald-500/40 bg-emerald-500/10 text-emerald-500"
      >
        up to date
      </Badge>
    )
  }
  return (
    <Badge variant="secondary" title={latest.source}>
      update available: {remote}
    </Badge>
  )
}

/** Requests / 5xx / traffic-flip for the last verify window. */
function VerifyCell({ report }: { report: VerifyEntry | undefined }) {
  if (report === undefined) {
    return <Skeleton className="h-5 w-24 rounded-4xl" />
  }
  // A missing access log or an unreadable one is not an error to show.
  if (report === 'error') return null
  return (
    <div className="flex flex-wrap items-center gap-1 text-xs">
      <Badge variant="outline">{report.requests} req</Badge>
      {report.errors_5xx > 0 && (
        <Badge variant="destructive">{report.errors_5xx} 5xx</Badge>
      )}
      {report.flip ? (
        <span className="text-muted-foreground">
          flip {formatMs(report.flip.at_ms)}
        </span>
      ) : (
        <span className="text-muted-foreground">no flip</span>
      )}
    </div>
  )
}

function Index() {
  const [apps, setApps] = useState<AppEntry[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [validate, setValidate] = useState<{
    status: number
    errors: number
    warnings: number
  } | null>(null)
  const [statuses, setStatuses] = useState<Record<string, StatusEntry>>({})
  const [latest, setLatest] = useState<Record<string, LatestEntry>>({})
  const [runsByApp, setRunsByApp] = useState<Record<string, RunSummary[]>>({})
  const [runsReady, setRunsReady] = useState(false)
  const [verify, setVerify] = useState<Record<string, VerifyEntry>>({})

  useEffect(() => {
    const controller = new AbortController()
    api.apps(controller.signal).then(
      (a) => {
        if (!controller.signal.aborted) setApps(a)
      },
      (e) => {
        if (!controller.signal.aborted) setError(String(e))
      },
    )
    api.validate(controller.signal).then(
      (r) =>
        setValidate({
          status: r.status,
          errors: r.body.errors,
          warnings: r.body.warnings,
        }),
      () => setValidate(null),
    )
    return () => controller.abort()
  }, [])

  // Probe every app's live port once the registry is known. Apps without a
  // live port are skipped entirely: the UI renders "not deployed" for them.
  useEffect(() => {
    if (!apps) return
    const controller = new AbortController()
    const now = Date.now()
    const map: Record<string, StatusEntry> = {}
    const toFetch: AppEntry[] = []
    for (const entry of apps) {
      if (entry.app.live_port == null) continue
      const hit = statusCache.get(entry.app.name)
      if (hit && now - hit.at < STATUS_TTL_MS) {
        map[entry.app.name] = hit.status
      } else {
        toFetch.push(entry)
      }
    }
    setStatuses(map)
    if (toFetch.length === 0) return
    Promise.allSettled(
      toFetch.map((e) => api.appStatus(e.app.name, controller.signal)),
    ).then((results) => {
      if (controller.signal.aborted) return
      const at = Date.now()
      setStatuses((prev) => {
        const next = { ...prev }
        results.forEach((r, i) => {
          const name = toFetch[i].app.name
          const status: StatusEntry = r.status === 'fulfilled' ? r.value : 'error'
          next[name] = status
          statusCache.set(name, { at, status })
        })
        return next
      })
    })
    return () => controller.abort()
  }, [apps])

  // What each app's upstream ref resolves to, for the pending-update chip.
  useEffect(() => {
    if (!apps || apps.length === 0) return
    const controller = new AbortController()
    Promise.allSettled(
      apps.map(({ app }) => api.appLatest(app.name, controller.signal)),
    ).then((results) => {
      if (controller.signal.aborted) return
      const map: Record<string, LatestEntry> = {}
      results.forEach((r, i) => {
        map[apps[i].app.name] = r.status === 'fulfilled' ? r.value : 'error'
      })
      setLatest(map)
    })
    return () => controller.abort()
  }, [apps])

  // Run summaries carry their app; keep the newest three per app. Newest-first
  // order means the first three seen are the three most recent.
  useEffect(() => {
    if (!apps || apps.length === 0) return
    const controller = new AbortController()
    setRunsReady(false)
    api.runs(50, controller.signal).then(
      (runs) => {
        if (controller.signal.aborted) return
        const map: Record<string, RunSummary[]> = {}
        for (const run of runs) {
          if (!run.app) continue
          const list = map[run.app] ?? (map[run.app] = [])
          if (list.length < 3) list.push(run)
        }
        setRunsByApp(map)
        setRunsReady(true)
      },
      () => {
        if (!controller.signal.aborted) setRunsReady(true)
      },
    )
    return () => controller.abort()
  }, [apps])

  // Verify each app since its most recent run (or an hour ago when there is no
  // run). A failure — missing or unreadable access log — renders nothing.
  useEffect(() => {
    if (!apps || apps.length === 0 || !runsReady) return
    const controller = new AbortController()
    const fallback = new Date(Date.now() - 3_600_000).toISOString()
    Promise.allSettled(
      apps.map(({ app }) => {
        const since = runsByApp[app.name]?.[0]?.run_id ?? fallback
        return api.verify(app.name, since, controller.signal)
      }),
    ).then((results) => {
      if (controller.signal.aborted) return
      const map: Record<string, VerifyEntry> = {}
      results.forEach((r, i) => {
        map[apps[i].app.name] = r.status === 'fulfilled' ? r.value : 'error'
      })
      setVerify(map)
    })
    return () => controller.abort()
  }, [apps, runsReady, runsByApp])

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h1 className="text-xl font-semibold">Apps</h1>
        {validate && (
          <div className="flex gap-2">
            <Badge variant={validate.errors > 0 ? 'destructive' : 'outline'}>
              {validate.errors} errors
            </Badge>
            <Badge variant="outline">{validate.warnings} warnings</Badge>
          </div>
        )}
      </div>

      {error && <p className="text-destructive">{error}</p>}

      {!apps && !error && (
        <Card>
          <CardContent className="space-y-2 pt-4">
            <Skeleton className="h-6 w-full" />
            <Skeleton className="h-6 w-full" />
            <Skeleton className="h-6 w-2/3" />
          </CardContent>
        </Card>
      )}

      {apps && (
        <Card className="py-0">
          <CardHeader className="border-b py-4">
            <CardTitle>Registry</CardTitle>
            <CardAction>
              <span className="text-sm text-muted-foreground">
                {apps.length} app{apps.length === 1 ? '' : 's'}
              </span>
            </CardAction>
          </CardHeader>
          <CardContent className="px-0">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="pl-6">Name</TableHead>
                  <TableHead>Kind</TableHead>
                  <TableHead>Strategy</TableHead>
                  <TableHead>Release</TableHead>
                  <TableHead>Update</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Live port</TableHead>
                  <TableHead>Last deploy</TableHead>
                  <TableHead>Recent runs</TableHead>
                  <TableHead>Verify</TableHead>
                  <TableHead className="pr-6">Issues</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {apps.map(({ app, problems }) => {
                  const runs = runsByApp[app.name] ?? []
                  const newest = runs[0]
                  return (
                    <TableRow key={app.name}>
                      <TableCell className="pl-6">
                        <Link
                          to="/apps/$appName"
                          params={{ appName: app.name }}
                          className="font-medium underline-offset-4 hover:underline"
                        >
                          {app.name}
                        </Link>
                      </TableCell>
                      <TableCell>{app.kind}</TableCell>
                      <TableCell>{app.strategy}</TableCell>
                      <TableCell className="font-mono text-xs">
                        {app.release ?? '—'}
                      </TableCell>
                      <TableCell>
                        <UpdateChip
                          release={app.release}
                          latest={latest[app.name]}
                          livePort={app.live_port}
                        />
                      </TableCell>
                      <TableCell>
                        <StatusBadge
                          status={statuses[app.name]}
                          livePort={app.live_port}
                        />
                      </TableCell>
                      <TableCell>{app.live_port ?? '—'}</TableCell>
                      <TableCell>
                        {newest ? (
                          <Link
                            to="/runs/$runId"
                            params={{ runId: newest.run_id }}
                            title={newest.run_id}
                            className="inline-flex items-center gap-2 underline-offset-4 hover:underline"
                          >
                            <span className="font-mono text-xs">
                              {newest.run_id.slice(0, 8)}
                            </span>
                            <span className="text-muted-foreground">
                              {formatTime(newest.started)}
                            </span>
                            <Badge
                              variant={
                                newest.non_ok > 0 ? 'destructive' : 'outline'
                              }
                            >
                              {formatStatus(newest.status)}
                            </Badge>
                          </Link>
                        ) : (
                          <span className="text-muted-foreground">—</span>
                        )}
                      </TableCell>
                      <TableCell>
                        {runs.length === 0 ? (
                          <span className="text-muted-foreground">—</span>
                        ) : (
                          <div className="flex flex-col gap-1">
                            {runs.map((run) => (
                              <Link
                                key={run.run_id}
                                to="/runs/$runId"
                                params={{ runId: run.run_id }}
                                title={run.run_id}
                                className="inline-flex items-center gap-2 underline-offset-4 hover:underline"
                              >
                                <span className="font-mono text-xs">
                                  {run.run_id.slice(0, 8)}
                                </span>
                                <Badge
                                  variant={
                                    run.non_ok > 0 ? 'destructive' : 'outline'
                                  }
                                >
                                  {formatStatus(run.status)}
                                </Badge>
                              </Link>
                            ))}
                          </div>
                        )}
                      </TableCell>
                      <TableCell>
                        <VerifyCell report={verify[app.name]} />
                      </TableCell>
                      <TableCell className="pr-6">
                        {problems.length === 0 ? (
                          <Badge variant="outline">0</Badge>
                        ) : (
                          <Badge
                            variant={
                              problems.some((p) => p.severity === 'error')
                                ? 'destructive'
                                : 'outline'
                            }
                          >
                            {problems.length}
                          </Badge>
                        )}
                      </TableCell>
                    </TableRow>
                  )
                })}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      )}
    </div>
  )
}
