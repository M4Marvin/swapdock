import { useMemo, useState } from "react"
import { createFileRoute, Link } from "@tanstack/react-router"
import { useQuery } from "@tanstack/react-query"
import {
  ChevronDown,
  ChevronRight,
  RefreshCw,
  ScrollText,
  Search,
  WifiOff,
} from "lucide-react"
import { api, classifyStatus, type RunSummary } from "@/api"
import { queryKeys } from "@/lib/query-keys"
import { formatDay, plural, shortId } from "@/lib/format"
import { useDocumentTitle } from "@/lib/hooks"
import { cn } from "cn"
import { PageHeader } from "@/components/PageHeader"
import { AppGlyph } from "@/components/AppGlyph"
import { RunStatusPill, StatusDot, runTone } from "@/components/Status"
import {
  Callout,
  EmptyState,
  FilterPills,
  RelativeTime,
  SearchInput,
} from "@/components/kit"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"

export const Route = createFileRoute("/runs/")({
  validateSearch: (search: Record<string, unknown>) =>
    typeof search.app === "string" && search.app !== "" ? { app: search.app } : {},
  component: RunsPage,
})

type StatusFilter = "all" | "ok" | "failed" | "running" | "dry"

function groupByDay(runs: RunSummary[]) {
  const groups: { key: string; label: string; runs: RunSummary[] }[] = []
  for (const run of runs) {
    const date = new Date(run.started)
    const key = Number.isNaN(date.getTime())
      ? "unknown"
      : `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`
    const label = formatDay(date)
    const last = groups[groups.length - 1]
    if (last && last.key === key) last.runs.push(run)
    else groups.push({ key, label, runs: [run] })
  }
  return groups
}

