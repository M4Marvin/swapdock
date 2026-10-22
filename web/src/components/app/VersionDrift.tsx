import { Link } from "@tanstack/react-router"
import {
  ArrowRight,
  ArrowUpRight,
  CircleHelp,
  FolderOpen,
  GitBranch,
  Hammer,
  RefreshCw,
  Rocket,
  TriangleAlert,
} from "lucide-react"
import { ApiError, type App } from "@/api"
import { deploymentKind } from "@/lib/deployment"
import { useVersionDrift, type VersionDriftView } from "@/lib/version-drift"
import { sameRelease } from "@/lib/releases"
import { errorText, shortRelease } from "@/lib/format"
import { appSiteUrl } from "@/lib/hostnames"
import { cn } from "cn"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { Callout, RelativeTime, SectionHeading } from "@/components/kit"
import { StatusDot, StatusPill } from "@/components/Status"


function behindLabel(count: number | null): string {
  if (count == null) return "New version available"
  return `${count} commit${count === 1 ? "" : "s"} behind`
}

function headline(view: VersionDriftView): string {
  switch (view.state) {
    case "up_to_date":
      return `Up to date with ${view.latestBranch}`
    case "behind_built":
    case "behind_unbuilt":
      return behindLabel(view.commitsBehind)
    case "not_deployed":
      return "Not deployed yet"
    case "no_source":
      return "No source configured"
    case "live_unbuilt":
      return "Live version is not built"
    case "static_served":
      return "Static, served"
    default:
      return view.authoritative ? "Version state unknown" : "Check source"
  }
}

function detail(view: VersionDriftView, app: App): string {
  if (view.message) return view.message
  switch (view.state) {
    case "up_to_date":
      return view.latestSubject
        ? `Latest commit: ${view.latestSubject}`
        : "The live release matches the last fetched source."
    case "behind_built":
      return "The newest source version is already built and ready to deploy."
    case "behind_unbuilt":
      return app.kind === "static"
        ? "The latest static release is not staged. Place files in releases/<release> beside the root, then deploy."
        : "The newest source version needs an image build before it can go live."
    case "not_deployed":
      return "No release is behind the front door yet."
    case "no_source":
      return deploymentKind(app) === "image"
        ? "This app deploys image tags, not commits. Add a source checkout to track version drift."
        : "No checkout is recorded, so the latest source version cannot be resolved."
    case "live_unbuilt":
      return "The running version has no local image; rebuild before the next deploy or rollback."
    case "static_served":
      return "Files are served directly from the configured root."
    default:
      return view.authoritative
        ? "The API could not resolve the latest source version."
        : "Showing the last fetched source and local image state; this does not confirm the deployment is current."
  }
}

function sourceTitle(view: VersionDriftView): string {
  if (view.latestSource === "HEAD") return "Latest from the checkout"
  return `Latest from ${view.latestBranch}`
}

function imageLabel(view: VersionDriftView, app: App): string {
  if (app.kind === "static") {
    if (view.imageReady === true) return "Release ready"
    if (view.imageReady === false) return "Not staged"
    return "Release state unknown"
  }
  if (view.imageReady === true) return "Image ready"
  if (view.imageReady === false) return "Needs build"
  return "Build state unknown"
}

function healthLine(view: VersionDriftView, app: App): string {
  if (app.kind === "static" && app.root != null) return "Static, served"
  if (view.livePort == null) return "No live port"
  const health =
    view.liveUp === true ? "live" : view.liveUp === false ? "down" : "health unknown"
  return `${health} · port ${view.livePort}`
}

/** Compact header badge: "3 behind" / "Up to date" / "Check source". */
export function VersionDriftBadge({ app }: { app: App }) {
  const view = useVersionDrift(app)
  if (view.loading) return null

  const behind = view.state === "behind_built" || view.state === "behind_unbuilt"
  const label =
    view.state === "up_to_date"
      ? "Up to date"
      : behind
        ? view.commitsBehind == null
          ? "Update available"
          : `${view.commitsBehind} behind`
        : view.state === "not_deployed"
          ? "Not deployed"
          : view.state === "no_source"
            ? "No source"
            : view.state === "live_unbuilt"
              ? "Needs build"
              : view.state === "static_served"
                ? "Static, served"
                : "Check source"

  const variant =
    view.state === "up_to_date"
      ? "success"
      : view.state === "behind_built"
        ? "accent"
        : view.state === "behind_unbuilt" || view.state === "live_unbuilt"
          ? "warning"
          : view.state === "static_served"
            ? "info"
            : "outline"

  const title = [
    view.state === "up_to_date"
      ? `Up to date with ${view.latestBranch}`
      : headline(view),
    view.latestSubject ?? null,
    view.latestCommittedAt ? `Committed ${new Date(view.latestCommittedAt).toLocaleString()}` : null,
    view.latestRelease ? `Latest ${view.latestRelease}` : null,
    view.liveRelease ? `Live ${view.liveRelease}` : null,
    view.imageReady === true
      ? "Image ready"
      : view.imageReady === false
        ? "Needs build"
        : null,
    view.authoritative ? "Checked against source drift" : "Version check unavailable; open the app to check the source",
  ]
    .filter(Boolean)
    .join(" · ")

  return (
    <Badge variant={variant} title={title}>
      {view.state === "behind_built" ? (
        <ArrowUpRight data-icon="inline-start" />
      ) : view.state === "behind_unbuilt" ? (
        <TriangleAlert data-icon="inline-start" />
      ) : view.state === "up_to_date" ? (
        <StatusDot tone="success" />
      ) : view.state === "unknown" ? (
        <CircleHelp data-icon="inline-start" />
      ) : null}
      {label}
      <span className="sr-only">. {title}</span>
    </Badge>
  )
}

