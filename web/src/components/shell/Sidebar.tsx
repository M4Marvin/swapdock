import { useMemo } from "react"
import { Link } from "@tanstack/react-router"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import {
  Activity,
  Command,
  LogOut,
  Plus,
  ScrollText,
  Stethoscope,
  X,
} from "lucide-react"
import { cfgState, type AppEntry } from "@/api"
import { logout } from "@/lib/auth"
import { queryKeys } from "@/lib/query-keys"
import { needsAdoption } from "@/lib/deployment"
import { cn } from "cn"
import { Wordmark } from "@/components/Brand"
import { StatusDot, type Tone } from "@/components/Status"
import { Kbd, RelativeTime, Spinner } from "@/components/kit"
import { useModKey } from "@/lib/hooks"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { NewAppButton } from "@/components/app/NewAppDialog"

function appTone(entry: AppEntry): Tone {
  if (entry.problems.some((problem) => problem.severity === "error")) return "danger"
  if (entry.problems.length > 0) return "warning"
  if (needsAdoption(entry.app)) return "warning"
  if (entry.app.live_port != null) return "success"
  if (entry.app.kind === "static" && entry.app.root != null) return "info"
  return "neutral"
}

/** Screen-reader status for the colour-only dot in each app row. */
function statusText(entry: AppEntry): string {
  const worst = entry.problems.find((problem) => problem.severity === "error") ?? entry.problems[0]
  if (worst) {
    return `${worst.severity === "error" ? "Error" : "Warning"}: ${worst.message}`
  }
  if (needsAdoption(entry.app)) return "Unmanaged, needs takeover"
  if (entry.app.live_port != null) return "Live"
  if (entry.app.kind === "static" && entry.app.root != null) return "Static, served"
  return "Not deployed"
}

function navClass(active: boolean) {
  return cn(
    "group relative flex h-8 items-center gap-2.5 rounded-md px-2.5 text-sm transition-[color,background-color] duration-150",
    active
      ? "bg-sidebar-accent font-medium text-foreground"
      : "text-muted-foreground hover:bg-sidebar-accent/60 hover:text-foreground"
  )
}

