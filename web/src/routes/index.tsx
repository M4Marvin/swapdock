import { useMemo, useState } from "react"
import { createFileRoute, Link } from "@tanstack/react-router"
import { useQueries, useQuery } from "@tanstack/react-query"
import {
  Activity,
  ArrowUpRight,
  ChevronRight,
  CircleAlert,
  Gauge,
  Layers,
  RefreshCw,
  Rocket,
  Stethoscope,
  TriangleAlert,
  WifiOff,
} from "lucide-react"
import {
  api,
  classifyStatus,
  type AppEntry,
  type AppLatest,
  type AppStatus,
  type Problem,
  type RunSummary,
} from "@/api"
import { queryKeys } from "@/lib/query-keys"
import { formatCount, plural } from "@/lib/format"
import { useDocumentTitle } from "@/lib/hooks"
import { cn } from "cn"
import { isAppUp } from "@/components/Status"
import { needsAdoption } from "@/lib/deployment"
import { PageHeader } from "@/components/PageHeader"
import { AppGlyph } from "@/components/AppGlyph"
import { HealthPill, RunStatusPill, StatusDot, runTone } from "@/components/Status"
import {
  Callout,
  CodeSnippet,
  EmptyState,
  FilterPills,
  RelativeTime,
  SearchInput,
  SectionHeading,
  Spinner,
  StatTile,
} from "@/components/kit"
import { Button } from "@/components/ui/button"
import { NewAppButton } from "@/components/app/NewAppDialog"
import { Skeleton } from "@/components/ui/skeleton"

export const Route = createFileRoute("/")({
  component: OverviewPage,
})

type AppFilter = "all" | "live" | "attention" | "unmanaged" | "idle"

function sameRelease(a: string | null | undefined, b: string | null | undefined): boolean {
  if (!a || !b) return false
  return a === b || a.startsWith(b) || b.startsWith(a)
}

function codeLabel(code: string): string {
  return code.replace(/-/g, " ").replace(/^./, (char) => char.toUpperCase())
}

function useOverviewData() {
  const appsQuery = useQuery({
    queryKey: queryKeys.apps,
    queryFn: ({ signal }) => api.apps(signal),
    placeholderData: (previous) => previous,
  })
  const runsQuery = useQuery({
    queryKey: queryKeys.runs,
    queryFn: ({ signal }) => api.runs(50, signal),
    refetchInterval: 30_000,
    placeholderData: (previous) => previous,
  })
  const validateQuery = useQuery({
    queryKey: queryKeys.validate,
    queryFn: ({ signal }) => api.validate(signal),
    retry: false,
    staleTime: 60_000,
  })
  const doctorQuery = useQuery({
    queryKey: [...queryKeys.doctor, false],
    queryFn: ({ signal }) => api.doctorAll(false, signal),
    retry: false,
    staleTime: 60_000,
  })

  const apps = appsQuery.data ?? []
  const liveEntries = useMemo(
    () => apps.filter((entry) => entry.app.live_port != null),
    [apps]
  )

  // One health probe and one upstream check per deployed app. They share cache
  // keys with the app detail page, so navigating does not double-fetch.
  const statusResults = useQueries({
    queries: liveEntries.map((entry) => ({
      queryKey: queryKeys.appStatus(entry.app.name),
      queryFn: ({ signal }: { signal: AbortSignal }) => api.appStatus(entry.app.name, signal),
      refetchInterval: 20_000,
      staleTime: 10_000,
      retry: 1,
    })),
  })
  const latestResults = useQueries({
    queries: liveEntries.map((entry) => ({
      queryKey: queryKeys.latest(entry.app.name),
      queryFn: ({ signal }: { signal: AbortSignal }) => api.appLatest(entry.app.name, signal),
      staleTime: 60_000,
      retry: 0,
    })),
  })

  const statusByName = useMemo(() => {
    const map = new Map<string, AppStatus>()
    liveEntries.forEach((entry, index) => {
      const data = statusResults[index]?.data
      if (data) map.set(entry.app.name, data)
    })
    return map
  }, [liveEntries, statusResults])

  const latestByName = useMemo(() => {
    const map = new Map<string, AppLatest>()
    liveEntries.forEach((entry, index) => {
      const data = latestResults[index]?.data
      if (data) map.set(entry.app.name, data)
    })
    return map
  }, [liveEntries, latestResults])

  return { appsQuery, runsQuery, validateQuery, doctorQuery, apps, statusByName, latestByName }
}

