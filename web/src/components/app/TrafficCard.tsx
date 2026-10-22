import { useMemo, useState } from "react"
import { useQuery } from "@tanstack/react-query"
import {
  ArrowRight,
  Check,
  Clock,
  RefreshCw,
  SlidersHorizontal,
  Waypoints,
} from "lucide-react"
import { api, type App, type RunSummary, type VerifyReport } from "@/api"
import { queryKeys } from "@/lib/query-keys"
import { formatClock, formatCount, formatExact, plural } from "@/lib/format"
import { cn } from "cn"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Skeleton } from "@/components/ui/skeleton"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Callout, EmptyState, SectionHeading } from "@/components/kit"

type Preset = "1h" | "24h" | "run" | "custom"

const HOUR = 3_600_000

function statusClass(code: string): string {
  if (code.startsWith("2")) return "bg-success/70"
  if (code.startsWith("3")) return "bg-info/60"
  if (code.startsWith("4")) return "bg-warning/60"
  if (code.startsWith("5")) return "bg-destructive/70"
  return "bg-muted-foreground/40"
}

function statusDot(code: string): string {
  if (code.startsWith("2")) return "bg-success"
  if (code.startsWith("3")) return "bg-info"
  if (code.startsWith("4")) return "bg-warning"
  if (code.startsWith("5")) return "bg-destructive"
  return "bg-muted-foreground"
}

