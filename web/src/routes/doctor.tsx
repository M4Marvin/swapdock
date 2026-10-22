import { useMemo, useState } from "react"
import { createFileRoute, Link } from "@tanstack/react-router"
import { useQuery } from "@tanstack/react-query"
import {
  ArrowUpRight,
  Globe2,
  Layers,
  RefreshCw,
  Search,
  ShieldCheck,
  Stethoscope,
  TriangleAlert,
  WifiOff,
} from "lucide-react"
import { api, type Problem } from "@/api"
import { queryKeys } from "@/lib/query-keys"
import { plural } from "@/lib/format"
import { useDocumentTitle } from "@/lib/hooks"
import { cn } from "cn"
import { PageHeader } from "@/components/PageHeader"
import { AppGlyph } from "@/components/AppGlyph"
import { StatusDot } from "@/components/Status"
import {
  EmptyState,
  FilterPills,
  RelativeTime,
  SearchInput,
  StatTile,
} from "@/components/kit"
import { Button } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
import { Skeleton } from "@/components/ui/skeleton"

export const Route = createFileRoute("/doctor")({
  component: DoctorPage,
})

type SeverityFilter = "all" | "errors" | "warnings"

function groupProblems(problems: Problem[]) {
  const groups = new Map<string, Problem[]>()
  for (const problem of problems) {
    const key = problem.app ?? "__estate__"
    const list = groups.get(key) ?? []
    list.push(problem)
    groups.set(key, list)
  }
  return [...groups.entries()]
    .map(([key, items]) => ({
      key,
      name: key === "__estate__" ? "Estate" : key,
      problems: items,
      errors: items.filter((problem) => problem.severity === "error").length,
      warnings: items.filter((problem) => problem.severity === "warning").length,
    }))
    .sort((a, b) => {
      if (a.errors !== b.errors) return b.errors - a.errors
      if (a.warnings !== b.warnings) return b.warnings - a.warnings
      if (a.key === "__estate__") return -1
      if (b.key === "__estate__") return 1
      return a.name.localeCompare(b.name)
    })
}

