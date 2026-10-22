import { Link } from "@tanstack/react-router"
import { ChevronRight, ScrollText } from "lucide-react"
import type { RunSummary } from "@/api"
import { classifyStatus } from "@/api"
import { plural, shortId } from "@/lib/format"
import { cn } from "cn"
import { RelativeTime } from "@/components/kit"
import { Skeleton } from "@/components/ui/skeleton"
import { RunStatusPill, StatusDot, runTone } from "@/components/Status"
import { Button } from "@/components/ui/button"

export function RecentRunsCard({
  appName,
  runs,
  loading = false,
}: {
  appName: string
  runs: RunSummary[]
  loading?: boolean
}) {
  return (
    <section className="overflow-hidden rounded-xl border border-border bg-card shadow-[var(--shadow-card)] xl:col-span-3">
      <div className="flex items-start justify-between gap-3 border-b border-border/60 px-4 py-3.5">
        <div>
          <h2 className="text-sm font-medium tracking-[-0.01em]">Recent runs</h2>
          <p className="text-2xs text-muted-foreground">
            Deploys, builds, syncs and operator commands for {appName}
          </p>
        </div>
        <Button variant="ghost" size="xs" render={<Link to="/runs" search={{ app: appName }} />}>
          All runs
          <ChevronRight />
        </Button>
      </div>

      {loading && runs.length === 0 ? (
        <div className="space-y-2 p-4">
          {[0, 1, 2].map((index) => (
            <div key={index} className="flex items-center gap-3">
              <Skeleton className="h-5 w-[86px] rounded-full" />
              <Skeleton className="h-3 flex-1" />
              <Skeleton className="h-3 w-14" />
            </div>
          ))}
        </div>
      ) : runs.length === 0 ? (
        <div className="flex flex-col items-center gap-2 px-4 py-10 text-center">
          <ScrollText className="size-4 text-subtle-foreground" />
          <p className="text-sm text-foreground">No runs recorded for this app</p>
          <p className="max-w-sm text-xs text-muted-foreground">
            Start a deploy or a check from the Pipeline or Operations tab and it will
            appear here, with every step.
          </p>
        </div>
      ) : (
        <div>
          <div className="hidden items-center gap-3 border-b border-border/60 px-4 py-1.5 sm:flex">
            <span className="label-eyebrow w-[106px]">Status</span>
            <span className="label-eyebrow flex-1">Run</span>
            <span className="label-eyebrow w-[86px]">Started</span>
            <span className="label-eyebrow w-[64px] text-right">Steps</span>
            <span className="w-3.5" />
          </div>
          {runs.map((run) => {
            const running = classifyStatus(run.status) === "running"
            return (
              <Link
                key={run.run_id}
                to="/runs/$runId"
                params={{ runId: run.run_id }}
                className="group row-hover flex items-center gap-3 border-b border-border/60 px-4 py-2.5 last:border-0"
              >
                <span className="flex w-[106px] shrink-0 items-center gap-2">
                  <RunStatusPill status={run.status} />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="flex items-center gap-2">
                    <span
                      className="truncate font-mono text-xs text-foreground"
                      title={run.run_id}
                    >
                      {shortId(run.run_id)}
                    </span>
                    <RelativeTime
                      value={run.started}
                      className="text-2xs text-subtle-foreground sm:hidden"
                    />
                  </span>
                  <span className="mt-0.5 flex items-center gap-1.5 text-2xs text-subtle-foreground">
                    <span>{running ? "in progress" : plural(run.steps, "step")}</span>
                    {run.non_ok > 0 ? (
                      <>
                        <span>·</span>
                        <span className="text-warning-strong">{run.non_ok} non-ok</span>
                      </>
                    ) : null}
                  </span>
                </span>
                <span className="hidden w-[86px] shrink-0 sm:block">
                  <RelativeTime value={run.started} className="text-2xs text-muted-foreground" />
                </span>
                <span className="hidden w-[64px] shrink-0 items-center justify-end gap-1.5 sm:flex">
                  <StatusDot tone={runTone(run.status)} pulse={running} />
                  <span className={cn("tabular text-2xs", running ? "text-info" : "text-muted-foreground")}>
                    {running ? "live" : run.steps}
                  </span>
                </span>
                <ChevronRight className="size-3.5 shrink-0 text-subtle-foreground/50 transition-[color,transform] duration-150 group-hover:translate-x-0.5 group-hover:text-muted-foreground" />
              </Link>
            )
          })}
        </div>
      )}
    </section>
  )
}
