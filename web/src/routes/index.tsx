import { createFileRoute, Link } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import {
  api,
  formatStatus,
  type AppEntry,
  type AppStatus,
  type RunSummary,
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

/** How long a health-probe verdict is reused before it is fetched again. */
const STATUS_TTL_MS = 15_000
/** Cross-mount cache, so revisiting the page does not re-probe every app. */
const statusCache = new Map<string, { at: number; status: StatusEntry }>()

/** Formats an RFC3339 timestamp for display, falling back to the raw value. */
function formatTime(iso: string): string {
  const d = new Date(iso)
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString()
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

function Index() {
  const [apps, setApps] = useState<AppEntry[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [validate, setValidate] = useState<{
    status: number
    errors: number
    warnings: number
  } | null>(null)
  const [statuses, setStatuses] = useState<Record<string, StatusEntry>>({})
  const [lastRuns, setLastRuns] = useState<Record<string, RunSummary>>({})

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

  // Run summaries now carry their app, so the latest run per app is just the
  // first one seen in the newest-first list — no per-run detail fetch needed.
  useEffect(() => {
    if (!apps || apps.length === 0) return
    const controller = new AbortController()
    api.runs(50, controller.signal).then(
      (runs) => {
        if (controller.signal.aborted) return
        const map: Record<string, RunSummary> = {}
        for (const run of runs) {
          if (run.app && !map[run.app]) map[run.app] = run
        }
        setLastRuns(map)
      },
      () => undefined,
    )
    return () => controller.abort()
  }, [apps])

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
                  <TableHead>Status</TableHead>
                  <TableHead>Live port</TableHead>
                  <TableHead>Hostnames</TableHead>
                  <TableHead>Last deploy</TableHead>
                  <TableHead className="pr-6">Issues</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {apps.map(({ app, problems }) => {
                  const run = lastRuns[app.name]
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
                        <StatusBadge
                          status={statuses[app.name]}
                          livePort={app.live_port}
                        />
                      </TableCell>
                      <TableCell>{app.live_port ?? '—'}</TableCell>
                      <TableCell className="text-muted-foreground">
                        {app.hostnames.join(', ') || '—'}
                      </TableCell>
                      <TableCell>
                        {run ? (
                          <Link
                            to="/runs/$runId"
                            params={{ runId: run.run_id }}
                            title={run.run_id}
                            className="inline-flex items-center gap-2 underline-offset-4 hover:underline"
                          >
                            <span className="font-mono text-xs">
                              {run.run_id.slice(0, 8)}
                            </span>
                            <span className="text-muted-foreground">
                              {formatTime(run.started)}
                            </span>
                            <Badge
                              variant={
                                run.non_ok > 0 ? 'destructive' : 'outline'
                              }
                            >
                              {formatStatus(run.status)}
                            </Badge>
                          </Link>
                        ) : (
                          <span className="text-muted-foreground">—</span>
                        )}
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
