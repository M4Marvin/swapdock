import { Link } from "@tanstack/react-router"
import {
  ArrowDown,
  ArrowRight,
  ExternalLink,
  Globe,
  RotateCcw,
  Server,
  Settings2,
} from "lucide-react"
import type { App } from "@/api"
import { cn } from "cn"
import { shortRelease } from "@/lib/format"
import { sameRelease } from "@/lib/releases"
import { appSiteUrl } from "@/lib/hostnames"
import { HealthPill, StatusPill, isAppUp, useAppHealth } from "@/components/Status"
import { Callout, MetaList, SectionHeading } from "@/components/kit"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"

function Connector() {
  return (
    <>
      <div className="hidden items-center md:flex" aria-hidden="true">
        <span className="h-px w-full bg-border" />
        <ArrowRight className="-ml-1 size-3 shrink-0 text-subtle-foreground" />
      </div>
      <div className="flex justify-center py-0.5 md:hidden" aria-hidden="true">
        <ArrowDown className="size-3 text-subtle-foreground" />
      </div>
    </>
  )
}

function Slot({
  eyebrow,
  icon: Icon,
  title,
  subtitle,
  footer,
  tone = "neutral",
  className,
}: {
  eyebrow: string
  icon: typeof Globe
  title: React.ReactNode
  subtitle: React.ReactNode
  footer?: React.ReactNode
  tone?: "neutral" | "success" | "danger" | "warning"
  className?: string
}) {
  const toneClass =
    tone === "success"
      ? "border-success/25 bg-success/[0.035]"
      : tone === "danger"
        ? "border-destructive/30 bg-destructive/[0.035]"
        : tone === "warning"
          ? "border-warning/25 bg-warning/[0.03]"
          : "border-border bg-surface"
  return (
    <div
      className={cn(
        "flex min-w-0 flex-col justify-between gap-3 rounded-lg border p-3",
        toneClass,
        className
      )}
    >
      <div className="flex items-center gap-1.5">
        <Icon className="size-3 text-subtle-foreground" />
        <span className="label-eyebrow">{eyebrow}</span>
      </div>
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <span className="truncate text-sm font-medium text-foreground">{title}</span>
          {tone === "success" ? <span className="size-1.5 shrink-0 rounded-full bg-success" /> : null}
        </div>
        <div className="mt-0.5 truncate text-xs text-muted-foreground">{subtitle}</div>
      </div>
      {footer ? <div className="flex items-center gap-2">{footer}</div> : null}
    </div>
  )
}

/**
 * The blue/green topology, at a glance: one stable front port, a live backend,
 * and the previous slot kept for rollback. This is the app's mental model, so
 * it gets the largest card on the overview.
 */
