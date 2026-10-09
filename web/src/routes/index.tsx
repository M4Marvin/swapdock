import { createFileRoute, Link } from '@tanstack/react-router'
import { useMemo, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import {
  api,
  formatStatus,
  type AppEntry,
  type AppLatest,
  type RunSummary,
  type VerifyReport,
} from '../api'
import { queryKeys } from '@/lib/query-keys'
import { StatusPill } from '@/components/StatusPill'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
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

/** Formats an RFC3339 timestamp for display, falling back to the raw value. */
function formatTime(iso: string): string {
  const d = new Date(iso)
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString()
}

function formatMs(ms: number): string {
  const d = new Date(ms)
  return Number.isNaN(d.getTime()) ? String(ms) : d.toLocaleTimeString()
}

/** Compares the deployed release with what `origin/<branch>` resolves to. */
function UpdateChip({
  release,
  latest,
  latestError,
  livePort,
}: {
  release: string | null
  latest: AppLatest | undefined
  latestError: boolean
  livePort: number | null
}) {
  if (livePort == null) {
    return (
      <Badge variant="outline" className="text-muted-foreground">
        not deployed
      </Badge>
    )
  }
  if (latestError)
    return (
      <Badge variant="warning" title="could not read the upstream ref">
        check failed
      </Badge>
    )
  if (latest === undefined)
    return <Skeleton className="h-5 w-24 rounded-4xl" />
  const remote = latest.release
  if (!remote) {
    return (
      <Badge variant="outline" className="text-muted-foreground">
        no ref
      </Badge>
    )
  }
  if (release != null && release === remote) {
    return <Badge variant="success">up to date</Badge>
  }
  return (
    <Badge variant="secondary" title={latest.source}>
      update available: {remote}
    </Badge>
  )
}

/** Requests / 5xx / traffic-flip for the last verify window. */
function VerifyCell({
  report,
  pending,
  error,
}: {
  report: VerifyReport | undefined
  pending: boolean
  error: boolean
}) {
  if (pending) return <Skeleton className="h-5 w-24 rounded-4xl" />
  // A missing access log or an unreadable one is not an error to show.
  if (error)
    return (
      <span className="text-xs text-muted-foreground" title="could not read the access log">
        unavailable
      </span>
    )
  if (report === undefined) return <Skeleton className="h-5 w-24 rounded-4xl" />
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

/**
 * One registry row.
 *
 * Each row owns its own status, latest-ref, and verify queries, keyed by app
 * name, so a probe for one app never blocks or re-renders the others.
 */
function AppRow({
  entry,
  runs,
  runsReady,
  fallbackSince,
}: {
  entry: AppEntry
  runs: RunSummary[]
  runsReady: boolean
  fallbackSince: string
}) {
  const { app, problems } = entry
  const newest = runs[0]

  const latestQuery = useQuery({
    queryKey: queryKeys.latest(app.name),
    queryFn: ({ signal }) => api.appLatest(app.name, signal),
    enabled: app.live_port != null,
  })

  const since = newest?.run_id ?? fallbackSince
  const verifyQuery = useQuery({
    queryKey: queryKeys.verify(app.name, since),
    queryFn: ({ signal }) => api.verify(app.name, since, signal),
    enabled: runsReady,
    retry: false,
  })

  return (
    <TableRow>
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
      <TableCell className="font-mono text-xs" title={app.release ?? undefined}>
        {app.release ? app.release.slice(0, 12) : '—'}
      </TableCell>
      <TableCell>
        <UpdateChip
          release={app.release}
          latest={latestQuery.data}
          latestError={latestQuery.isError}
          livePort={app.live_port}
        />
      </TableCell>
      <TableCell>
        <StatusPill name={app.name} livePort={app.live_port} />
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
            <Badge variant="outline">{formatStatus(newest.status)}</Badge>
            {newest.non_ok > 0 && (
              <Badge variant="destructive">{newest.non_ok} non-ok</Badge>
            )}
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
                <Badge variant="outline">{formatStatus(run.status)}</Badge>
                {run.non_ok > 0 && (
                  <Badge variant="destructive">{run.non_ok} non-ok</Badge>
                )}
              </Link>
            ))}
          </div>
        )}
      </TableCell>
      <TableCell>
        <VerifyCell
          report={verifyQuery.data}
          pending={verifyQuery.isPending}
          error={verifyQuery.isError}
        />
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
}

function Index() {
  const appsQuery = useQuery({
    queryKey: queryKeys.apps,
    queryFn: ({ signal }) => api.apps(signal),
  })

  const validateQuery = useQuery({
    queryKey: queryKeys.validate,
    queryFn: ({ signal }) => api.validate(signal),
  })

  const runsQuery = useQuery({
    queryKey: queryKeys.runs,
    queryFn: ({ signal }) => api.runs(50, signal),
  })

  // A stable fallback window for apps with no recorded run, captured once so
  // the verify query key does not shift on every render.
  const [fallbackSince] = useState(() =>
    new Date(Date.now() - 3_600_000).toISOString(),
  )

  // Run summaries carry their app; keep the newest three per app. Newest-first
  // order means the first three seen are the three most recent.
  const runsByApp = useMemo(() => {
    const map: Record<string, RunSummary[]> = {}
    for (const run of runsQuery.data ?? []) {
      if (!run.app) continue
      const list = map[run.app] ?? (map[run.app] = [])
      if (list.length < 3) list.push(run)
    }
    return map
  }, [runsQuery.data])

  const runsReady = runsQuery.isSuccess || runsQuery.isError
  const apps = appsQuery.data
  const validate = validateQuery.data

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h1 className="text-xl font-semibold">Apps</h1>
        {validate && (
          <div className="flex gap-2">
            <Badge
              variant={validate.body.errors > 0 ? 'destructive' : 'outline'}
            >
              {validate.body.errors} errors
            </Badge>
            <Badge variant="outline">{validate.body.warnings} warnings</Badge>
          </div>
        )}
      </div>

      {appsQuery.isError && (
        <div
          role="alert"
          className="flex flex-wrap items-center gap-3 rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive"
        >
          <span>Could not load the registry: {String(appsQuery.error)}</span>
          <Button
            size="sm"
            variant="outline"
            onClick={() => void appsQuery.refetch()}
          >
            Retry
          </Button>
        </div>
      )}

      {appsQuery.isPending && (
        <Card className="py-0">
          <CardHeader className="border-b py-4">
            <Skeleton className="h-5 w-24" />
          </CardHeader>
          <CardContent className="space-y-3 px-6 py-4">
            <Skeleton className="h-6 w-full" />
            <Skeleton className="h-6 w-full" />
            <Skeleton className="h-6 w-2/3" />
          </CardContent>
        </Card>
      )}

      {apps && apps.length === 0 && (
        <Card>
          <CardContent className="p-6 text-sm text-muted-foreground">
            No apps in the registry.
          </CardContent>
        </Card>
      )}

      {apps && apps.length > 0 && (
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
                {apps.map((entry) => (
                  <AppRow
                    key={entry.app.name}
                    entry={entry}
                    runs={runsByApp[entry.app.name] ?? []}
                    runsReady={runsReady}
                    fallbackSince={fallbackSince}
                  />
                ))}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      )}
    </div>
  )
}