/* -------------------------------------------------------------------------- */
/* application row                                                            */
/* -------------------------------------------------------------------------- */

function AppRow({
  entry,
  runs,
  latest,
}: {
  entry: AppEntry
  runs: RunSummary[]
  latest: AppLatest | undefined
}) {
  const { app, problems } = entry
  const lastRun = runs[0]
  const hasProblems = problems.length > 0
  const hasError = problems.some((problem) => problem.severity === "error")
  const update =
    latest?.release != null && app.release != null && !sameRelease(app.release, latest.release)

  return (
    <Link
      to="/apps/$appName"
      params={{ appName: app.name }}
      search={{ tab: "overview" }}
      className="group row-hover flex items-center gap-3 border-b border-border/60 px-4 py-3 last:border-0"
    >
      <AppGlyph name={app.name} size="md" />

      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <span className="truncate text-sm font-medium text-foreground">{app.name}</span>
          {hasProblems ? (
            <span
              title={problems.map((problem) => problem.message).join("\n")}
              className={cn(
                "inline-flex h-4 items-center gap-1 rounded px-1 text-2xs",
                hasError ? "bg-destructive/10 text-destructive-strong" : "bg-warning/10 text-warning-strong"
              )}
            >
              <TriangleAlert className="size-2.5" />
              {problems.length}
              <span className="sr-only">
                registry finding{problems.length === 1 ? "" : "s"}
              </span>
            </span>
          ) : null}
        </div>
        <div className="mt-0.5 flex min-w-0 items-center gap-1.5 text-2xs text-muted-foreground">
          <span className="truncate">
            {app.hostnames.length > 0 ? app.hostnames[0] : "No hostname"}
          </span>
          <span className="text-subtle-foreground/40">·</span>
          <span className="shrink-0">{app.kind}</span>
          <span className="hidden text-subtle-foreground/40 sm:inline">·</span>
          <span className="hidden shrink-0 sm:inline">{app.strategy}</span>
          <span className="md:hidden">
            <HealthPill
              name={app.name}
              livePort={app.live_port}
              staticServed={app.kind === "static" && app.root != null}
              unmanaged={needsAdoption(app)}
            />
          </span>
        </div>
      </div>

      <div className="hidden w-[104px] shrink-0 md:block">
        <HealthPill
          name={app.name}
          livePort={app.live_port}
          staticServed={app.kind === "static" && app.root != null}
          unmanaged={needsAdoption(app)}
        />
      </div>

      <div className="hidden w-[148px] shrink-0 lg:block">
        {app.release ? (
          <div className="flex items-center gap-1.5">
            <span
              className="truncate font-mono text-xs text-foreground"
              title={`release ${app.release}`}
            >
              {app.release.length > 10 ? app.release.slice(0, 10) : app.release}
            </span>
            {update ? (
              <span
                title={`Upstream is at ${latest?.release} (${latest?.source ?? "git"})`}
                className="inline-flex items-center text-accent"
              >
                <ArrowUpRight className="size-3" />
                <span className="sr-only">Update available</span>
              </span>
            ) : null}
          </div>
        ) : needsAdoption(app) ? (
          <span className="text-xs text-subtle-foreground">Not managed</span>
        ) : app.kind === "static" && app.root != null ? (
          <span className="text-xs text-info-strong">Static, served</span>
        ) : (
          <span className="text-xs text-subtle-foreground">Not deployed</span>
        )}
        <div className="mt-px truncate text-2xs text-subtle-foreground">
          {app.live_port != null
            ? `:${app.live_port} → front ${app.front_port}`
            : `front :${app.front_port}`}
        </div>
      </div>

      <div className="hidden w-[126px] shrink-0 xl:block">
        {lastRun ? (
          <div className="flex items-center gap-1.5">
            <StatusDot tone={runTone(lastRun.status)} />
            <RelativeTime value={lastRun.started} className="text-xs text-muted-foreground" />
            {lastRun.non_ok > 0 ? (
              <span
                className="text-2xs text-warning-strong"
                title={`${lastRun.non_ok} non-ok step${lastRun.non_ok === 1 ? "" : "s"}`}
              >
                {lastRun.non_ok}
              </span>
            ) : null}
          </div>
        ) : (
          <span className="text-xs text-subtle-foreground">Never</span>
        )}
      </div>

      <ChevronRight className="size-3.5 shrink-0 text-subtle-foreground/50 transition-[color,transform] duration-150 group-hover:translate-x-0.5 group-hover:text-muted-foreground" />
    </Link>
  )
}

