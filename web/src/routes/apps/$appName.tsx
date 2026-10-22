import { useEffect, useRef, useState } from "react"
import { createFileRoute, Link } from "@tanstack/react-router"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"
import {
  ArrowRight,
  ChevronRight,
  Ellipsis,
  ExternalLink,
  GitBranch,
  PackagePlus,
  Rocket,
  RotateCcw,
  Terminal,
  Wrench,
} from "lucide-react"
import { ApiError, api, type App, type AppEntry } from "@/api"
import { queryKeys } from "@/lib/query-keys"
import { errorText, shortId, shortRelease } from "@/lib/format"
import { isRelease, sameRelease } from "@/lib/releases"
import { appSiteUrl } from "@/lib/hostnames"
import { useDocumentTitle } from "@/lib/hooks"
import { cn } from "cn"
import { PageHeader } from "@/components/PageHeader"
import { AppGlyph } from "@/components/AppGlyph"
import { HealthPill, StatusPill } from "@/components/Status"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { Skeleton } from "@/components/ui/skeleton"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Callout, EmptyState, MetaList } from "@/components/kit"
import { AppOverview } from "@/components/app/AppOverview"
import { VersionDriftBadge } from "@/components/app/VersionDrift"
import { DeployDialog } from "@/components/app/DeployDialog"
import { Pipeline } from "@/components/Pipeline"
import { Operations } from "@/components/Operations"
import { RegistryEditor } from "@/components/RegistryEditor"
import { AdoptDialog } from "@/components/app/AdoptDialog"
import { DeploymentModeView } from "@/components/app/DeploymentModes"
import { deploymentKind, needsAdoption } from "@/lib/deployment"

const TABS = ["overview", "pipeline", "operations", "configuration"] as const
type Tab = (typeof TABS)[number]

const TAB_LABELS: Record<Tab, string> = {
  overview: "Overview",
  pipeline: "Deployment",
  operations: "Operations",
  configuration: "Configuration",
}

function isTab(value: unknown): value is Tab {
  return typeof value === "string" && (TABS as readonly string[]).includes(value)
}

export const Route = createFileRoute("/apps/$appName")({
  validateSearch: (
    search: Record<string, unknown>
  ): { tab: Tab; release?: string } => {
    const result: { tab: Tab; release?: string } = {
      tab: isTab(search.tab) ? search.tab : "overview",
    }
    // Set when the version card sends the operator to the pipeline with a
    // specific release preselected. It is display state, not a deploy action.
    if (typeof search.release === "string" && isRelease(search.release)) {
      result.release = search.release
    }
    return result
  },
  component: AppDetailRoute,
})

/** Remount when the app changes so tab, dialogs and drafts never leak across apps. */
function AppDetailRoute() {
  const { appName } = Route.useParams()
  return <AppDetail key={appName} />
}

/** A rollback must move backward: a retained-but-identical release is not an action. */
function canRollback(app: App): boolean {
  return (
    app.old_release != null &&
    app.live_port != null &&
    !sameRelease(app.release, app.old_release)
  )
}

function TechnicalDetails({ app }: { app: App }) {
  const items: { label: string; value: string; mono?: boolean }[] = [
    { label: "Front port", value: `:${app.front_port}`, mono: true },
    { label: "Slot", value: String(app.slot), mono: true },
    { label: "Strategy", value: app.strategy },
    { label: "Live port", value: app.live_port != null ? `:${app.live_port}` : "—", mono: true },
    { label: "Raw release", value: app.release ?? "—", mono: true },
    { label: "Health check", value: app.health_url ?? "—", mono: true },
  ]
  return (
    <details className="group/details mt-1.5 w-full max-w-xl text-2xs">
      <summary className="inline-flex cursor-pointer list-none items-center gap-1 rounded-md py-0.5 text-subtle-foreground transition-colors hover:text-muted-foreground [&::-webkit-details-marker]:hidden">
        <ChevronRight className="size-3 transition-transform duration-150 group-open/details:rotate-90" />
        Technical details
      </summary>
      <dl className="mt-2 grid grid-cols-2 gap-x-5 gap-y-2 rounded-lg border border-border bg-surface/50 p-2.5 sm:grid-cols-3">
        {items.map((item) => (
          <div key={item.label} className="min-w-0">
            <dt className="label-eyebrow">{item.label}</dt>
            <dd
              className={cn(
                "mt-0.5 truncate text-[11px] text-muted-foreground",
                item.mono && "font-mono"
              )}
              title={item.value}
            >
              {item.value}
            </dd>
          </div>
        ))}
      </dl>
    </details>
  )
}