export function SidebarContent({
  apps,
  appsLoading,
  appsError,
  appsUpdatedAt,
  apiStatus,
  pathname,
  onOpenPalette,
  onNavigate,
  onClose,
}: {
  apps: AppEntry[]
  appsLoading: boolean
  appsError: boolean
  appsUpdatedAt: number
  apiStatus: "online" | "offline" | "checking"
  pathname: string
  onOpenPalette: () => void
  onNavigate?: () => void
  onClose?: () => void
}) {
  const modKey = useModKey()
  const queryClient = useQueryClient()
  const configQuery = useQuery({ queryKey: queryKeys.config, queryFn: () => cfgState() })
  const endpoint = useMemo(() => {
    const base = configQuery.data?.config.deployApi
    if (!base) return null
    try {
      const url = new URL(base, window.location.origin)
      return url.host || "same origin"
    } catch {
      return base
    }
  }, [configQuery.data])

  const sorted = useMemo(
    () => [...apps].sort((a, b) => a.app.name.localeCompare(b.app.name)),
    [apps]
  )
  const liveApps = apps.filter((entry) => entry.app.live_port != null).length

  const isOverview = pathname === "/"
  const isRuns = pathname === "/runs" || pathname.startsWith("/runs/")
  const isDoctor = pathname === "/doctor"

  return (
    <div className="flex h-full min-h-0 flex-col bg-sidebar text-sidebar-foreground">
      <div className="flex h-14 shrink-0 items-center gap-2 border-b border-sidebar-border px-4">
        <Link
          to="/"
          onClick={onNavigate}
          className="flex min-w-0 items-center rounded-md outline-none focus-visible:ring-2 focus-visible:ring-sidebar-ring"
          aria-label="swapdock — fleet overview"
        >
          <Wordmark />
        </Link>
        {onClose ? (
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            className="ml-auto text-muted-foreground"
            onClick={onClose}
            aria-label="Close navigation"
          >
            <X />
          </Button>
        ) : null}
      </div>

      <div className="px-2 pt-3">
        <nav aria-label="Main" className="flex flex-col gap-0.5">
          <Link
            to="/"
            onClick={onNavigate}
            className={navClass(isOverview)}
            aria-current={isOverview ? "page" : undefined}
          >
            <Activity className="size-4 opacity-70" />
            <span>Overview</span>
            {apps.length > 0 ? (
              <span className="ml-auto font-mono text-2xs text-subtle-foreground">{apps.length}</span>
            ) : null}
          </Link>
          <Link
            to="/runs"
            onClick={onNavigate}
            className={navClass(isRuns)}
            aria-current={isRuns ? "page" : undefined}
          >
            <ScrollText className="size-4 opacity-70" />
            <span>Runs</span>
          </Link>
          <Link
            to="/doctor"
            onClick={onNavigate}
            className={navClass(isDoctor)}
            aria-current={isDoctor ? "page" : undefined}
          >
            <Stethoscope className="size-4 opacity-70" />
            <span>Doctor</span>
          </Link>
        </nav>
      </div>

      <div className="mt-6 flex h-6 shrink-0 items-center justify-between pl-4 pr-3">
        <span className="label-eyebrow">Applications</span>
        <span className="flex items-center gap-1.5 text-2xs text-subtle-foreground">
          <span className="tabular">{liveApps} live</span>
        </span>
      </div>

      <nav
        aria-label="Applications"
        className="mt-1 min-h-0 flex-1 overflow-y-auto overscroll-contain px-2 pb-3 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
      >
        {appsLoading ? (
          <div className="space-y-1 px-2 pt-1">
            {[0, 1, 2, 3, 4, 5].map((index) => (
              <div key={index} className="flex h-7 items-center gap-2">
                <span className="size-1.5 rounded-full bg-muted" />
                <Skeleton className="h-3" style={{ width: `${52 + (index % 3) * 18}px` }} />
              </div>
            ))}
          </div>
        ) : appsError ? (
          <p className="px-2 pt-1 text-xs leading-relaxed text-muted-foreground">
            Registry unavailable. Check the API connection below.
          </p>
        ) : sorted.length === 0 ? (
          <p className="px-2 pt-1 text-xs leading-relaxed text-muted-foreground">
            No applications in the registry yet.
          </p>
        ) : (
          sorted.map((entry) => {
            const active = pathname === `/apps/${entry.app.name}`
            const tone = appTone(entry)
            return (
              <Link
                key={entry.app.name}
                to="/apps/$appName"
                params={{ appName: entry.app.name }}
                search={{ tab: "overview" }}
                onClick={onNavigate}
                aria-current={active ? "page" : undefined}
                title={
                  entry.problems[0]?.message ??
                  (entry.app.live_port != null
                    ? `Live on port ${entry.app.live_port}`
                    : "Not deployed")
                }
                className={cn(
                  "group relative flex h-8 items-center gap-2.5 rounded-md px-2.5 text-sm transition-[color,background-color] duration-150",
                  active
                    ? "bg-sidebar-accent font-medium text-foreground"
                    : "text-muted-foreground hover:bg-sidebar-accent/60 hover:text-foreground"
                )}
              >
                <StatusDot tone={tone} />
                <span className="sr-only">Status: {statusText(entry)}. </span>
                <span className="min-w-0 flex-1 truncate">{entry.app.name}</span>
                {entry.app.live_port != null ? (
                  <span
                    className={cn(
                      "font-mono text-2xs tabular transition-colors",
                      active
                        ? "text-muted-foreground"
                        : "text-subtle-foreground group-hover:text-muted-foreground"
                    )}
                  >
                    {entry.app.live_port}
                  </span>
                ) : entry.app.kind === "static" && entry.app.root != null ? (
                  <span
                    className={cn(
                      "text-2xs transition-colors",
                      active ? "text-muted-foreground" : "text-subtle-foreground group-hover:text-muted-foreground"
                    )}
                  >
                    static
                  </span>
                ) : (
                  <span className="text-2xs text-subtle-foreground/60">—</span>
                )}
              </Link>
            )
          })
        )}
      </nav>

      <div className="shrink-0 px-2 pb-2">
        <NewAppButton
          variant="ghost"
          size="sm"
          className="w-full justify-start text-muted-foreground"
          onCreated={onNavigate}
        >
          <Plus />
          New app
        </NewAppButton>
      </div>

      <div className="shrink-0 border-t border-sidebar-border p-3">
        <button
          type="button"
          onClick={onOpenPalette}
          className="mb-2 flex h-7 w-full items-center gap-2 rounded-md px-2 text-xs text-muted-foreground transition-colors hover:bg-sidebar-accent/70 hover:text-foreground"
        >
          <Command className="size-3.5" />
          <span>Quick actions</span>
          <Kbd className="ml-auto border-sidebar-border bg-sidebar-accent/60">{modKey} K</Kbd>
        </button>

        <div className="flex items-center gap-2 px-2 py-1">
          {apiStatus === "checking" ? (
            <Spinner className="size-3 text-subtle-foreground" />
          ) : (
            <StatusDot
              tone={apiStatus === "online" ? "success" : "danger"}
              title={apiStatus === "online" ? "API connected" : "API unreachable"}
            />
          )}
          <div className="min-w-0 flex-1 leading-tight">
            <div className="truncate text-2xs font-medium text-muted-foreground">
              {apiStatus === "online"
                ? "API connected"
                : apiStatus === "offline"
                  ? "API unreachable"
                  : "Connecting…"}
            </div>
            <div className="truncate font-mono text-2xs text-subtle-foreground">
              {endpoint ?? "resolving endpoint…"}
            </div>
          </div>
          {apiStatus === "online" && appsUpdatedAt > 0 ? (
            <span className="text-2xs text-subtle-foreground" title="Last registry refresh">
              <RelativeTime value={appsUpdatedAt} />
            </span>
          ) : null}
        </div>
        <button
          type="button"
          onClick={() => {
            queryClient.clear()
            void logout()
          }}
          className="mt-1 flex h-6 w-full items-center gap-2 rounded-md px-2 text-2xs text-subtle-foreground transition-colors hover:bg-sidebar-accent/70 hover:text-foreground"
        >
          <LogOut className="size-3" />
          <span>Sign out</span>
          <span className="ml-auto text-subtle-foreground/70">sign out</span>
        </button>
      </div>
    </div>
  )
}