/* -------------------------------------------------------------------------- */
/* right rail                                                                 */
/* -------------------------------------------------------------------------- */

function DoctorSummary({
  problems,
  loading,
  error,
}: {
  problems: Problem[]
  loading: boolean
  error: boolean
}) {
  const errors = problems.filter((problem) => problem.severity === "error").length
  const warnings = problems.length - errors
  const affected = useMemo(
    () => new Set(problems.map((problem) => problem.app).filter(Boolean)).size,
    [problems]
  )
  const byCode = useMemo(() => {
    const counts = new Map<string, number>()
    for (const problem of problems) counts.set(problem.code, (counts.get(problem.code) ?? 0) + 1)
    return [...counts.entries()].sort((a, b) => b[1] - a[1]).slice(0, 4)
  }, [problems])
  const max = Math.max(1, ...byCode.map(([, count]) => count))

  return (
    <div className="rounded-xl border border-border bg-card shadow-[var(--shadow-card)]">
      <div className="flex items-start justify-between gap-3 border-b border-border/60 px-4 py-3">
        <div>
          <h2 className="text-sm font-medium tracking-[-0.01em]">Host findings</h2>
          <p className="text-2xs text-muted-foreground">
            Docker, compose, ports, health · separate from registry attention
          </p>
        </div>
        <Stethoscope className="mt-0.5 size-3.5 text-subtle-foreground" />
      </div>
      <div className="p-4">
        {loading ? (
          <div className="space-y-3">
            <div className="flex items-center gap-2 text-xs text-muted-foreground">
              <Spinner className="size-3" />
              Reading the host…
            </div>
            <Skeleton className="h-3 w-full" />
            <Skeleton className="h-3 w-4/5" />
            <Skeleton className="h-3 w-2/3" />
          </div>
        ) : error ? (
          <p className="text-xs leading-relaxed text-muted-foreground">
            The host check could not complete. Open Doctor to retry against the live host.
          </p>
        ) : problems.length === 0 ? (
          <div className="flex items-center gap-2 text-sm">
            <StatusDot tone="success" />
            <span className="text-foreground">Everything checks out</span>
          </div>
        ) : (
          <>
            <div className="flex items-baseline gap-2">
              <span
                className={cn(
                  "tabular text-2xl font-semibold tracking-[-0.03em]",
                  errors > 0 ? "text-destructive-strong" : "text-foreground"
                )}
              >
                {errors > 0 ? errors : warnings}
              </span>
              <span className="text-xs text-muted-foreground">
                {errors > 0 ? "errors" : "warnings"}
              </span>
            </div>
            <p className="mt-1 text-2xs text-muted-foreground">
              {errors > 0
                ? `${plural(warnings, "warning")} alongside`
                : `across ${plural(affected, "app")}`}
            </p>
            <div className="mt-3.5 space-y-2">
              {byCode.map(([code, count]) => (
                <div key={code} className="space-y-1">
                  <div className="flex items-center justify-between gap-3 text-2xs">
                    <span className="truncate text-muted-foreground">{codeLabel(code)}</span>
                    <span className="tabular text-subtle-foreground">{count}</span>
                  </div>
                  <div className="h-1 overflow-hidden rounded-full bg-muted">
                    <div
                      className="h-full rounded-full bg-warning/60"
                      style={{ width: `${Math.max(8, (count / max) * 100)}%` }}
                    />
                  </div>
                </div>
              ))}
            </div>
          </>
        )}
      </div>
      <div className="border-t border-border/60 px-4 py-2.5">
        <Link
          to="/doctor"
          className="group inline-flex items-center gap-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
        >
          Open doctor
          <ChevronRight className="size-3 transition-transform duration-150 group-hover:translate-x-0.5" />
        </Link>
      </div>
    </div>
  )
}