function DoctorPage() {
  useDocumentTitle("Doctor")
  const [publicUrls, setPublicUrls] = useState(false)
  const [severity, setSeverity] = useState<SeverityFilter>("all")
  const [query, setQuery] = useState("")

  const doctorQuery = useQuery({
    queryKey: [...queryKeys.doctor, publicUrls],
    queryFn: ({ signal }) => api.doctorAll(publicUrls, signal),
    staleTime: 30_000,
    retry: 0,
  })

  const problems = useMemo(() => doctorQuery.data?.problems ?? [], [doctorQuery.data])
  const errors = problems.filter((problem) => problem.severity === "error").length
  const warnings = problems.length - errors
  const affectedApps = new Set(problems.map((problem) => problem.app).filter(Boolean)).size
  const unavailableHint = doctorQuery.isError ? "check unavailable" : "checking…"
  const counted = doctorQuery.isSuccess

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase()
    return problems.filter((problem) => {
      if (severity === "errors" && problem.severity !== "error") return false
      if (severity === "warnings" && problem.severity !== "warning") return false
      if (!needle) return true
      return (
        problem.message.toLowerCase().includes(needle) ||
        problem.code.toLowerCase().includes(needle) ||
        (problem.app ?? "estate").toLowerCase().includes(needle)
      )
    })
  }, [problems, severity, query])

  const groups = useMemo(() => groupProblems(filtered), [filtered])
  const hasFilters = severity !== "all" || query.trim() !== ""
  const filteredErrors = filtered.filter((problem) => problem.severity === "error").length
  const filteredWarnings = filtered.length - filteredErrors

  return (
    <div className="space-y-5">
      <PageHeader
        eyebrow="Diagnostics"
        title="Doctor"
        description="Host, compose, port, mount, image and healthcheck checks across the whole estate. Read-only — nothing here changes a container."
        actions={
          <>
            <div className="flex h-8 items-center gap-2 rounded-lg border border-border bg-surface px-2.5 text-xs text-muted-foreground transition-colors hover:border-border-strong">
              <Switch
                id="probe-public-urls"
                size="sm"
                checked={publicUrls}
                onCheckedChange={(checked) => setPublicUrls(checked === true)}
                aria-label="Probe public URLs"
              />
              <label htmlFor="probe-public-urls" className="cursor-pointer select-none">
                Probe public URLs
              </label>
            </div>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="Refresh doctor"
              title="Refresh doctor"
              disabled={doctorQuery.isFetching}
              onClick={() => void doctorQuery.refetch()}
            >
              <RefreshCw className={doctorQuery.isFetching ? "animate-spin" : undefined} />
            </Button>
          </>
        }
      />

      <div className="grid grid-cols-3 gap-3">
        <StatTile
          label="Errors"
          value={doctorQuery.isSuccess ? errors : "—"}
          hint={
            !doctorQuery.isSuccess
              ? unavailableHint
              : errors === 0
                ? "nothing blocking"
                : "blocks deploys"
          }
          icon={TriangleAlert}
          tone={doctorQuery.isSuccess && errors > 0 ? "danger" : "neutral"}
        />
        <StatTile
          label="Warnings"
          value={doctorQuery.isSuccess ? warnings : "—"}
          hint={doctorQuery.isSuccess ? "worth a look" : unavailableHint}
          icon={Stethoscope}
          tone={doctorQuery.isSuccess && warnings > 0 ? "warning" : "neutral"}
        />
        <StatTile
          label="Apps affected"
          value={doctorQuery.isSuccess ? affectedApps : "—"}
          hint={
            doctorQuery.isSuccess
              ? affectedApps === 0
                ? "clean estate"
                : "with findings"
              : unavailableHint
          }
          icon={Layers}
          tone="neutral"
        />
      </div>

      <section className="overflow-hidden rounded-xl border border-border bg-card shadow-[var(--shadow-card)]">
        <div className="flex flex-col gap-3 border-b border-border/60 px-4 py-3 lg:flex-row lg:items-center lg:justify-between">
          <div>
            <h2 className="text-sm font-medium tracking-[-0.01em]">Findings</h2>
            <p className="text-2xs text-muted-foreground">
              {doctorQuery.isFetching
                ? "Checking the host…"
                : doctorQuery.data
                  ? "Grouped by application · read-only"
                  : "Reading the estate"}
              {doctorQuery.dataUpdatedAt > 0 ? (
                <span className="ml-1">
                  · checked <RelativeTime value={doctorQuery.dataUpdatedAt} />
                </span>
              ) : null}
            </p>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <SearchInput
              value={query}
              onChange={setQuery}
              placeholder="Search findings…"
              ariaLabel="Search findings"
              className="w-full sm:w-52"
            />
            <FilterPills
              ariaLabel="Filter by severity"
              value={severity}
              onChange={setSeverity}
              options={[
                { value: "all", label: "All", count: counted ? problems.length : undefined },
                { value: "errors", label: "Errors", count: counted ? errors : undefined },
                { value: "warnings", label: "Warnings", count: counted ? warnings : undefined },
              ]}
            />
            {hasFilters ? (
              <Button
                variant="ghost"
                size="xs"
                onClick={() => {
                  setQuery("")
                  setSeverity("all")
                }}
              >
                Clear
              </Button>
            ) : null}
          </div>
        </div>

        {doctorQuery.isPending ? (
          <div className="space-y-2 p-4">
            {[0, 1, 2, 3, 4].map((index) => (
              <div key={index} className="flex items-center gap-3 py-1.5">
                <Skeleton className="size-1.5 rounded-full" />
                <Skeleton className="h-3 flex-1" />
                <Skeleton className="h-3 w-20" />
              </div>
            ))}
          </div>
        ) : doctorQuery.isError ? (
          <EmptyState
            icon={WifiOff}
            title="The host checks did not run"
            description={`${
              doctorQuery.error instanceof Error
                ? doctorQuery.error.message
                : String(doctorQuery.error)
            }. No findings are shown because this request did not complete — this is not a clean estate.`}
            action={
              <Button variant="outline" size="sm" onClick={() => void doctorQuery.refetch()}>
                <RefreshCw />
                Retry host checks
              </Button>
            }
            className="rounded-none border-0 bg-transparent py-12 shadow-none"
          />
        ) : problems.length === 0 ? (
          <EmptyState
            icon={ShieldCheck}
            title="Everything checks out"
            description="No host, compose, port, mount or healthcheck findings in this pass. A clean estate is the normal state; this page stays quiet."
            className="rounded-none border-0 bg-transparent py-12 shadow-none"
          />
        ) : filtered.length === 0 ? (
          <div className="flex flex-col items-center gap-2 px-4 py-12 text-center">
            <Search className="size-4 text-subtle-foreground" />
            <p className="text-sm text-foreground">No findings match</p>
            <p className="text-xs text-muted-foreground">
              The estate has {plural(problems.length, "finding")}; loosen the filter to see
              them.
            </p>
            <Button
              variant="outline"
              size="sm"
              className="mt-2"
              onClick={() => {
                setQuery("")
                setSeverity("all")
              }}
            >
              Clear filters
            </Button>
          </div>
        ) : (
          <>
            <div className="flex flex-wrap items-center gap-x-4 gap-y-1 border-b border-border/60 bg-muted/20 px-4 py-2 text-2xs text-muted-foreground">
              <span>
                {filtered.length} of {problems.length} findings
              </span>
              {filteredErrors > 0 ? (
                <span className="flex items-center gap-1.5 text-destructive-strong">
                  <StatusDot tone="danger" />
                  {filteredErrors} errors
                </span>
              ) : null}
              {filteredWarnings > 0 ? (
                <span className="flex items-center gap-1.5 text-warning-strong">
                  <StatusDot tone="warning" />
                  {filteredWarnings} warnings
                </span>
              ) : null}
              <span className="ml-auto hidden items-center gap-1.5 sm:flex">
                <Globe2 className="size-3" />
                {publicUrls ? "public URLs probed" : "host checks only"}
              </span>
            </div>

            {groups.map((group) => (
              <div key={group.key} className="border-b border-border/60 last:border-0">
                <div className="flex items-center gap-2.5 border-b border-border/60 bg-muted/15 px-4 py-2">
                  {group.key === "__estate__" ? (
                    <span className="flex size-5 items-center justify-center rounded-[5px] border border-border bg-surface text-subtle-foreground">
                      <Layers className="size-2.5" />
                    </span>
                  ) : (
                    <AppGlyph name={group.name} size="xs" />
                  )}
                  <span className="text-xs font-medium text-foreground">
                    {group.key === "__estate__" ? "Estate" : group.name}
                  </span>
                  {group.errors > 0 ? (
                    <span className="text-2xs text-destructive-strong">{group.errors} errors</span>
                  ) : null}
                  <span className="text-2xs text-subtle-foreground">
                    {plural(group.problems.length, "finding")}
                  </span>
                  {group.key !== "__estate__" ? (
                    <Link
                      to="/apps/$appName"
                      params={{ appName: group.name }}
                      search={{ tab: "operations" }}
                      className="ml-auto inline-flex items-center gap-1 text-2xs text-muted-foreground transition-colors hover:text-foreground"
                    >
                      Open app
                      <ArrowUpRight className="size-2.5" />
                    </Link>
                  ) : null}
                </div>
                {group.problems.map((problem, index) => (
                  <div
                    key={`${problem.code}-${index}`}
                    className="flex items-start gap-3 border-b border-border/40 px-4 py-2.5 last:border-0"
                  >
                    <span
                      className={cn(
                        "mt-1.5 size-1.5 shrink-0 rounded-full",
                        problem.severity === "error" ? "bg-destructive" : "bg-warning"
                      )}
                    />
                    <div className="min-w-0 flex-1">
                      <p className="text-pretty text-xs leading-relaxed text-muted-foreground">
                        {problem.message}
                      </p>
                      <span className="mt-0.5 inline-flex items-center gap-1.5 font-mono text-[10px] text-subtle-foreground">
                        {problem.code}
                        <span className="text-subtle-foreground/40">·</span>
                        {problem.severity}
                      </span>
                    </div>
                  </div>
                ))}
              </div>
            ))}
          </>
        )}
      </section>
    </div>
  )
}
