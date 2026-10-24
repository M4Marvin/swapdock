import { useEffect, useMemo, useState } from "react"
import { createFileRoute, Link } from "@tanstack/react-router"
import { useQuery } from "@tanstack/react-query"
import {
  ArrowRight,
  ArrowUpRight,
  Check,
  ChevronDown,
  Clock,
  Radio,
  RefreshCw,
  ScrollText,
  Terminal,
  WifiOff,
} from "lucide-react"
import {
  ApiError,
  api,
  classifyStatus,
  formatStatus,
  openRunEvents,
  type RunStream,
  type TraceEvent,
} from "@/api"
import { queryKeys } from "@/lib/query-keys"
import {
  errorText,
  formatClock,
  formatDuration,
  formatExact,
  plural,
  shortId,
} from "@/lib/format"
import { useCopy, useDocumentTitle } from "@/lib/hooks"
import { cn } from "cn"
import { PageHeader } from "@/components/PageHeader"
import { AppGlyph } from "@/components/AppGlyph"
import { RunStatusPill, StatusDot, StatusPill, runTone } from "@/components/Status"
import { Callout, EmptyState, MetaList, RelativeTime } from "@/components/kit"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"

export const Route = createFileRoute("/runs/$runId")({
  component: RunDetailPage,
})

const IDLE_TIMEOUT_MS = 60_000

type Phase = "loading" | "streaming" | "reconnecting" | "ended" | "error"

/** Human verb for a run, taken from its argv; never the raw run id or SHA. */
function runAction(argv: string[] | undefined): string | null {
  const labels: Record<string, string> = {
    up: "Deploy",
    deploy: "Deploy",
    rollback: "Rollback",
    build: "Build",
    transfer: "Transfer",
    adopt: "Take over",
    backup: "Backup",
    prune: "Prune",
    sync: "Sync",
    doctor: "Host check",
    verify: "Traffic check",
    apply: "Apply config",
    validate: "Validate estate",
    init: "Create app",
  }
  for (const arg of argv ?? []) {
    const hit = labels[arg]
    if (hit) return hit
  }
  return null
}

function StepRow({
  step,
  index,
  last,
}: {
  step: TraceEvent
  index: number
  last: boolean
}) {
  const [open, setOpen] = useState(false)
  const failed =
    classifyStatus(step.status) === "failed" ||
    (step.exit_code != null && step.exit_code !== 0)
  const hasDetail =
    step.argv != null || step.detail != null || step.error != null || step.exit_code != null

  return (
    <li className="animate-fade-in relative">
      {!last ? (
        <span
          aria-hidden="true"
          className="absolute top-8 bottom-0 left-[27px] w-px bg-border"
        />
      ) : null}
      <button
        type="button"
        onClick={() => hasDetail && setOpen((value) => !value)}
        aria-expanded={hasDetail ? open : undefined}
        className={cn(
          "flex w-full items-start gap-3 px-4 py-2.5 text-left transition-colors duration-150",
          hasDetail && "hover:bg-muted/40"
        )}
      >
        <span className="relative z-10 mt-0.5 flex size-5 items-center justify-center rounded-full border border-border bg-card">
          <StatusDot tone={runTone(step.status)} />
        </span>
        <span className="min-w-0 flex-1">
          <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span className="font-mono text-[10px] text-subtle-foreground">
              {String(step.seq ?? index + 1).padStart(2, "0")}
            </span>
            <span
              className={cn(
                "text-sm font-medium",
                failed ? "text-destructive-strong" : "text-foreground"
              )}
            >
              {step.step ?? "step"}
            </span>
            <StatusPill
              tone={runTone(step.status)}
              label={step.status ? formatStatus(step.status) : "recorded"}
            />
            {step.exit_code != null && step.exit_code !== 0 ? (
              <span className="rounded bg-destructive/10 px-1 py-0.5 font-mono text-[11px] text-destructive-strong">
                exit {step.exit_code}
              </span>
            ) : null}
            <span className="ml-auto flex items-center gap-2 text-2xs text-subtle-foreground">
              {step.duration_ms != null ? (
                <span className="tabular font-mono">{formatDuration(step.duration_ms)}</span>
              ) : null}
              {step.ts ? <span>{formatClock(step.ts)}</span> : null}
              {hasDetail ? (
                <ChevronDown
                  className={cn(
                    "size-3 transition-transform duration-150",
                    open && "rotate-180"
                  )}
                />
              ) : null}
            </span>
          </span>
          {step.error ? (
            <span className="mt-1 block text-pretty text-2xs leading-relaxed text-destructive-strong">
              {step.error}
            </span>
          ) : null}
          {open ? (
            <span className="mt-2 block space-y-2 rounded-lg border border-border bg-surface p-2.5">
              {step.argv && step.argv.length > 0 ? (
                <span className="block">
                  <span className="label-eyebrow">argv</span>
                  <code className="mt-1 block overflow-x-auto font-mono text-[10px] whitespace-pre text-muted-foreground">
                    {step.argv.join(" ")}
                  </code>
                </span>
              ) : null}
              {step.detail != null ? (
                <span className="block">
                  <span className="label-eyebrow">detail</span>
                  <pre className="mt-1 max-h-48 overflow-auto font-mono text-[10px] text-muted-foreground">
                    {JSON.stringify(step.detail, null, 2)}
                  </pre>
                </span>
              ) : null}
              {step.exit_code != null ? (
                <span className="flex items-center gap-3 text-[10px] text-subtle-foreground">
                  <span>exit {step.exit_code}</span>
                  {step.duration_ms != null ? <span>{formatDuration(step.duration_ms)}</span> : null}
                  {step.ts ? <span>{formatExact(step.ts)}</span> : null}
                </span>
              ) : null}
              {!step.argv && step.detail == null && step.exit_code == null ? (
                <span className="text-[10px] text-subtle-foreground">
                  No extra detail was recorded for this step.
                </span>
              ) : null}
            </span>
          ) : null}
        </span>
      </button>
    </li>
  )
}