function RecentRuns({ runs, loading }: { runs: RunSummary[]; loading: boolean }) {
  return (
    <div className="rounded-xl border border-border bg-card shadow-[var(--shadow-card)]">
      <div className="flex items-start justify-between gap-3 border-b border-border/60 px-4 py-3">
        <div>
          <h2 className="text-sm font-medium tracking-[-0.01em]">Recent activity</h2>
          <p className="text-2xs text-muted-foreground">The last commands written to the log</p>
        </div>
        <Activity className="mt-0.5 size-3.5 text-subtle-foreground" />
      </div>
      <div className="p-1.5">
        {loading ? (
          <div className="space-y-1 p-2">
            {[0, 1, 2, 3].map((index) => (
              <div key={index} className="flex items-center gap-2 py-1.5">
                <span className="size-1.5 rounded-full bg-muted" />
                <Skeleton className="h-3 flex-1" />
              </div>
            ))}
          </div>
        ) : runs.length === 0 ? (
          <p className="px-2.5 py-6 text-center text-xs text-muted-foreground">
            No runs recorded yet.
          </p>
        ) : (
          runs.map((run) => {
            const running = classifyStatus(run.status) === "running"
            return (
              <Link
                key={run.run_id}
                to="/runs/$runId"
                params={{ runId: run.run_id }}
                className="flex items-center gap-2.5 rounded-lg px-2.5 py-2 transition-colors duration-150 hover:bg-muted/60"
              >
                <StatusDot tone={runTone(run.status)} pulse={running} />
                <div className="min-w-0 flex-1">
                  <div className="flex items-baseline justify-between gap-2">
                    <span className="truncate text-xs font-medium text-foreground">
                      {run.app ?? "Registry"}
                    </span>
                    <RelativeTime
                      value={run.started}
                      className="shrink-0 text-2xs text-subtle-foreground"
                    />
                  </div>
                  <div className="mt-0.5 flex items-center gap-1.5 text-2xs text-subtle-foreground">
                    <span className="font-mono">{run.run_id.slice(0, 7)}</span>
                    <span>·</span>
                    {running ? (
                      <span className="text-info">running</span>
                    ) : (
                      <span>{plural(run.steps, "step")}</span>
                    )}
                    {run.non_ok > 0 ? (
                      <>
                        <span>·</span>
                        <span className="text-warning-strong">{run.non_ok} non-ok</span>
                      </>
                    ) : null}
                  </div>
                </div>
              </Link>
            )
          })
        )}
      </div>
      <div className="border-t border-border/60 px-4 py-2.5">
        <Link
          to="/runs"
          className="group inline-flex items-center gap-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
        >
          All runs
          <ChevronRight className="size-3 transition-transform duration-150 group-hover:translate-x-0.5" />
        </Link>
      </div>
    </div>
  )
}

/* -------------------------------------------------------------------------- */
/* page                                                                       */
/* -------------------------------------------------------------------------- */

