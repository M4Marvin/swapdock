import { useQuery } from "@tanstack/react-query"
import { api, classifyStatus, type AppStatus } from "@/api"
import { queryKeys } from "@/lib/query-keys"
import { cn } from "cn"
import { formatStatus } from "@/api"

export type Tone = "success" | "warning" | "danger" | "neutral" | "info" | "accent"

const toneDot: Record<Tone, string> = {
  success: "bg-success",
  warning: "bg-warning",
  danger: "bg-destructive",
  neutral: "bg-subtle-foreground/60",
  info: "bg-info",
  accent: "bg-accent",
}

const tonePill: Record<Tone, string> = {
  success: "border-success/20 bg-success/10 text-success-strong",
  warning: "border-warning/25 bg-warning/10 text-warning-strong",
  danger: "border-destructive/20 bg-destructive/10 text-destructive-strong",
  neutral: "border-border bg-muted/50 text-muted-foreground",
  info: "border-info/20 bg-info/10 text-info-strong",
  accent: "border-accent/20 bg-accent-soft text-accent",
}

export function StatusDot({
  tone,
  pulse = false,
  className,
  title,
}: {
  tone: Tone
  pulse?: boolean
  className?: string
  title?: string
}) {
  return (
    <span
      aria-hidden="true"
      title={title}
      className={cn(
        "status-dot",
        toneDot[tone],
        pulse && "animate-pulse-soft",
        tone === "neutral" && "ring-1 ring-inset ring-foreground/10",
        className
      )}
    />
  )
}

/** A small status capsule: dot, label, optional count or code. */
export function StatusPill({
  tone,
  label,
  pulse = false,
  className,
  title,
  children,
}: {
  tone: Tone
  label: React.ReactNode
  pulse?: boolean
  className?: string
  title?: string
  children?: React.ReactNode
}) {
  return (
    <span
      title={title}
      className={cn(
        "inline-flex h-5 max-w-full items-center gap-1.5 rounded-full border px-2 text-2xs font-medium whitespace-nowrap",
        tonePill[tone],
        className
      )}
    >
      <StatusDot tone={tone} pulse={pulse} />
      <span className="truncate">{label}</span>
      {children}
    </span>
  )
}

export function isAppUp(status: AppStatus | undefined): boolean {
  if (!status) return false
  if (status.up === true) return true
  return typeof status.status === "number" && status.status >= 200 && status.status < 300
}

/** Polls one app's live health while a component that needs it is mounted. */
export function useAppHealth(name: string, livePort: number | null) {
  return useQuery({
    queryKey: queryKeys.appStatus(name),
    queryFn: ({ signal }) => api.appStatus(name, signal),
    enabled: livePort != null,
    refetchInterval: 20_000,
    staleTime: 10_000,
    retry: 1,
  })
}

/** Health verdict for an app, including "never deployed" as its own state. */
export function HealthPill({
  name,
  livePort,
  showCode = false,
  staticServed = false,
  unmanaged = false,
  className,
}: {
  name: string
  livePort: number | null
  showCode?: boolean
  /** A static app is served by nginx directly and has no container probe. */
  staticServed?: boolean
  /** Running outside swapdock; it needs a one-time adoption before deploys. */
  unmanaged?: boolean
  className?: string
}) {
  const query = useAppHealth(name, livePort)

  if (livePort == null) {
    if (staticServed) {
      return (
        <StatusPill
          tone="info"
          label="Static"
          title="Served directly from root; there is no container health probe"
          className={className}
        />
      )
    }
    if (unmanaged) {
      return (
        <StatusPill
          tone="warning"
          label="Unmanaged"
          title="Running outside swapdock; take it over before deploying"
          className={className}
        />
      )
    }
    return (
      <StatusPill
        tone="neutral"
        label="Idle"
        title="No release is behind nginx yet"
        className={className}
      />
    )
  }
  if (query.isPending) {
    return (
      <span
        className={cn(
          "inline-flex h-5 items-center gap-1.5 rounded-full border border-border bg-muted/50 px-2 text-2xs text-muted-foreground",
          className
        )}
      >
        <StatusDot tone="neutral" />
        Checking
      </span>
    )
  }
  if (query.isError || !query.data) {
    return (
      <StatusPill
        tone="warning"
        label="Unknown"
        title="The health probe could not be completed"
        className={className}
      />
    )
  }
  const code = showCode && typeof query.data.status === "number" ? ` ${query.data.status}` : ""
  return isAppUp(query.data) ? (
    <StatusPill
      tone="success"
      label={`Live${code}`}
      title={`HTTP ${query.data.status ?? "?"} on 127.0.0.1:${query.data.live_port ?? livePort}`}
      className={className}
    />
  ) : (
    <StatusPill
      tone="danger"
      label={`Down${code}`}
      title={`Last probe returned ${query.data.status ?? "no answer"}`}
      className={className}
    />
  )
}

const runLabel: Record<string, string> = {
  ok: "Succeeded",
  failed: "Failed",
  dry: "Dry run",
  running: "Running",
  interrupted: "Interrupted",
  unknown: "Unknown",
}

export function runTone(status: string | null | undefined): Tone {
  switch (classifyStatus(status)) {
    case "ok":
      return "success"
    case "failed":
      return "danger"
    case "dry":
      return "info"
    case "running":
      return "info"
    case "interrupted":
      return "warning"
    default:
      return "neutral"
  }
}

/** Status capsule for a run or a step. */
export function RunStatusPill({
  status,
  label,
  className,
}: {
  status: string | null | undefined
  label?: React.ReactNode
  className?: string
}) {
  const outcome = classifyStatus(status)
  const running = outcome === "running"
  return (
    <StatusPill
      tone={runTone(status)}
      label={
        label ??
        (running
          ? "Running"
          : (runLabel[outcome] ?? (status ? formatStatus(status) : "Unknown")))
      }
      pulse={running || (outcome === "unknown" && status == null)}
      className={className}
    />
  )
}

export function problemTone(
  problems: { severity: "warning" | "error" }[],
  healthDown = false
): Tone {
  if (problems.some((p) => p.severity === "error") || healthDown) return "danger"
  if (problems.length > 0) return "warning"
  return "success"
}