function HeaderActionMenu({
  app,
  onRollback,
  onSync,
  syncing,
  onOpenOperations,
}: {
  app: App
  onRollback: () => void
  onSync: () => void
  syncing: boolean
  onOpenOperations: () => void
}) {
  const [open, setOpen] = useState(false)
  const rootRef = useRef<HTMLDivElement>(null)
  const firstRef = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    if (!open) return
    const onPointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false)
    }
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setOpen(false)
        rootRef.current?.querySelector<HTMLButtonElement>("button[data-menu-trigger]")?.focus()
      }
    }
    document.addEventListener("pointerdown", onPointerDown, true)
    document.addEventListener("keydown", onKeyDown)
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true)
      document.removeEventListener("keydown", onKeyDown)
    }
  }, [open])

  useEffect(() => {
    if (open) firstRef.current?.focus()
  }, [open])

  const copyCommand = async () => {
    const command = `swapdock up ${app.name}`
    try {
      await navigator.clipboard.writeText(command)
      toast.success("Deploy command copied", { description: command })
    } catch {
      toast.error("Could not copy the command", { description: command })
    }
    setOpen(false)
  }

  const rollbackDisabled = !canRollback(app)
  const rollbackUnavailable =
    app.old_release == null || sameRelease(app.release, app.old_release)
      ? "No previous version retained"
      : "Nothing is live to roll back"

  return (
    <div ref={rootRef} className="relative">
      <Button
        type="button"
        data-menu-trigger=""
        variant="outline"
        size="icon-sm"
        aria-label="More app actions"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        <Ellipsis />
      </Button>

      {open ? (
        <div
          role="menu"
          aria-label="App actions"
          className="animate-pop-in absolute top-full right-0 z-40 mt-1.5 w-56 origin-top-right rounded-lg border border-border bg-popover p-1 shadow-[var(--shadow-pop)]"
        >
          <button
            ref={firstRef}
            type="button"
            role="menuitem"
            disabled={app.repo == null || syncing}
            title={app.repo == null ? "No source checkout is recorded" : undefined}
            onClick={() => {
              onSync()
              setOpen(false)
            }}
            className="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm text-muted-foreground transition-colors duration-100 hover:bg-muted hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
          >
            <GitBranch className="size-3.5" />
            <span className="flex-1">{syncing ? "Syncing…" : "Sync source"}</span>
          </button>
          <button
            type="button"
            role="menuitem"
            disabled={rollbackDisabled}
            title={rollbackDisabled ? rollbackUnavailable : `Roll back to ${app.old_release}`}
            onClick={() => {
              if (rollbackDisabled) return
              onRollback()
              setOpen(false)
            }}
            className="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm text-muted-foreground transition-colors duration-100 hover:bg-muted hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
          >
            <RotateCcw className="size-3.5" />
            <span className="flex-1">
              {rollbackDisabled ? rollbackUnavailable : "Roll back"}
            </span>
            {!rollbackDisabled && app.old_release ? (
              <span className="font-mono text-2xs text-subtle-foreground">
                {shortRelease(app.old_release)}
              </span>
            ) : null}
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => void copyCommand()}
            className="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm text-muted-foreground transition-colors duration-100 hover:bg-muted hover:text-foreground"
          >
            <Terminal className="size-3.5" />
            <span className="flex-1">Copy deploy command</span>
          </button>
          <div role="separator" className="my-1 h-px bg-border" />
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              onOpenOperations()
              setOpen(false)
            }}
            className="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm text-muted-foreground transition-colors duration-100 hover:bg-muted hover:text-foreground"
          >
            <Wrench className="size-3.5" />
            <span className="flex-1">Operations</span>
            <ChevronRight className="size-3" />
          </button>
        </div>
      ) : null}
    </div>
  )
}