function RunDetailPage() {
  const { runId } = Route.useParams()
  const [events, setEvents] = useState<TraceEvent[]>([])
  const [ended, setEnded] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [phase, setPhase] = useState<Phase>("loading")
  const [attempt, setAttempt] = useState(0)
  const { copied, copy } = useCopy()

  const resumeQuery = useQuery({
    queryKey: queryKeys.run(runId),
    queryFn: ({ signal }) => api.resume(runId, "deploy", signal),
    retry: 0,
    staleTime: Infinity,
    refetchOnWindowFocus: false,
  })

  useEffect(() => {
    setEvents([])
    setEnded(null)
    setError(null)
    setPhase("loading")
    setAttempt(0)
  }, [runId])

  useEffect(() => {
    if (resumeQuery.status === "pending") return

    const controller = new AbortController()
    let stream: RunStream | null = null
    let idle: ReturnType<typeof setTimeout> | null = null

    const clearIdle = () => {
      if (idle) {
        clearTimeout(idle)
        idle = null
      }
    }
    const finish = (status: string | null) => {
      clearIdle()
      setEnded(status ?? "stream closed")
      setPhase("ended")
    }
    const armIdle = () => {
      clearIdle()
      idle = setTimeout(() => {
        stream?.close()
        setError("stalled: no events for 60s — the run may have been killed")
        setPhase("error")
      }, IDLE_TIMEOUT_MS)
    }

    const startStream = () => {
      setPhase("streaming")
      stream = openRunEvents(runId, {
        onEvent: (event) => {
          setEvents((current) => [...current, event])
          armIdle()
        },
        onError: (message) => setError(message),
        onStatus: (status) => setPhase(status === "reconnecting" ? "reconnecting" : "streaming"),
        onEnd: (status) => finish(status),
      })
      armIdle()
    }

    const loadHistory = async (status: string) => {
      try {
        const past = await api.run(runId, controller.signal)
        if (controller.signal.aborted) return
        setEvents(past)
        finish(status)
      } catch (caught) {
        if (controller.signal.aborted) return
        setError(errorText(caught))
        setPhase("error")
      }
    }

    const fallbackToTrace = async () => {
      try {
        const past = await api.run(runId, controller.signal)
        if (controller.signal.aborted) return
        const end = past.find((event) => event.event === "run_end")
        if (end) {
          setEvents(past)
          finish(end.status ?? "stream closed")
        } else {
          startStream()
        }
      } catch {
        if (controller.signal.aborted) return
        setPhase("error")
        setError("run not found")
      }
    }

    if (resumeQuery.status === "success") {
      if (resumeQuery.data.ended != null) void loadHistory(resumeQuery.data.ended)
      else startStream()
    } else {
      const caught = resumeQuery.error
      if (caught instanceof ApiError && caught.status === 404) void fallbackToTrace()
      else {
        setPhase("error")
        setError(errorText(caught))
      }
    }

    return () => {
      controller.abort()
      clearIdle()
      stream?.close()
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runId, resumeQuery.status, attempt])

  const start = events.find((event) => event.event === "run_start")
  const end = events.find((event) => event.event === "run_end")
  const steps = useMemo(
    () =>
      events
        .filter((event) => event.event === "step")
        .sort((a, b) => (a.seq ?? 0) - (b.seq ?? 0)),
    [events]
  )
  const okCount = steps.filter((step) => classifyStatus(step.status) === "ok").length
  const failedCount = steps.filter(
    (step) =>
      classifyStatus(step.status) === "failed" ||
      (step.exit_code != null && step.exit_code !== 0)
  ).length
  const outcome = end?.status ?? ended
  const appName = start?.app ?? null
  const mode = start?.mode
  const live = phase === "streaming" || phase === "reconnecting"
  const command = start?.argv?.join(" ")
  useDocumentTitle(
    `${runAction(start?.argv) ?? (appName ? "Run" : "Registry run")}${
      appName ? ` · ${appName}` : ""
    }`
  )

  if (resumeQuery.isError && phase === "error" && steps.length === 0) {
    const notFound = resumeQuery.error instanceof ApiError && resumeQuery.error.status === 404
    if (notFound && error === "run not found") {
      return (
        <EmptyState
          icon={ScrollText}
          title="That run is not in the log"
          description="The run id was not found in this estate's trace file. It may have been rotated away, or the API points at another host."
          action={
            <Button variant="outline" size="sm" render={<Link to="/runs" />}>
              Back to runs
            </Button>
          }
        />
      )
    }
  }

  return (
    <div className="space-y-5">
      <PageHeader
        eyebrow={
          <Link to="/runs" className="transition-colors hover:text-muted-foreground">
            Runs
          </Link>
        }
        title={
          `${runAction(start?.argv) ?? (appName ? "Run" : "Registry run")}${
            appName ? ` · ${appName}` : ""
          }${mode === "dry_run" ? " (dry run)" : ""}`
        }
        description={
          <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span>
              Run <span className="font-mono">{shortId(runId, 10)}</span>
            </span>
            <span className="text-subtle-foreground/50">·</span>
            {start?.ts ? (
              <>
                <span>Started</span>
                <RelativeTime value={start.ts} />
                <span className="text-subtle-foreground/50">·</span>
                <span className="font-mono text-xs" title={formatExact(start.ts)}>
                  {formatExact(start.ts)}
                </span>
              </>
            ) : (
              <span>Reading the trace…</span>
            )}
            {appName ? (
              <>
                <span className="text-subtle-foreground/50">·</span>
                <Link
                  to="/apps/$appName"
                  params={{ appName }}
                  search={{ tab: "overview" }}
                  className="inline-flex items-center gap-1 text-foreground hover:text-accent"
                >
                  <AppGlyph name={appName} size="xs" />
                  {appName}
                </Link>
              </>
            ) : null}
          </span>
        }
        badges={
          <>
            <RunStatusPill status={outcome ?? (live ? "running" : null)} />
            {mode === "dry_run" ? <StatusPill tone="info" label="Dry run" /> : null}
            {live ? (
              <StatusPill
                tone="info"
                label={phase === "reconnecting" ? "Reconnecting" : "Following live"}
                pulse
              />
            ) : null}
          </>
        }
        actions={
          <>
            <Button
              variant="outline"
              size="sm"
              onClick={() => void copy(runId)}
              title="Copy the full run id"
            >
              {copied ? <Check className="text-success" /> : <Terminal />}
              {copied ? "Copied" : "Copy id"}
            </Button>
            {appName ? (
              <Button
                size="sm"
                render={
                  <Link
                    to="/apps/$appName"
                    params={{ appName }}
                    search={{ tab: "overview" }}
                  />
                }
              >
                Open app
                <ArrowUpRight />
              </Button>
            ) : null}
          </>
        }
      />

      {end?.error ? (
        <Callout tone="danger" title="Why this run failed">
          {end.error}
        </Callout>
      ) : null}

      {error && phase === "error" ? (
        <Callout
          tone="danger"
          title="The run view could not continue"
          action={
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                void resumeQuery.refetch()
                setAttempt((value) => value + 1)
              }}
            >
              <RefreshCw />
              Retry
            </Button>
          }
        >
          {error}. Any steps already read remain below.
        </Callout>
      ) : null}

      <section className="overflow-hidden rounded-xl border border-border bg-card shadow-[var(--shadow-card)]">
        <div className="flex flex-wrap items-center gap-x-6 gap-y-3 px-4 py-3.5">
          <MetaList
            className="flex-1 sm:grid-cols-4"
            columns={4}
            items={[
              {
                label: "Outcome",
                value: <RunStatusPill status={outcome ?? (live ? "running" : null)} />,
              },
              {
                label: "App",
                value: appName ?? "Registry-wide",
              },
              {
                label: "Started",
                value: <RelativeTime value={start?.ts} />,
              },
              {
                label: "Steps",
                value: (
                  <span className="flex items-center gap-2">
                    <span>{steps.length}</span>
                    {failedCount > 0 ? (
                      <span className="text-destructive-strong">{failedCount} failed</span>
                    ) : (
                      <span className="text-2xs text-subtle-foreground">{okCount} ok</span>
                    )}
                  </span>
                ),
              },
            ]}
          />
          <div className="flex shrink-0 items-center gap-1.5 text-2xs text-subtle-foreground">
            {phase === "streaming" ? (
              <>
                <Radio className="size-3 animate-pulse text-info" />
                streaming
              </>
            ) : phase === "reconnecting" ? (
              <>
                <Radio className="size-3 animate-pulse text-warning" />
                reconnecting
              </>
            ) : phase === "error" ? (
              <>
                <WifiOff className="size-3 text-destructive-strong" />
                stalled
              </>
            ) : phase === "ended" ? (
              <>
                <Check className="size-3 text-success" />
                trace complete
              </>
            ) : (
              <>
                <Clock className="size-3" />
                loading
              </>
            )}
          </div>
        </div>

        {steps.length > 0 ? (
          <div className="flex h-1.5 w-full gap-px overflow-hidden border-t border-border/60 bg-muted">
            {steps.map((step, index) => (
              <span
                key={`${step.seq ?? index}-bar`}
                className={cn(
                  "h-full",
                  classifyStatus(step.status) === "failed" ? "bg-destructive/70" : "bg-success/60"
                )}
                style={{ width: `${(1 / steps.length) * 100}%` }}
              />
            ))}
          </div>
        ) : null}

        {command ? (
          <div className="flex items-center gap-2 border-t border-border/60 bg-surface/60 px-4 py-2">
            <Terminal className="size-3 shrink-0 text-subtle-foreground" />
            <code className="min-w-0 flex-1 overflow-x-auto font-mono text-[10px] whitespace-nowrap text-muted-foreground">
              {command}
            </code>
          </div>
        ) : null}
      </section>

      <section className="overflow-hidden rounded-xl border border-border bg-card shadow-[var(--shadow-card)]">
        <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border/60 px-4 py-3">
          <div>
            <h2 className="text-sm font-medium tracking-[-0.01em]">Steps</h2>
            <p className="text-2xs text-muted-foreground">
              {plural(steps.length, "step")} read
              {end?.steps != null && end.steps !== steps.length ? `, ${end.steps} reported` : ""}
              {failedCount > 0 ? ` · ${failedCount} failed` : ""}
            </p>
          </div>
          {live ? (
            <span className="inline-flex items-center gap-1.5 text-2xs text-info">
              <StatusDot tone="info" pulse />
              appending live
            </span>
          ) : null}
        </div>

        {steps.length === 0 ? (
          phase === "loading" || phase === "streaming" || phase === "reconnecting" ? (
            <div className="space-y-3 p-4">
              {[0, 1, 2].map((index) => (
                <div key={index} className="flex items-center gap-3">
                  <Skeleton className="size-5 rounded-full" />
                  <Skeleton className="h-3 flex-1" />
                  <Skeleton className="h-3 w-16" />
                </div>
              ))}
            </div>
          ) : (
            <div className="flex flex-col items-center gap-2 px-4 py-10 text-center">
              <ScrollText className="size-4 text-subtle-foreground" />
              <p className="text-sm text-foreground">No steps recorded</p>
              <p className="text-xs text-muted-foreground">
                The run wrote a start and end record but no step in between.
              </p>
            </div>
          )
        ) : (
          <ol className="py-1">
            {steps.map((step, index) => (
              <StepRow
                key={`${step.seq ?? index}-${step.step ?? ""}`}
                step={step}
                index={index}
                last={index === steps.length - 1}
              />
            ))}
          </ol>
        )}
      </section>

      <details className="group rounded-xl border border-border bg-card">
        <summary className="flex cursor-pointer list-none items-center gap-2 px-4 py-3 text-xs font-medium text-muted-foreground select-none">
          <ArrowRight className="size-3 transition-transform duration-150 group-open:rotate-90" />
          Raw trace
          <span className="ml-auto text-2xs font-normal text-subtle-foreground">
            {plural(events.length, "event")}
          </span>
        </summary>
        <pre className="code-surface max-h-96 overflow-auto border-t border-border/60 p-4 font-mono text-[10px] leading-relaxed">
          {JSON.stringify(events, null, 2)}
        </pre>
      </details>

      <div className="flex items-center justify-between border-t border-border/60 pt-3 text-2xs text-subtle-foreground">
        <span className="font-mono">{runId}</span>
        <Link to="/runs" className="inline-flex items-center gap-1 hover:text-muted-foreground">
          Back to all runs
          <ArrowRight className="size-3" />
        </Link>
      </div>
    </div>
  )
}