function OverviewPage() {
  useDocumentTitle("Fleet overview")
  const {
    appsQuery,
    runsQuery,
    validateQuery,
    doctorQuery,
    apps,
    statusByName,
    latestByName,
  } = useOverviewData()
  const [filter, setFilter] = useState<AppFilter>("all")
  const [search, setSearch] = useState("")

  const runs = runsQuery.data ?? []
  const liveApps = apps.filter((entry) => entry.app.live_port != null)
  const staticServed = apps.filter(
    (entry) => entry.app.kind === "static" && entry.app.root != null
  ).length
  const unmanagedApps = apps.filter((entry) => needsAdoption(entry.app)).length
  const idleApps = Math.max(
    0,
    apps.length - liveApps.length - staticServed - unmanagedApps
  )
  const upCount = liveApps.filter((entry) => isAppUp(statusByName.get(entry.app.name))).length
  const downApps = liveApps.filter((entry) => {
    const status = statusByName.get(entry.app.name)
    return status != null && !isAppUp(status)
  })
  const attention = apps.filter(
    (entry) =>
      entry.problems.length > 0 || downApps.some((down) => down.app.name === entry.app.name)
  )

  const validation = validateQuery.data?.body as
    | { errors?: number; warnings?: number }
    | undefined
  const validationOk = validateQuery.isSuccess && validation != null
  const errors = validationOk ? (validation.errors ?? 0) : null
  const warnings = validationOk ? (validation.warnings ?? 0) : null
  const hostFindings = doctorQuery.isSuccess
    ? (doctorQuery.data?.problems.length ?? 0)
    : null

  const dayAgo = Date.now() - 86_400_000
  const lastDay = runs.filter((run) => {
    const started = Date.parse(run.started)
    return Number.isFinite(started) && started >= dayAgo
  })
  const okRuns = lastDay.filter((run) => classifyStatus(run.status) === "ok").length
  const failedRuns = lastDay.filter((run) => classifyStatus(run.status) === "failed").length
  const runningRuns = lastDay.filter((run) => classifyStatus(run.status) === "running").length
  const successPct = lastDay.length === 0 ? 0 : Math.round((okRuns / lastDay.length) * 100)

  const latestDeploy = runs.find((run) => run.app != null)

  const runsByApp = useMemo(() => {
    const map = new Map<string, RunSummary[]>()
    for (const run of runs) {
      if (!run.app) continue
      const list = map.get(run.app) ?? []
      list.push(run)
      map.set(run.app, list)
    }
    return map
  }, [runs])

  const filteredApps = useMemo(() => {
    const needle = search.trim().toLowerCase()
    return apps
      .filter((entry) => {
        if (filter === "live" && entry.app.live_port == null) return false
        if (filter === "unmanaged" && !needsAdoption(entry.app)) return false
        if (
          filter === "idle" &&
          (entry.app.live_port != null ||
            needsAdoption(entry.app) ||
            (entry.app.kind === "static" && entry.app.root != null))
        ) {
          return false
        }
        if (
          filter === "attention" &&
          entry.problems.length === 0 &&
          !downApps.some((down) => down.app.name === entry.app.name)
        ) {
          return false
        }
        if (!needle) return true
        return (
          entry.app.name.toLowerCase().includes(needle) ||
          entry.app.hostnames.some((host) => host.toLowerCase().includes(needle)) ||
          entry.app.kind.toLowerCase().includes(needle) ||
          entry.app.strategy.toLowerCase().includes(needle)
        )
      })
      .sort((a, b) => a.app.name.localeCompare(b.app.name))
  }, [apps, filter, search, downApps])

  const loading = appsQuery.isPending && appsQuery.data == null
  const hasApps = apps.length > 0
  const appsError = appsQuery.isError && appsQuery.data == null

  return (
    <div className="space-y-5">
      <PageHeader
        eyebrow="Fleet"
        title="Overview"
        description="Every application behind the front door, with the state that matters and the next action within reach."
        badges={
          downApps.length > 0 ? (
            <span className="inline-flex items-center gap-1.5 rounded-full border border-destructive/20 bg-destructive/10 px-2 py-0.5 text-2xs font-medium text-destructive-strong">
              <CircleAlert className="size-3" />
              {downApps.length} down
            </span>
          ) : null
        }
        actions={
          <>
            <Button variant="outline" size="sm" render={<Link to="/doctor" />}>
              <Stethoscope />
              Doctor
            </Button>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="Refresh fleet"
              title="Refresh fleet"
              disabled={appsQuery.isFetching}
              onClick={() => {
                void appsQuery.refetch()
                void runsQuery.refetch()
                void validateQuery.refetch()
                void doctorQuery.refetch()
              }}
            >
              <RefreshCw className={appsQuery.isFetching ? "animate-spin" : undefined} />
            </Button>
          </>
        }
      />

      {loading ? (
        <OverviewSkeleton />
      ) : appsError ? (
        <EmptyState
          icon={WifiOff}
          title="The registry is out of reach"
          description="The API did not answer. Check the connection, then retry — the shell will pick the estate back up."
          action={
            <Button variant="outline" size="sm" onClick={() => void appsQuery.refetch()}>
              <RefreshCw />
              Retry
            </Button>
          }
        />
      ) : !hasApps ? (
        <EmptyState
          icon={Layers}
          title="No applications yet"
          description="Add your first application here. swapdock writes one TOML file per app; you set its ports and hostname before deploying."
          action={
            <>
              <NewAppButton size="sm" />
              <Button variant="ghost" size="sm" onClick={() => void appsQuery.refetch()}>
                <RefreshCw />
                Refresh registry
              </Button>
            </>
          }
        >
          <details className="mt-2 w-full max-w-sm rounded-lg border border-border bg-surface/40 text-left">
            <summary className="cursor-pointer list-none px-3 py-2 text-2xs text-muted-foreground transition-colors hover:text-foreground">
              Use the CLI instead
            </summary>
            <div className="border-t border-border/60 p-2">
              <CodeSnippet code="swapdock init my-app" className="text-left" />
            </div>
          </details>
        </EmptyState>
      ) : (
        <>
          <div className="grid grid-cols-2 gap-3 xl:grid-cols-4">
            <StatTile
              label="Healthy"
              value={upCount}
              hint={`of ${plural(apps.length, "app")}`}
              icon={Gauge}
              tone={upCount === liveApps.length && liveApps.length > 0 ? "success" : "neutral"}
            >
              <div className="mt-2 flex flex-wrap items-center gap-x-2 gap-y-1 text-2xs text-muted-foreground">
                <span className="flex items-center gap-1">
                  <StatusDot tone="success" />
                  {liveApps.length} deployed
                </span>
                {unmanagedApps > 0 ? (
                  <span className="flex items-center gap-1">
                    <StatusDot tone="warning" />
                    {unmanagedApps} unmanaged
                  </span>
                ) : null}
                {staticServed > 0 ? (
                  <span className="flex items-center gap-1">
                    <StatusDot tone="info" />
                    {staticServed} static
                  </span>
                ) : null}
                {idleApps > 0 ? (
                  <span className="flex items-center gap-1">
                    <StatusDot tone="neutral" />
                    {idleApps} idle
                  </span>
                ) : null}
                {downApps.length > 0 ? (
                  <span className="flex items-center gap-1 text-destructive-strong">
                    <StatusDot tone="danger" />
                    {downApps.length} down
                  </span>
                ) : null}
              </div>
            </StatTile>

            <StatTile
              label="Attention"
              value={attention.length}
              hint="registry + live probes"
              icon={TriangleAlert}
              tone={
                attention.length > 0 || (errors ?? 0) > 0
                  ? "warning"
                  : validationOk
                    ? "success"
                    : "neutral"
              }
            >
              <div className="mt-2 space-y-0.5 text-2xs text-muted-foreground">
                {validationOk ? (
                  <p>
                    {errors} errors · {warnings} registry warnings
                  </p>
                ) : validateQuery.isError ? (
                  <p className="flex items-center gap-1 text-warning-strong">
                    Validation unavailable
                    <button
                      type="button"
                      onClick={() => void validateQuery.refetch()}
                      className="underline underline-offset-2 hover:text-foreground"
                    >
                      Retry
                    </button>
                  </p>
                ) : (
                  <p>Validation pending…</p>
                )}
                <p className="text-subtle-foreground">
                  {hostFindings == null
                    ? "Host findings unavailable"
                    : `${plural(hostFindings, "host finding")} (separate)`}
                </p>
              </div>
            </StatTile>

            <StatTile
              label="Runs · 24h"
              value={formatCount(lastDay.length)}
              hint={lastDay.length > 0 ? `${successPct}% succeeded` : "nothing yet"}
              icon={Activity}
              tone={failedRuns > 0 ? "warning" : "neutral"}
            >
              <div className="mt-2.5 h-1 overflow-hidden rounded-full bg-muted">
                <div
                  className={cn(
                    "h-full rounded-full",
                    failedRuns > 0 ? "bg-warning" : "bg-success"
                  )}
                  style={{ width: `${lastDay.length === 0 ? 0 : Math.max(6, successPct)}%` }}
                />
              </div>
              <div className="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-2xs text-muted-foreground">
                <span>{okRuns} ok</span>
                {failedRuns > 0 ? <span className="text-destructive-strong">{failedRuns} failed</span> : null}
                {runningRuns > 0 ? (
                  <span className="text-info">{runningRuns} running</span>
                ) : null}
              </div>
            </StatTile>

            <StatTile
              label="Latest deploy"
              value={<RelativeTime value={latestDeploy?.started} fallback="—" />}
              hint={latestDeploy ? latestDeploy.app : "no app activity"}
              icon={Rocket}
            >
              <div className="mt-2 flex items-center gap-1.5 text-2xs text-muted-foreground">
                {latestDeploy ? (
                  <>
                    <RunStatusPill status={latestDeploy.status} />
                    <span className="truncate">{plural(latestDeploy.steps, "step")}</span>
                  </>
                ) : (
                  <span>Deploy a release to see activity here</span>
                )}
              </div>
            </StatTile>
          </div>

          <div className="grid gap-5 xl:grid-cols-[minmax(0,1fr)_312px]">
            <div className="min-w-0 overflow-hidden rounded-xl border border-border bg-card shadow-[var(--shadow-card)]">
              <div className="flex flex-col gap-3 border-b border-border/60 px-4 py-3 lg:flex-row lg:items-center lg:justify-between">
                <SectionHeading
                  title="Applications"
                  description={`${plural(apps.length, "app")} in the registry`}
                />
                <div className="flex flex-wrap items-center gap-2">
                  <SearchInput
                    value={search}
                    onChange={setSearch}
                    placeholder="Filter apps…"
                    ariaLabel="Filter applications"
                    className="w-full sm:w-44"
                  />
                  <FilterPills
                    ariaLabel="Filter applications"
                    value={filter}
                    onChange={setFilter}
                    options={[
                      { value: "all", label: "All", count: apps.length },
                      { value: "live", label: "Live", count: liveApps.length },
                      { value: "attention", label: "Attention", count: attention.length },
                      {
                        value: "unmanaged",
                        label: "Unmanaged",
                        count: unmanagedApps,
                      },
                      { value: "idle", label: "Idle", count: idleApps },
                    ]}
                  />
                </div>
              </div>

              <div className="hidden items-center gap-3 border-b border-border/60 px-4 py-1.5 md:flex">
                <span className="label-eyebrow flex-1">Application</span>
                <span className="label-eyebrow w-[104px]">Health</span>
                <span className="label-eyebrow hidden w-[148px] lg:block">Release</span>
                <span className="label-eyebrow hidden w-[126px] xl:block">Last activity</span>
                <span className="w-3.5" />
              </div>

              <div>
                {filteredApps.length === 0 ? (
                  <div className="px-4 py-10 text-center">
                    <p className="text-sm text-foreground">No apps match</p>
                    <p className="mt-1 text-xs text-muted-foreground">
                      {search ? `Nothing matches “${search}”.` : "Nothing is in this state right now."}
                    </p>
                    <Button
                      variant="outline"
                      size="sm"
                      className="mt-3"
                      onClick={() => {
                        setSearch("")
                        setFilter("all")
                      }}
                    >
                      Clear filters
                    </Button>
                  </div>
                ) : (
                  filteredApps.map((entry) => (
                    <AppRow
                      key={entry.app.name}
                      entry={entry}
                      runs={runsByApp.get(entry.app.name) ?? []}
                      latest={latestByName.get(entry.app.name)}
                    />
                  ))
                )}
              </div>

              <div className="flex items-center justify-between border-t border-border/60 bg-muted/20 px-4 py-2">
                <span className="text-2xs text-subtle-foreground">
                  Health probes refresh every 20 seconds
                </span>
                <span className="text-2xs text-subtle-foreground">{filteredApps.length} shown</span>
              </div>
            </div>

            <aside className="flex min-w-0 flex-col gap-4">
              {attention.length > 0 ? (
                <Callout
                  tone={(errors ?? 0) > 0 ? "danger" : "warning"}
                  title={`${plural(attention.length, "app")} need${attention.length === 1 ? "s" : ""} a look`}
                  action={
                    <Button
                      variant="ghost"
                      size="xs"
                      className="text-current"
                      onClick={() => setFilter("attention")}
                    >
                      Show
                    </Button>
                  }
                >
                  {attention
                    .slice(0, 3)
                    .map((entry) => entry.app.name)
                    .join(", ")}
                  {attention.length > 3 ? ` and ${attention.length - 3} more` : ""} have registry
                  findings or a failed probe. Open an app to see the details.
                </Callout>
              ) : null}
              <DoctorSummary
                problems={doctorQuery.data?.problems ?? []}
                loading={doctorQuery.isPending}
                error={doctorQuery.isError}
              />
              <RecentRuns runs={runs.slice(0, 5)} loading={runsQuery.isPending} />
            </aside>
          </div>
        </>
      )}
    </div>
  )
}