function AppDetail() {
  const { appName } = Route.useParams()
  const search = Route.useSearch()
  const navigate = Route.useNavigate()
  const queryClient = useQueryClient()
  const tab: Tab = search.tab ?? "overview"

  const [deployOpen, setDeployOpen] = useState(false)
  const [deployRelease, setDeployRelease] = useState<string | null>(null)
  const [adoptOpen, setAdoptOpen] = useState(false)
  const [rollbackOpen, setRollbackOpen] = useState(false)

  const appQuery = useQuery({
    queryKey: queryKeys.app(appName),
    queryFn: ({ signal }) => api.app(appName, signal),
  })

  const entry: AppEntry | undefined = appQuery.data
  const app = entry?.app
  useDocumentTitle(app ? app.name : "Application")

  const invalidateApp = () => {
    void queryClient.invalidateQueries({ queryKey: queryKeys.app(appName) })
    void queryClient.invalidateQueries({ queryKey: queryKeys.apps })
    void queryClient.invalidateQueries({ queryKey: queryKeys.runs })
    void queryClient.invalidateQueries({ queryKey: queryKeys.appStatus(appName) })
    void queryClient.invalidateQueries({ queryKey: queryKeys.latest(appName) })
    void queryClient.invalidateQueries({ queryKey: queryKeys.git(appName) })
    void queryClient.invalidateQueries({ queryKey: queryKeys.images(appName) })
  }

  const sync = useMutation({
    mutationFn: () => api.sync(appName),
    onSuccess: ({ run_id }) => {
      toast.success("Source sync started", { description: `run ${shortId(run_id)}` })
      invalidateApp()
    },
    onError: (error) => toast.error("Sync did not start", { description: errorText(error) }),
  })

  const rollback = useMutation({
    mutationFn: () => {
      if (!app || !canRollback(app)) {
        throw new Error("No previous version is retained for a rollback.")
      }
      return api.rollback(appName)
    },
    onSuccess: ({ run_id }) => {
      toast.success("Rollback started", {
        description: `run ${shortId(run_id)} · back to ${app?.old_release ?? "previous release"}`,
      })
      setRollbackOpen(false)
      invalidateApp()
    },
    onError: (error) => toast.error("Rollback did not start", { description: errorText(error) }),
  })

  const openTab = (next: Tab) => {
    void navigate({ to: "/apps/$appName", params: { appName }, search: { tab: next } })
  }

  const openDeploy = (release: string | null = null) => {
    setDeployRelease(release)
    setDeployOpen(true)
  }

  const openPipeline = (release?: string) => {
    void navigate({
      to: "/apps/$appName",
      params: { appName },
      search: { tab: "pipeline", release },
    })
  }

  useEffect(() => {
    window.scrollTo({ top: 0, behavior: "auto" })
  }, [tab, appName])

  if (appQuery.isError && !appQuery.data) {
    const notFound = appQuery.error instanceof ApiError && appQuery.error.status === 404
    return (
      <EmptyState
        icon={notFound ? Wrench : Terminal}
        title={notFound ? "That app is not in the registry" : "The app could not be loaded"}
        description={
          notFound
            ? `No registry entry named “${appName}” was found. It may have been removed, or the API is pointing at another estate.`
            : errorText(appQuery.error)
        }
        action={
          <>
            <Button variant="outline" size="sm" onClick={() => void appQuery.refetch()}>
              Retry
            </Button>
            <Button variant="ghost" size="sm" render={<Link to="/" />}>
              Back to fleet
            </Button>
          </>
        }
      />
    )
  }

  if (appQuery.isPending || !app || !entry) {
    return (
      <div className="space-y-5">
        <div className="flex items-center gap-3">
          <Skeleton className="size-10 rounded-xl" />
          <div className="space-y-2">
            <Skeleton className="h-6 w-36" />
            <Skeleton className="h-3 w-64" />
          </div>
        </div>
        <Skeleton className="h-9 w-full max-w-md" />
        <Skeleton className="h-52 w-full" />
        <div className="grid gap-4 xl:grid-cols-3">
          <Skeleton className="h-64 xl:col-span-2" />
          <Skeleton className="h-64" />
        </div>
      </div>
    )
  }

  const hostname = app.hostnames[0]
  const problems = entry.problems
  const kind = deploymentKind(app)
  const adoption = needsAdoption(app)

  return (
    <div className="space-y-0">
      <PageHeader
        leading={<AppGlyph name={app.name} size="xl" />}
        eyebrow="Application"
        title={app.name}
        badges={
          <>
            {app.kind === "static" ? (
              <StatusPill
                tone={app.root != null ? "info" : "neutral"}
                label={app.root != null ? "Static" : "No root"}
                title="Static apps are served by nginx from root; there is no container health probe"
              />
            ) : (
              <HealthPill
                name={app.name}
                livePort={app.live_port}
                showCode
                unmanaged={adoption}
              />
            )}
            <Badge variant="outline">
              {app.kind === "static" ? "Static site" : "Container app"}
            </Badge>
            <VersionDriftBadge app={app} />
            {problems.length > 0 ? (
              <Badge variant={problems.some((p) => p.severity === "error") ? "destructive" : "warning"}>
                {problems.length} finding{problems.length === 1 ? "" : "s"}
              </Badge>
            ) : null}
          </>
        }
        description={
          <div className="flex flex-col gap-1">
            <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
              {hostname ? (
                <a
                  href={appSiteUrl(app) ?? undefined}
                  target="_blank"
                  rel="noreferrer"
                  className="inline-flex items-center gap-1 font-mono text-xs text-muted-foreground underline decoration-border underline-offset-4 transition-colors hover:text-foreground hover:decoration-foreground/30"
                >
                  {hostname}
                  <ExternalLink className="size-2.5" />
                </a>
              ) : (
                <span className="text-muted-foreground">No hostname routes here yet</span>
              )}
            </div>
            <TechnicalDetails app={app} />
          </div>
        }
        actions={
          <>
            {adoption ? (
              <Button size="sm" onClick={() => setAdoptOpen(true)}>
                <PackagePlus />
                Take over app
              </Button>
            ) : kind === "static" ? (
              hostname ? (
                <Button
                  variant="outline"
                  size="sm"
                  render={<a href={appSiteUrl(app) ?? undefined} target="_blank" rel="noreferrer" />}
                >
                  <ExternalLink />
                  Open site
                </Button>
              ) : null
            ) : kind === "compose" ? (
              <Button
                variant="outline"
                size="sm"
                render={
                  <Link
                    to="/apps/$appName"
                    params={{ appName: app.name }}
                    search={{ tab: "configuration" }}
                  />
                }
              >
                Set up deployment
              </Button>
            ) : (
              <Button size="sm" onClick={() => openDeploy(null)}>
                <Rocket />
                Deploy
              </Button>
            )}
            <HeaderActionMenu
              app={app}
              syncing={sync.isPending}
              onSync={() => sync.mutate()}
              onRollback={() => setRollbackOpen(true)}
              onOpenOperations={() => openTab("operations")}
            />
          </>
        }
      />

      <nav
        aria-label="Application sections"
        className="sticky top-14 z-20 -mx-4 mt-5 flex items-center gap-0.5 overflow-x-auto border-b border-border bg-[var(--header)] px-4 backdrop-blur-xl sm:-mx-6 sm:px-6 lg:-mx-8 lg:px-8"
      >
        {TABS.map((item) => {
          const active = tab === item
          return (
            <Link
              key={item}
              to="/apps/$appName"
              params={{ appName }}
              search={{ tab: item }}
              aria-current={active ? "page" : undefined}
              className={cn(
                "relative flex h-10 shrink-0 items-center gap-1.5 px-2.5 text-sm transition-colors duration-150",
                active
                  ? "font-medium text-foreground"
                  : "text-muted-foreground hover:text-foreground"
              )}
            >
              {TAB_LABELS[item]}
              {item === "configuration" && problems.length > 0 ? (
                <span className="size-1.5 rounded-full bg-warning" aria-hidden="true" />
              ) : null}
              <span
                aria-hidden="true"
                className={cn(
                  "absolute inset-x-2 bottom-0 h-[2px] rounded-full transition-[background-color,opacity] duration-150",
                  active ? "bg-foreground opacity-100" : "bg-foreground opacity-0"
                )}
              />
            </Link>
          )
        })}
        <span className="ml-auto hidden items-center gap-1.5 pr-1 text-2xs text-subtle-foreground md:flex">
          {adoption ? (
            <>
              <span className="size-1.5 rounded-full bg-warning" />
              unmanaged
            </>
          ) : app.live_port != null ? (
            <>
              <span className="size-1.5 rounded-full bg-success" />
              live
            </>
          ) : app.kind === "static" && app.root != null ? (
            <>
              <span className="size-1.5 rounded-full bg-info" />
              static, served
            </>
          ) : (
            <>
              <span className="size-1.5 rounded-full bg-subtle-foreground/50" />
              not deployed
            </>
          )}
        </span>
      </nav>

      <div key={tab} className="animate-fade-in pt-4">
        {tab === "overview" ? (
          <AppOverview
            app={app}
            problems={problems}
            onAdopt={() => setAdoptOpen(true)}
            onDeploy={(release) => openDeploy(release)}
            onBuild={(release) => openPipeline(release)}
            onOpenPipeline={() => openTab("pipeline")}
          />
        ) : null}
        {tab === "pipeline" ? (
          <DeploymentModeView
            app={app}
            onAdopt={() => setAdoptOpen(true)}
            onDeploy={() => openDeploy(null)}
            pipeline={
              <Pipeline
                app={app}
                initialRelease={search.release}
                onDeployed={invalidateApp}
              />
            }
          />
        ) : null}
        {tab === "operations" ? <Operations app={app} /> : null}
        {tab === "configuration" ? <RegistryEditor key={app.name} app={app} /> : null}
      </div>

      <DeployDialog
        app={app}
        open={deployOpen}
        initialRelease={deployRelease}
        onOpenChange={(open) => {
          setDeployOpen(open)
          if (!open) setDeployRelease(null)
        }}
        onStarted={invalidateApp}
      />

      <AdoptDialog
        app={app}
        open={adoptOpen}
        onOpenChange={setAdoptOpen}
        onStarted={invalidateApp}
      />

      <Dialog
        open={rollbackOpen && canRollback(app)}
        onOpenChange={(open) => {
          if (!open) setRollbackOpen(false)
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Roll back {app.name}?</DialogTitle>
            <DialogDescription>
              This redeploys the previous release through the same {app.strategy} health gate.
              It changes production now; another deploy is required to move forward again.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-3">
            <div className="rounded-xl border border-border bg-muted/25 p-3">
              <MetaList
                columns={2}
                items={[
                  { label: "Rolling back to", value: shortRelease(app.old_release), mono: true },
                  { label: "Live now", value: shortRelease(app.release), mono: true },
                  { label: "Front port", value: `:${app.front_port}`, mono: true },
                  { label: "Health gate", value: app.health_url ?? "—", mono: true },
                ]}
              />
            </div>
            <Callout tone="warning" title="This is a production change">
              The old image is retained, so rollback does not need a build. Traffic moves after
              the candidate answers its health check.
            </Callout>
            {rollback.isError ? (
              <Callout tone="danger" title="Rollback was refused">
                {errorText(rollback.error)}
              </Callout>
            ) : null}
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setRollbackOpen(false)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              disabled={rollback.isPending || !canRollback(app)}
              onClick={() => rollback.mutate()}
            >
              <RotateCcw />
              {rollback.isPending ? "Rolling back…" : "Roll back now"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {tab !== "configuration" ? (
        <div className="mt-4 flex items-center justify-between border-t border-border/60 pt-3 text-2xs text-subtle-foreground">
          <span>
            Registry entry for <span className="font-mono">{app.name}</span> ·{" "}
            {app.compose_svc ? `compose service ${app.compose_svc}` : "managed by swapdock"}
          </span>
          <button
            type="button"
            onClick={() => openTab("configuration")}
            className="inline-flex items-center gap-1 transition-colors hover:text-muted-foreground"
          >
            Edit registry
            <ArrowRight className="size-3" />
          </button>
        </div>
      ) : null}
    </div>
  )
}