function RunsPage() {
  useDocumentTitle("Runs")
  const search = Route.useSearch()
  const navigate = Route.useNavigate()
  const [status, setStatus] = useState<StatusFilter>("all")
  const [query, setQuery] = useState("")

  const runsQuery = useQuery({
    queryKey: queryKeys.runs,
    queryFn: ({ signal }) => api.runs(50, signal),
    refetchInterval: 30_000,
    placeholderData: (previous) => previous,
  })
  const appsQuery = useQuery({
    queryKey: queryKeys.apps,
    queryFn: ({ signal }) => api.apps(signal),
    placeholderData: (previous) => previous,
  })

  const runs = useMemo(() => runsQuery.data ?? [], [runsQuery.data])
  const runsUnavailable = runsQuery.isError && runsQuery.data == null
  const apps = useMemo(() => appsQuery.data ?? [], [appsQuery.data])
  const appFilter = search.app ?? "all"

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase()
    return runs.filter((run) => {
      if (appFilter !== "all" && run.app !== appFilter) return false
      const outcome = classifyStatus(run.status)
      if (status === "ok" && outcome !== "ok") return false
      if (status === "failed" && outcome !== "failed" && outcome !== "interrupted") return false
      if (status === "running" && outcome !== "running") return false
      if (status === "dry" && outcome !== "dry") return false
      if (!needle) return true
      return (
        run.run_id.toLowerCase().includes(needle) ||
        (run.app ?? "registry").toLowerCase().includes(needle)
      )
    })
  }, [runs, appFilter, status, query])

  const counts = useMemo(() => {
    const tally = { ok: 0, failed: 0, running: 0, dry: 0 }
    for (const run of runs) {
      const outcome = classifyStatus(run.status)
      if (outcome === "ok") tally.ok += 1
      else if (outcome === "failed" || outcome === "interrupted") tally.failed += 1
      else if (outcome === "running") tally.running += 1
      else if (outcome === "dry") tally.dry += 1
    }
    return tally
  }, [runs])

  const groups = useMemo(() => groupByDay(filtered), [filtered])
  const filteredCounts = useMemo(() => {
    const tally = { ok: 0, failed: 0, running: 0, dry: 0 }
    for (const run of filtered) {
      const outcome = classifyStatus(run.status)
      if (outcome === "ok") tally.ok += 1
      else if (outcome === "failed" || outcome === "interrupted") tally.failed += 1
      else if (outcome === "running") tally.running += 1
      else if (outcome === "dry") tally.dry += 1
    }
    return tally
  }, [filtered])

  const hasFilters = appFilter !== "all" || status !== "all" || query.trim() !== ""

  return (
    <div className="space-y-5">
      <PageHeader
        eyebrow="Audit"
        title="Runs"
        description="The append-only ledger every swapdock command writes to. Each run keeps its argv, exit codes and timings."
        actions={
          <Button
            variant="ghost"
            size="icon-sm"
            className="hidden sm:inline-flex"
            aria-label="Refresh runs"
            title="Refresh runs"
            disabled={runsQuery.isFetching}
            onClick={() => void runsQuery.refetch()}
          >
            <RefreshCw className={runsQuery.isFetching ? "animate-spin" : undefined} />
          </Button>
        }
      />

      {runsQuery.isError ? (
        <Callout
          tone="danger"
          title="The run log could not be read"
          action={
            <Button variant="outline" size="sm" onClick={() => void runsQuery.refetch()}>
              Retry
            </Button>
          }
        >
          {runsQuery.error instanceof Error ? runsQuery.error.message : String(runsQuery.error)}
        </Callout>
      ) : null}

      <section className="overflow-hidden rounded-xl border border-border bg-card shadow-[var(--shadow-card)]">
        <div className="flex flex-col gap-3 border-b border-border/60 px-4 py-3 lg:flex-row lg:items-center lg:justify-between">
          <div className="flex items-center gap-2.5">
            <span className="flex size-7 items-center justify-center rounded-md border border-border bg-surface text-subtle-foreground">
              <ScrollText className="size-3.5" />
            </span>
            <div>
              <h2 className="text-sm font-medium tracking-[-0.01em]">Run ledger</h2>
              <p className="text-2xs text-muted-foreground">Newest first · refreshed every 30s</p>
            </div>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            <SearchInput
              value={query}
              onChange={setQuery}
              placeholder="Search a run or app…"
              ariaLabel="Search runs"
              className="w-full sm:w-52"
            />

            <div className="relative">
              <select
                aria-label="Filter by app"
                value={appFilter}
                onChange={(event) => {
                  const next = event.target.value
                  void navigate({
                    to: "/runs",
                    search: next === "all" ? {} : { app: next },
                  })
                }}
                className="h-8 min-w-[130px] appearance-none rounded-lg border border-border bg-surface pr-7 pl-2.5 text-xs text-foreground outline-none transition-colors hover:border-border-strong focus-visible:ring-[3px] focus-visible:ring-ring/40"
              >
                <option value="all">All apps</option>
                {apps.map((entry) => (
                  <option key={entry.app.name} value={entry.app.name}>
                    {entry.app.name}
                  </option>
                ))}
              </select>
              <ChevronDown className="pointer-events-none absolute top-1/2 right-2 size-3.5 -translate-y-1/2 text-subtle-foreground" />
            </div>

            <FilterPills
              ariaLabel="Filter by status"
              value={status}
              onChange={setStatus}
              options={[
                { value: "all", label: "All", count: runsUnavailable ? undefined : runs.length },
                { value: "ok", label: "Ok", count: runsUnavailable ? undefined : counts.ok },
                { value: "failed", label: "Failed", count: runsUnavailable ? undefined : counts.failed },
                { value: "running", label: "Running", count: runsUnavailable ? undefined : counts.running },
              ]}
            />

            {hasFilters ? (
              <Button
                variant="ghost"
                size="xs"
                onClick={() => {
                  setQuery("")
                  setStatus("all")
                  void navigate({ to: "/runs", search: {} })
                }}
              >
                Clear
              </Button>
            ) : null}
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-x-4 gap-y-1 border-b border-border/60 bg-muted/20 px-4 py-2 text-2xs text-muted-foreground">
          <span>
            {runsUnavailable
              ? "Run log unavailable"
              : `${filtered.length} of ${runs.length} runs`}
          </span>
          <span className="flex items-center gap-1.5">
            <StatusDot tone="success" />
            {filteredCounts.ok} ok
          </span>
          {filteredCounts.failed > 0 ? (
            <span className="flex items-center gap-1.5 text-destructive-strong">
              <StatusDot tone="danger" />
              {filteredCounts.failed} failed
            </span>
          ) : null}
          {filteredCounts.running > 0 ? (
            <span className="flex items-center gap-1.5 text-info">
              <StatusDot tone="info" pulse />
              {filteredCounts.running} running
            </span>
          ) : null}
          {filteredCounts.dry > 0 ? (
            <span className="flex items-center gap-1.5">
              <StatusDot tone="info" />
              {filteredCounts.dry} dry
            </span>
          ) : null}
          <span className="ml-auto hidden sm:inline">Times shown in your local zone</span>
        </div>

        {runsQuery.isPending ? (
          <div>
            {[0, 1, 2, 3, 4, 5, 6].map((index) => (
              <div
                key={index}
                className="flex items-center gap-3 border-b border-border/60 px-4 py-3 last:border-0"
              >
                <Skeleton className="h-5 w-24 rounded-full" />
                <Skeleton className="size-5 rounded-[5px]" />
                <Skeleton className="h-3 w-28" />
                <Skeleton className="ml-auto h-3 w-16" />
              </div>
            ))}
          </div>
        ) : runsUnavailable ? (
          <EmptyState
            icon={WifiOff}
            title="The run log could not be read"
            description={`${runsQuery.error instanceof Error ? runsQuery.error.message : String(runsQuery.error)}. Nothing is shown because the request failed — this is not an empty ledger.`}
            action={
              <Button variant="outline" size="sm" onClick={() => void runsQuery.refetch()}>
                <RefreshCw />
                Retry run log
              </Button>
            }
            className="rounded-none border-0 bg-transparent py-14 shadow-none"
          />
        ) : runs.length === 0 ? (
          <EmptyState
            icon={ScrollText}
            title="No runs recorded yet"
            description="Deploy an app or run a check and its trace will appear here, with every step and timing."
            className="rounded-none border-0 bg-transparent py-14 shadow-none"
          />
        ) : filtered.length === 0 ? (
          <div className="flex flex-col items-center gap-2 px-4 py-14 text-center">
            <Search className="size-4 text-subtle-foreground" />
            <p className="text-sm text-foreground">No runs match</p>
            <p className="text-xs text-muted-foreground">
              Loosen the app, status or search filters.
            </p>
            <Button
              variant="outline"
              size="sm"
              className="mt-2"
              onClick={() => {
                setQuery("")
                setStatus("all")
                void navigate({ to: "/runs", search: {} })
              }}
            >
              Clear filters
            </Button>
          </div>
        ) : (
          groups.map((group) => (
            <div key={group.key}>
              <div className="sticky top-14 z-10 flex items-center gap-2 border-b border-border/60 bg-[var(--header)] px-4 py-1.5 backdrop-blur-xl">
                <span className="label-eyebrow">{group.label}</span>
                <span className="text-2xs text-subtle-foreground">
                  {plural(group.runs.length, "run")}
                </span>
              </div>
              {group.runs.map((run) => {
                const running = classifyStatus(run.status) === "running"
                return (
                  <Link
                    key={run.run_id}
                    to="/runs/$runId"
                    params={{ runId: run.run_id }}
                    className="group row-hover flex items-center gap-3 border-b border-border/60 px-4 py-3 last:border-0"
                  >
                    <span className="w-[104px] shrink-0">
                      <RunStatusPill status={run.status} />
                    </span>

                    <span className="flex w-[142px] shrink-0 items-center gap-2">
                      {run.app ? (
                        <AppGlyph name={run.app} size="xs" />
                      ) : (
                        <span className="flex size-5 items-center justify-center rounded-[5px] border border-border bg-surface text-subtle-foreground">
                          <ScrollText className="size-2.5" />
                        </span>
                      )}
                      <span className="truncate text-xs text-foreground">
                        {run.app ?? "Registry"}
                      </span>
                    </span>

                    <span className="min-w-0 flex-1">
                      <span className="flex items-center gap-2">
                        <span className="truncate font-mono text-xs text-muted-foreground" title={run.run_id}>
                          {shortId(run.run_id)}
                        </span>
                        <RelativeTime
                          value={run.started}
                          className="text-2xs text-subtle-foreground md:hidden"
                        />
                        {run.non_ok > 0 ? (
                          <span className="rounded bg-warning/10 px-1 py-0.5 text-[11px] text-warning-strong">
                            {run.non_ok} non-ok
                          </span>
                        ) : null}
                      </span>
                      {run.error ? (
                        <span
                          className="mt-0.5 block truncate text-2xs text-destructive-strong"
                          title={run.error}
                        >
                          {run.error}
                        </span>
                      ) : null}
                    </span>

                    <span className="hidden w-[92px] shrink-0 md:block">
                      <RelativeTime value={run.started} className="text-xs text-muted-foreground" />
                    </span>
                    <span className="hidden w-[64px] shrink-0 items-center gap-1.5 sm:flex">
                      <StatusDot tone={runTone(run.status)} pulse={running} />
                      <span
                        className={cn(
                          "tabular text-2xs",
                          running ? "text-info" : "text-muted-foreground"
                        )}
                      >
                        {running ? "live" : plural(run.steps, "step")}
                      </span>
                    </span>
                    <ChevronRight className="size-3.5 shrink-0 text-subtle-foreground/50 transition-[color,transform] duration-150 group-hover:translate-x-0.5 group-hover:text-muted-foreground" />
                  </Link>
                )
              })}
            </div>
          ))
        )}
      </section>
    </div>
  )
}