function OverviewSkeleton() {
  return (
    <div className="space-y-5">
      <div className="grid grid-cols-2 gap-3 xl:grid-cols-4">
        {[0, 1, 2, 3].map((index) => (
          <div key={index} className="rounded-xl border border-border bg-card p-4">
            <Skeleton className="h-2.5 w-16" />
            <Skeleton className="mt-4 h-7 w-12" />
            <Skeleton className="mt-2 h-2.5 w-20" />
          </div>
        ))}
      </div>
      <div className="grid gap-5 xl:grid-cols-[minmax(0,1fr)_312px]">
        <div className="overflow-hidden rounded-xl border border-border bg-card">
          <div className="border-b border-border/60 px-4 py-3">
            <Skeleton className="h-4 w-28" />
          </div>
          {[0, 1, 2, 3, 4, 5].map((index) => (
            <div
              key={index}
              className="flex items-center gap-3 border-b border-border/60 px-4 py-3 last:border-0"
            >
              <Skeleton className="size-7 rounded-[7px]" />
              <div className="flex-1 space-y-1.5">
                <Skeleton className="h-3 w-24" />
                <Skeleton className="h-2.5 w-40" />
              </div>
              <Skeleton className="h-5 w-16 rounded-full" />
            </div>
          ))}
        </div>
        <div className="space-y-4">
          <div className="rounded-xl border border-border bg-card p-4">
            <Skeleton className="h-4 w-24" />
            <Skeleton className="mt-4 h-7 w-24" />
            <Skeleton className="mt-3 h-3 w-full" />
            <Skeleton className="mt-2 h-3 w-4/5" />
          </div>
        </div>
      </div>
    </div>
  )
}