export function DeploymentCard({ app }: { app: App }) {
  const isStatic = app.kind === "static"
  const staticRoot = isStatic && app.root != null
  const live = app.live_port != null || staticRoot
  const health = useAppHealth(app.name, app.live_port)
  const previousAvailable =
    app.old_release != null && !sameRelease(app.release, app.old_release)
  const up = app.live_port != null && isAppUp(health.data)
  const down = app.live_port != null && health.data != null && !up
  const hostname = app.hostnames[0]
  const siteUrl = appSiteUrl(app)
  const healthTone = live
    ? app.live_port != null
      ? up
        ? "success"
        : down
          ? "danger"
          : "warning"
      : "success"
    : "neutral"

  return (
    <section className="overflow-hidden rounded-xl border border-border bg-card shadow-[var(--shadow-card)]">
      <div className="flex flex-wrap items-start justify-between gap-3 border-b border-border/60 px-4 py-3.5">
        <SectionHeading
          title="Deployment"
          description={
            isStatic
              ? "One stable front door and an atomic static root."
              : "One stable front door, one live slot, one rollback slot."
          }
        />
        <div className="flex items-center gap-2">
          <Badge variant="outline" className="font-mono lowercase">
            {app.strategy}
          </Badge>
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label="Edit routing configuration"
            title="Edit routing configuration"
            render={<Link to="/apps/$appName" params={{ appName: app.name }} search={{ tab: "configuration" }} />}
          >
            <Settings2 />
          </Button>
        </div>
      </div>

      <div className="space-y-4 p-4">
        <div className="grid items-stretch gap-1.5 md:grid-cols-[minmax(0,1fr)_34px_minmax(0,1fr)_34px_minmax(0,1fr)]">
          <Slot
            eyebrow="Front door"
            icon={Globe}
            title={`:${app.front_port}`}
            subtitle={hostname ?? "No hostname routes here yet"}
            footer={
              hostname ? (
                <a
                  href={siteUrl ?? undefined}
                  target="_blank"
                  rel="noreferrer"
                  className="inline-flex items-center gap-1 text-2xs text-muted-foreground transition-colors hover:text-foreground"
                >
                  Open site
                  <ExternalLink className="size-2.5" />
                </a>
              ) : (
                <span className="text-2xs text-subtle-foreground">nginx owns this port</span>
              )
            }
          />
          <Connector />
          <Slot
            eyebrow={isStatic ? "Live release" : "Live slot"}
            icon={Server}
            title={live ? (isStatic ? "Serving" : `:${app.live_port}`) : "Empty"}
            subtitle={
              isStatic
                ? (app.root ?? "No root configured")
                : app.release
                  ? `${shortRelease(app.release)} · ${app.strategy === "replace" ? "replaced in place" : "green slot"}`
                  : "No release is live"
            }
            tone={
              healthTone === "success" ? "success" : healthTone === "danger" ? "danger" : "neutral"
            }
            footer={
              isStatic ? (
                <StatusPill
                  tone={staticRoot ? "info" : "neutral"}
                  label={staticRoot ? "Static root" : "No root"}
                />
              ) : live ? (
                <HealthPill name={app.name} livePort={app.live_port} showCode />
              ) : (
                <StatusPill tone="neutral" label="Not serving" />
              )
            }
          />
          <Connector />
          <Slot
            eyebrow="Previous"
            icon={RotateCcw}
            title={previousAvailable ? shortRelease(app.old_release) : "—"}
            subtitle={
              previousAvailable
                ? app.old_port != null
                  ? `Kept on :${app.old_port}`
                  : "Image retained for rollback"
                : "No previous version retained"
            }
            footer={
              previousAvailable ? (
                <span className="text-2xs text-muted-foreground">Rollback available</span>
              ) : (
                <span className="text-2xs text-subtle-foreground">
                  {app.release ? "Current and previous are the same" : "First deploy has no previous"}
                </span>
              )
            }
          />
        </div>

        {!live ? (
          <Callout
            tone="warning"
            title="Nothing is serving behind the front door"
            action={
              <Button
                variant="outline"
                size="xs"
                render={
                  <Link
                    to="/apps/$appName"
                    params={{ appName: app.name }}
                    search={{ tab: "pipeline" }}
                  />
                }
              >
                Open pipeline
                <ArrowRight />
              </Button>
            }
          >
            nginx has no live backend for this app. Build and deploy a release to fill the
            live slot; the front port never changes.
          </Callout>
        ) : null}

        <div className="border-t border-border/60 pt-3.5">
          <MetaList
            columns={4}
            items={[
              {
                label: "Image",
                value:
                  app.kind === "static"
                    ? app.root ?? "—"
                    : app.image_repo
                      ? `${app.image_repo}:${shortRelease(app.release)}`
                      : "—",
                mono: true,
                title: app.image_repo && app.release ? `${app.image_repo}:${app.release}` : undefined,
              },
              {
                label: "Compose",
                value: app.compose_svc
                  ? `${app.compose_svc}${app.compose_dir ? ` · ${app.compose_dir}` : ""}`
                  : "—",
                mono: true,
                title: app.compose_dir ?? undefined,
              },
              {
                label: "Health check",
                value: app.health_url ?? "—",
                mono: true,
                title: app.health_url ?? undefined,
              },
              {
                label: "Public URL",
                value: app.public_url ?? "not set",
                mono: Boolean(app.public_url),
                href: app.public_url ?? undefined,
              },
            ]}
          />
        </div>
      </div>
    </section>
  )
}