function upstreamName(value: string): string {
  return value.replace(/^https?:\/\//, "").replace(/^127\.0\.0\.1:/, ":")
}

function TrafficStats({ report }: { report: VerifyReport }) {
  const entries = useMemo(
    () =>
      Object.entries(report.by_status)
        .map(([code, count]) => ({ code, count }))
        .sort((a, b) => Number(a.code) - Number(b.code)),
    [report.by_status]
  )
  const total = entries.reduce((sum, entry) => sum + entry.count, 0)
  const upstreams = useMemo(
    () =>
      Object.entries(report.upstreams).sort((a, b) => b[1].requests - a[1].requests),
    [report.upstreams]
  )
  const maxUpstream = Math.max(1, ...upstreams.map(([, info]) => info.requests))

  return (
    <div className="space-y-4">
      <div className="grid grid-cols-3 overflow-hidden rounded-lg border border-border bg-surface">
        <div className="px-3 py-2.5">
          <div className="label-eyebrow">Requests</div>
          <div className="mt-1 tabular text-lg font-semibold tracking-[-0.02em]">
            {formatCount(report.requests)}
          </div>
        </div>
        <div className="border-l border-border px-3 py-2.5">
          <div className="label-eyebrow">5xx</div>
          <div
            className={cn(
              "mt-1 tabular text-lg font-semibold tracking-[-0.02em]",
              report.errors_5xx > 0 ? "text-destructive-strong" : "text-foreground"
            )}
          >
            {formatCount(report.errors_5xx)}
          </div>
        </div>
        <div className="border-l border-border px-3 py-2.5">
          <div className="label-eyebrow">Malformed</div>
          <div className="mt-1 tabular text-lg font-semibold tracking-[-0.02em]">
            {formatCount(report.malformed)}
          </div>
        </div>
      </div>

      {total > 0 ? (
        <div className="space-y-2">
          <div className="flex h-2 w-full gap-px overflow-hidden rounded-full bg-muted">
            {entries.map((entry) => (
              <div
                key={entry.code}
                className={statusClass(entry.code)}
                style={{ width: `${(entry.count / total) * 100}%` }}
                title={`${entry.code}: ${entry.count}`}
              />
            ))}
          </div>
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
            {entries.slice(0, 8).map((entry) => (
              <span
                key={entry.code}
                className="inline-flex items-center gap-1.5 font-mono text-2xs text-muted-foreground"
              >
                <span className={cn("size-1.5 rounded-full", statusDot(entry.code))} />
                {entry.code}
                <span className="text-subtle-foreground">{entry.count}</span>
              </span>
            ))}
          </div>
        </div>
      ) : (
        <p className="text-xs text-muted-foreground">No responses recorded in this window.</p>
      )}

      {upstreams.length > 0 ? (
        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <span className="label-eyebrow">Upstreams</span>
            <span className="text-2xs text-subtle-foreground">
              {plural(report.lines, "line")} read
            </span>
          </div>
          <div className="space-y-2">
            {upstreams.map(([upstream, info]) => (
              <div key={upstream} className="space-y-1">
                <div className="flex items-center justify-between gap-3">
                  <span className="truncate font-mono text-xs text-foreground" title={upstream}>
                    {upstreamName(upstream)}
                  </span>
                  <span className="shrink-0 text-2xs text-muted-foreground">
                    {formatCount(info.requests)} req
                    {info.errors_5xx > 0 ? (
                      <span className="ml-1.5 text-destructive-strong">{info.errors_5xx} 5xx</span>
                    ) : null}
                  </span>
                </div>
                <div className="h-1 overflow-hidden rounded-full bg-muted">
                  <div
                    className={cn(
                      "h-full rounded-full",
                      info.errors_5xx > 0 ? "bg-warning/70" : "bg-success/60"
                    )}
                    style={{ width: `${Math.max(4, (info.requests / maxUpstream) * 100)}%` }}
                  />
                </div>
                <div className="flex items-center gap-1 text-2xs text-subtle-foreground">
                  <Clock className="size-2.5" />
                  <span>
                    first {formatClock(info.first_ms)} · last {formatClock(info.last_ms)}
                  </span>
                </div>
              </div>
            ))}
          </div>
        </div>
      ) : null}

      {report.flip ? (
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1 rounded-lg border border-accent/20 bg-accent-soft/40 px-3 py-2 text-xs">
          <Waypoints className="size-3.5 text-accent" />
          <span className="text-muted-foreground">Traffic flipped</span>
          <span className="font-mono text-foreground">{upstreamName(report.flip.from)}</span>
          <ArrowRight className="size-3 text-subtle-foreground" />
          <span className="font-mono text-foreground">{upstreamName(report.flip.to)}</span>
          <span className="ml-auto text-2xs text-subtle-foreground">
            {formatClock(report.flip.at_ms)}
          </span>
        </div>
      ) : (
        <div className="flex items-center gap-1.5 text-2xs text-subtle-foreground">
          <Check className="size-3" />
          No upstream flip in this window
        </div>
      )}

      <details className="group border-t border-border/60 pt-2">
        <summary className="cursor-pointer list-none text-2xs text-subtle-foreground transition-colors select-none hover:text-muted-foreground">
          Raw report
        </summary>
        <pre className="code-surface mt-2 max-h-56 overflow-auto rounded-lg border border-border p-3 font-mono text-2xs leading-relaxed">
          {JSON.stringify(report, null, 2)}
        </pre>
      </details>
    </div>
  )
}

/**
 * The nginx verdict for a window: requests, errors, status mix and the exact
 * instant traffic moved between upstreams. It defaults to the last hour, but
 * the operator can ask for a longer window or any run id.
 */
export function TrafficCard({ app, latestRun }: { app: App; latestRun?: RunSummary }) {
  const [preset, setPreset] = useState<Preset>("1h")
  const [since, setSince] = useState(() => new Date(Date.now() - HOUR).toISOString())
  const [customOpen, setCustomOpen] = useState(false)
  const [customValue, setCustomValue] = useState("")

  // A static app has no live port, but nginx still logs its hostnames.
  const canVerify =
    app.live_port != null || (app.kind === "static" && app.hostnames.length > 0)

  const query = useQuery({
    queryKey: queryKeys.verify(app.name, since),
    queryFn: ({ signal }) => api.verify(app.name, since, signal),
    enabled: canVerify && since !== "",
    retry: false,
    staleTime: 15_000,
  })

  const choose = (next: Preset) => {
    setPreset(next)
    if (next === "1h") setSince(new Date(Date.now() - HOUR).toISOString())
    else if (next === "24h") setSince(new Date(Date.now() - 24 * HOUR).toISOString())
    else if (next === "run" && latestRun) setSince(latestRun.run_id)
  }

  const windowLabel =
    preset === "1h"
      ? "Last hour"
      : preset === "24h"
        ? "Last 24 hours"
        : preset === "run"
          ? "Since last run"
          : "Custom window"

  return (
    <section className="overflow-hidden rounded-xl border border-border bg-card shadow-[var(--shadow-card)] xl:col-span-2">
      <div className="flex flex-wrap items-start justify-between gap-3 border-b border-border/60 px-4 py-3.5">
        <SectionHeading
          title="Traffic"
          description={`nginx access log · ${windowLabel}`}
        />
        <div className="flex items-center gap-1.5">
          <div className="inline-flex items-center rounded-md border border-border bg-surface p-0.5">
            {(
              [
                { value: "1h", label: "1h" },
                { value: "24h", label: "24h" },
                { value: "run", label: "Run" },
              ] as const
            ).map((option) => (
              <button
                key={option.value}
                type="button"
                disabled={option.value === "run" && !latestRun}
                onClick={() => choose(option.value)}
                title={
                  option.value === "run"
                    ? (latestRun ? `Since run ${latestRun.run_id}` : "No runs for this app")
                    : undefined
                }
                className={cn(
                  "h-6 rounded px-2 text-2xs font-medium transition-colors duration-150 disabled:opacity-40",
                  preset === option.value
                    ? "bg-muted text-foreground"
                    : "text-muted-foreground hover:text-foreground"
                )}
              >
                {option.label}
              </button>
            ))}
          </div>
          <Button
            type="button"
            variant="ghost"
            size="icon-xs"
            aria-label="Custom traffic window"
            title="Custom traffic window"
            onClick={() => {
              setCustomValue(since)
              setCustomOpen(true)
            }}
          >
            <SlidersHorizontal />
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="icon-xs"
            aria-label="Refresh traffic"
            title="Refresh traffic"
            disabled={!canVerify || query.isFetching}
            onClick={() => void query.refetch()}
          >
            <RefreshCw className={query.isFetching ? "animate-spin" : undefined} />
          </Button>
        </div>
      </div>

      <div className="p-4">
        {!canVerify ? (
          <EmptyState
            icon={Waypoints}
            title={app.kind === "static" ? "Traffic needs a hostname" : "Traffic starts at the first deploy"}
            description={
              app.kind === "static"
                ? "Verify reads nginx's access log by hostname. Add one to the registry to see traffic here."
                : "There is no live backend yet, so nginx has nothing to log for this app."
            }
            className="border-0 bg-transparent py-8 shadow-none"
          />
        ) : query.isPending ? (
          <div className="space-y-3">
            <div className="grid grid-cols-3 gap-0 overflow-hidden rounded-lg border border-border">
              {[0, 1, 2].map((index) => (
                <div key={index} className="space-y-2 border-l border-border px-3 py-2.5 first:border-l-0">
                  <Skeleton className="h-2.5 w-14" />
                  <Skeleton className="h-5 w-10" />
                </div>
              ))}
            </div>
            <Skeleton className="h-2 w-full rounded-full" />
            <Skeleton className="h-3 w-2/3" />
            <Skeleton className="h-3 w-1/2" />
          </div>
        ) : query.isError ? (
          <Callout
            tone="neutral"
            title="No access-log verdict for this window"
          >
            {query.error instanceof Error ? query.error.message : String(query.error)}. The
            app can still be healthy; this check reads nginx's access log on the host.
          </Callout>
        ) : query.data ? (
          <TrafficStats report={query.data} />
        ) : null}
      </div>

      <Dialog open={customOpen} onOpenChange={setCustomOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Traffic window</DialogTitle>
            <DialogDescription>
              Enter an RFC3339 timestamp or a run id. Verify reads the access log from
              that point forward.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-1.5">
            <label htmlFor="verify-since" className="text-xs font-medium text-muted-foreground">
              Since
            </label>
            <Input
              id="verify-since"
              value={customValue}
              onChange={(event) => setCustomValue(event.target.value)}
              placeholder="2026-10-21T18:00:00Z or a run id"
              className="font-mono"
              autoComplete="off"
              spellCheck={false}
            />
            <p className="text-2xs text-muted-foreground">
              {customValue && Date.parse(customValue)
                ? `Absolute: ${formatExact(customValue)}`
                : "A run id is the most precise window: it starts at the deploy."}
            </p>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setCustomOpen(false)}>
              Cancel
            </Button>
            <Button
              disabled={customValue.trim() === ""}
              onClick={() => {
                setSince(customValue.trim())
                setPreset("custom")
                setCustomOpen(false)
              }}
            >
              Read window
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  )
}