/**
 * The version card: current deployment against the latest fetched source.
 *
 * This is the operator-facing answer to "is this app stale?" and the one place
 * that turns drift into an action: deploy a ready image, build an unbuilt one,
 * or go fix the missing source.
 */
export function VersionDriftCard({
  app,
  onDeploy,
  onBuild,
  onOpenPipeline,
}: {
  app: App
  onDeploy: (release: string) => void
  onBuild: (release: string) => void
  onOpenPipeline: () => void
}) {
  const view = useVersionDrift(app)
  const siteUrl = appSiteUrl(app)
  const configurationLink = (
    <Link to="/apps/$appName" params={{ appName: app.name }} search={{ tab: "configuration" }} />
  )

  if (view.loading) {
    return (
      <section
        aria-label="Version"
        className="overflow-hidden rounded-xl border border-border bg-card shadow-[var(--shadow-card)]"
      >
        <div className="border-b border-border/60 px-4 py-3.5">
          <SectionHeading title="Version" description="Current deployment against the latest source." />
        </div>
        <div className="space-y-3 p-4">
          <Skeleton className="h-3 w-28" />
          <Skeleton className="h-3 w-full max-w-sm" />
          <Skeleton className="h-3 w-2/3" />
        </div>
      </section>
    )
  }

  const primaryAction = (() => {
    switch (view.state) {
      case "behind_built":
        return view.latestRelease ? (
          <Button size="sm" onClick={() => onDeploy(view.latestRelease as string)}>
            <Rocket />
            Deploy latest
          </Button>
        ) : null
      case "behind_unbuilt":
        return view.latestRelease ? (
          app.kind === "static" ? (
            <Button variant="outline" size="sm" onClick={onOpenPipeline}>
              <Hammer />
              Prep release
            </Button>
          ) : (
            <Button size="sm" onClick={() => onBuild(view.latestRelease as string)}>
              <Hammer />
              Build &amp; deploy
            </Button>
          )
        ) : null
      case "not_deployed":
        return view.latestRelease ? (
          <Button size="sm" onClick={() => onDeploy(view.latestRelease as string)}>
            <Rocket />
            Deploy latest
          </Button>
        ) : (
          <Button variant="outline" size="sm" onClick={onOpenPipeline}>
            Open pipeline
            <ArrowRight />
          </Button>
        )
      case "live_unbuilt":
        return view.latestRelease && !sameRelease(view.latestRelease, view.liveRelease) ? (
          <Button size="sm" onClick={() => onBuild(view.latestRelease as string)}>
            <Hammer />
            Build &amp; deploy
          </Button>
        ) : (
          <Button variant="outline" size="sm" onClick={onOpenPipeline}>
            Open pipeline
            <ArrowRight />
          </Button>
        )
      case "no_source":
        return (
          <Button variant="outline" size="sm" render={configurationLink}>
            <FolderOpen />
            Open Configuration
          </Button>
        )
      case "static_served":
        return siteUrl ? (
          <Button
            variant="outline"
            size="sm"
            render={<a href={siteUrl} target="_blank" rel="noreferrer" />}
          >
            <ArrowUpRight />
            Open site
          </Button>
        ) : null
      case "up_to_date":
        return null
      default:
        return (
          <Button variant="outline" size="sm" onClick={view.retry} disabled={view.checking}>
            <RefreshCw className={cn(view.checking && "animate-spin")} />
            {view.checking ? "Checking source…" : "Check source"}
          </Button>
        )
    }
  })()

  const secondaryAction =
    view.state === "unknown" || !view.authoritative ? (
      <Button variant="ghost" size="sm" onClick={onOpenPipeline}>
        Open pipeline
        <ArrowRight />
      </Button>
    ) : null

  return (
    <section
      aria-label="Version"
      className="overflow-hidden rounded-xl border border-border bg-card shadow-[var(--shadow-card)]"
    >
      <div className="flex flex-wrap items-start justify-between gap-3 border-b border-border/60 px-4 py-3.5">
        <SectionHeading
          title="Version"
          description="The current deployment against the latest fetched source."
        />
        {view.state === "up_to_date" ? (
          <StatusPill tone="success" label="Up to date" />
        ) : view.state === "behind_built" || view.state === "behind_unbuilt" ? (
          <StatusPill tone="warning" label="Update available" />
        ) : null}
      </div>

      <div className="space-y-4 p-4">
        <div className="grid gap-3 sm:grid-cols-2">
          <div className="min-w-0 rounded-lg border border-border bg-surface/40 p-3">
            <div className="label-eyebrow">Current version</div>
            <div className="mt-1 flex min-w-0 items-baseline gap-2">
              {view.liveRelease ? (
                <span
                  className="truncate font-mono text-sm font-medium text-foreground"
                  title={view.liveRelease}
                >
                  {shortRelease(view.liveRelease)}
                </span>
              ) : (
                <span className="text-sm text-foreground">
                  {view.state === "static_served" ? "Served from root" : "Nothing live"}
                </span>
              )}
            </div>
            <div className="mt-1 flex items-center gap-1.5 text-2xs text-muted-foreground">
              {view.livePort != null || view.state === "static_served" ? (
                <StatusDot
                  tone={
                    view.state === "static_served"
                      ? "info"
                      : view.liveUp === true
                        ? "success"
                        : view.liveUp === false
                          ? "danger"
                          : "neutral"
                  }
                />
              ) : null}
              <span className="truncate">{healthLine(view, app)}</span>
            </div>
          </div>

          <div className="min-w-0 rounded-lg border border-border bg-surface/40 p-3">
            <div className="flex items-center gap-1.5">
              <GitBranch className="size-3 text-subtle-foreground" />
              <div className="label-eyebrow">{sourceTitle(view)}</div>
            </div>
            <div className="mt-1 flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-1">
              {view.latestRelease ? (
                <span
                  className="truncate font-mono text-sm font-medium text-foreground"
                  title={view.latestRelease}
                >
                  {shortRelease(view.latestRelease)}
                </span>
              ) : (
                <span className="text-sm text-foreground">Not resolved</span>
              )}
              {view.latestRelease ? (
                <span
                  className={cn(
                    "inline-flex items-center gap-1 rounded-full border px-1.5 py-0.5 text-[11px]",
                    view.imageReady === true
                      ? "border-success/20 bg-success/10 text-success-strong"
                      : view.imageReady === false
                        ? "border-warning/25 bg-warning/10 text-warning-strong"
                        : "border-border bg-muted/50 text-muted-foreground"
                  )}
                >
                  {imageLabel(view, app)}
                </span>
              ) : null}
            </div>
            <div className="mt-1 space-y-0.5 text-2xs text-muted-foreground">
              {view.latestSubject ? (
                <p className="line-clamp-2 text-pretty leading-relaxed">{view.latestSubject}</p>
              ) : null}
              <p className="text-subtle-foreground">
                {view.latestCommittedAt ? (
                  <>
                    Committed <RelativeTime value={view.latestCommittedAt} />
                  </>
                ) : view.latestSource ? (
                  `Last fetched ${view.latestSource}`
                ) : (
                  "No commit details available"
                )}
              </p>
            </div>
          </div>
        </div>

        <div className="flex flex-wrap items-end justify-between gap-3 border-t border-border/60 pt-3">
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <h3 className="text-sm font-medium text-foreground">{headline(view)}</h3>
              {view.state === "behind_built" || view.state === "behind_unbuilt" ? (
                <span className="text-2xs text-subtle-foreground">
                  {view.state === "behind_built" ? "Image ready" : "Needs build"}
                </span>
              ) : null}
            </div>
            <p className="mt-0.5 max-w-2xl text-pretty text-xs leading-relaxed text-muted-foreground">
              {detail(view, app)}
            </p>
            {view.latestCommittedAt && view.state === "up_to_date" ? null : view.latestSubject ? (
              <p className="mt-1 text-2xs text-subtle-foreground">
                Latest commit: {view.latestSubject}
                {view.latestCommittedAt ? (
                  <>
                    {" · "}
                    <RelativeTime value={view.latestCommittedAt} />
                  </>
                ) : null}
              </p>
            ) : null}
          </div>
          <div className="flex shrink-0 flex-wrap items-center gap-2">
            {secondaryAction}
            {primaryAction}
          </div>
        </div>

        {!view.authoritative ? (
          <Callout
            tone="warning"
            title="Version check unavailable"
            action={
              <Button variant="outline" size="xs" onClick={view.retry} disabled={view.checking}>
                <RefreshCw className={cn(view.checking && "animate-spin")} />
                {view.checking ? "Checking…" : "Retry"}
              </Button>
            }
          >
            {view.error instanceof ApiError && view.error.status === 404
              ? "This API does not expose the drift check yet. "
              : `${errorText(view.error)}. `}
            Showing the last fetched source and local image state below. This does not confirm
            that the deployment is current.
          </Callout>
        ) : null}

        <p className="text-2xs leading-relaxed text-subtle-foreground">
          {view.latestSource === "HEAD"
            ? "Source resolves from the local checkout because the upstream ref is unavailable."
            : `Source is read from the fetched ${view.latestSource ?? `origin/${view.latestBranch}`}; reading the version does not run a fetch. Use Sync to update it.`}
        </p>
      </div>
    </section>
  )
}