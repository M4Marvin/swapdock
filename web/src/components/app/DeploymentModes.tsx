import { Link } from "@tanstack/react-router"
import { useQuery } from "@tanstack/react-query"
import {
  ArrowRight,
  Box,
  Code2,
  ExternalLink,
  FolderOpen,
  PackagePlus,
  Rocket,
} from "lucide-react"
import { api, type App } from "@/api"
import { queryKeys } from "@/lib/query-keys"
import { deploymentKind, needsAdoption } from "@/lib/deployment"
import { appSiteUrl } from "@/lib/hostnames"
import {
  dockerTime,
  imageForRelease,
  releaseImages,
  sameRelease,
} from "@/lib/releases"
import {
  formatRelative,
  formatShortDateTime,
  plural,
  shortRelease,
} from "@/lib/format"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Callout, MetaList } from "@/components/kit"
import { StatusPill } from "@/components/Status"

function ModeCard({
  icon: Icon,
  title,
  description,
  children,
  action,
}: {
  icon: typeof PackagePlus
  title: string
  description: string
  children: React.ReactNode
  action?: React.ReactNode
}) {
  return (
    <section className="overflow-hidden rounded-xl border border-border bg-card shadow-[var(--shadow-card)]">
      <div className="flex flex-wrap items-start justify-between gap-3 border-b border-border/60 px-4 py-3.5">
        <div className="flex min-w-0 items-start gap-3">
          <span className="flex size-8 shrink-0 items-center justify-center rounded-lg border border-border bg-surface text-muted-foreground">
            <Icon className="size-4" />
          </span>
          <div className="min-w-0">
            <h2 className="text-sm font-medium tracking-[-0.01em] text-foreground">{title}</h2>
            <p className="mt-0.5 max-w-2xl text-pretty text-xs leading-relaxed text-muted-foreground">
              {description}
            </p>
          </div>
        </div>
        {action ? <div className="flex shrink-0 items-center gap-2">{action}</div> : null}
      </div>
      <div className="space-y-4 p-4">{children}</div>
    </section>
  )
}

/**
 * A replace app has no live port yet, so the backend refuses a normal deploy.
 * The screenshot failure mode: a version picker framed the problem as "no
 * release" when the real next move is taking over the running container.
 */
export function AdoptionRequired({
  app,
  onAdopt,
}: {
  app: App
  onAdopt: () => void
}) {
  const inspectQuery = useQuery({
    queryKey: queryKeys.inspect(app.name),
    queryFn: ({ signal }) => api.inspect(app.name, signal),
    retry: 0,
  })
  const container = inspectQuery.data?.container
  const imageRepoMissing = app.image_repo == null

  return (
    <ModeCard
      icon={PackagePlus}
      title="Take over this app"
      description="This app is still running outside swapdock. Until nginx owns a live port, there is nothing safe to replace and no version to deploy."
      action={
        <Button size="sm" onClick={onAdopt}>
          <PackagePlus />
          Take over app
        </Button>
      }
    >
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="rounded-xl border border-border bg-surface/40 p-3">
          <div className="label-eyebrow">Current arrangement</div>
          <MetaList
            className="mt-2"
            columns={1}
            items={[
              { label: "Front port", value: `:${app.front_port}`, mono: true },
              { label: "Strategy", value: app.strategy },
              {
                label: "Running container",
                value: container?.name ?? (inspectQuery.isError ? "host unreachable" : "not found"),
                mono: Boolean(container?.name),
              },
              { label: "Image", value: container?.image ?? "—", mono: true },
            ]}
          />
        </div>
        <div className="space-y-2">
          <div className="rounded-xl border border-border bg-muted/20 p-3">
            <div className="label-eyebrow">Why take over first?</div>
            <p className="mt-1.5 text-pretty text-xs leading-relaxed text-muted-foreground">
              The <span className="font-mono">replace</span> strategy stops and starts one
              container on a single port. swapdock needs an existing live port to replace.
              Adoption records the running image, moves nginx in front once, and gives the
              app a managed slot.
            </p>
            <p className="mt-2 text-pretty text-xs leading-relaxed text-muted-foreground">
              No commit hash is involved. The running image is snapshotted as a generated
              version, so rollback still works.
            </p>
          </div>
          {container == null && inspectQuery.isSuccess ? (
            <Callout tone="warning" title="Nothing is publishing the front port">
              Start the app or its legacy container first. Adoption cannot take over an app
              that is not running.
            </Callout>
          ) : null}
        </div>
      </div>

      {imageRepoMissing ? (
        <Callout
          tone="warning"
          title="An image repository is required for adoption"
          action={
            <Button
              variant="outline"
              size="xs"
              render={
                <Link
                  to="/apps/$appName"
                  params={{ appName: app.name }}
                  search={{ tab: "configuration" }}
                />
              }
            >
              Configure
              <ArrowRight />
            </Button>
          }
        >
          swapdock preserves the running image under an image repository. Add one under
          Configuration before taking over.
        </Callout>
      ) : null}

      <div className="flex flex-wrap items-center gap-2 border-t border-border/60 pt-3 text-2xs text-subtle-foreground">
        <span>Prefer a manual cutover?</span>
        <Link
          to="/apps/$appName"
          params={{ appName: app.name }}
          search={{ tab: "operations" }}
          className="inline-flex items-center gap-1 text-muted-foreground transition-colors hover:text-foreground"
        >
          Open operations
          <ArrowRight className="size-3" />
        </Link>
      </div>
    </ModeCard>
  )
}

/** An app whose versions are local image tags, not commits. */
export function ImageDeployment({
  app,
  onDeploy,
}: {
  app: App
  onDeploy: () => void
}) {
  const imagesQuery = useQuery({
    queryKey: queryKeys.images(app.name),
    queryFn: ({ signal }) => api.appImages(app.name, signal),
    enabled: app.image_repo != null,
    retry: 0,
  })
  const images = imagesQuery.data?.images ?? []
  const versions = releaseImages(images).slice(0, 6)
  const liveImage = imageForRelease(images, app.release)

  return (
    <ModeCard
      icon={Box}
      title="Image deployment"
      description="This app is not built from a source checkout. Its versions are local image tags, so there is no build stage and no commit hash."
      action={
        <Button size="sm" variant="outline" onClick={onDeploy}>
          <Rocket />
          Deploy a version
        </Button>
      }
    >
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="rounded-xl border border-border bg-surface/40 p-3">
          <div className="flex items-center justify-between gap-2">
            <span className="label-eyebrow">Live now</span>
            {app.live_port != null ? (
              <StatusPill tone="success" label={`port ${app.live_port}`} />
            ) : (
              <StatusPill tone="neutral" label="not live" />
            )}
          </div>
          <div className="mt-2 space-y-1.5">
            <div className="truncate font-mono text-xs text-foreground">
              {app.image_repo ? `${app.image_repo}:${app.release ?? "?"}` : "no image repository"}
            </div>
            <div className="text-2xs text-subtle-foreground">
              {liveImage?.created ? `Built ${formatShortDateTime(liveImage.created)}` : "No local image recorded for this release."}
            </div>
          </div>
        </div>
        <div className="rounded-xl border border-border bg-surface/40 p-3">
          <div className="label-eyebrow">How updates work</div>
          <p className="mt-1.5 text-pretty text-xs leading-relaxed text-muted-foreground">
            swapdock can redeploy a version that exists in the local Docker daemon. Updating
            the upstream image is done outside swapdock (pull/tag); once it is local, it
            appears in the deploy picker.
          </p>
        </div>
      </div>

      <div className="border-t border-border/60 pt-3">
        <div className="flex items-center justify-between gap-3">
          <span className="label-eyebrow">Local versions</span>
          <span className="text-2xs text-subtle-foreground">
            {imagesQuery.data ? plural(versions.length, "version") : "reading daemon…"}
          </span>
        </div>
        {imagesQuery.isError ? (
          <p className="mt-2 text-xs text-muted-foreground">
            The local image list could not be read.
          </p>
        ) : versions.length === 0 ? (
          <p className="mt-2 text-xs text-muted-foreground">
            No local release tags exist for this image repository.
          </p>
        ) : (
          <div className="mt-2 space-y-1">
            {versions.map((image) => {
              const live = sameRelease(image.tag, app.release)
              const created = dockerTime(image.created)
              return (
                <div
                  key={image.tag}
                  className="flex items-center gap-2 rounded-md px-2 py-1.5 text-xs transition-colors hover:bg-muted/40"
                >
                  <Box className="size-3 shrink-0 text-subtle-foreground" />
                  <span className="min-w-0 flex-1 truncate text-muted-foreground">
                    {created ? formatShortDateTime(created) : "build time unknown"}
                    <span className="ml-2 text-2xs text-subtle-foreground">
                      {created ? formatRelative(created) : ""}
                    </span>
                  </span>
                  {image.size ? (
                    <span className="text-2xs text-subtle-foreground">{image.size}</span>
                  ) : null}
                  <span className="font-mono text-2xs text-subtle-foreground" title={image.tag}>
                    {image.tag.slice(0, 7)}
                  </span>
                  {live ? <Badge variant="success">live</Badge> : null}
                </div>
              )
            })}
          </div>
        )}
      </div>
    </ModeCard>
  )
}

export function StaticDeployment({ app }: { app: App }) {
  const hostname = app.hostnames[0]
  return (
    <ModeCard
      icon={FolderOpen}
      title="Static release"
      description="This app serves files directly from nginx. A release is an atomic root symlink, not a container image or a commit build."
      action={
        hostname ? (
          <Button
            variant="outline"
            size="sm"
            render={<a href={appSiteUrl(app) ?? undefined} target="_blank" rel="noreferrer" />}
          >
            <ExternalLink />
            Open site
          </Button>
        ) : undefined
      }
    >
      <MetaList
        columns={2}
        items={[
          { label: "Root", value: app.root ?? "not configured", mono: true },
          { label: "Release", value: app.release ? shortRelease(app.release) : "not recorded", mono: true },
          { label: "Previous", value: app.old_release ? shortRelease(app.old_release) : "—", mono: true },
          { label: "SPA fallback", value: app.spa ? "enabled" : "off" },
        ]}
      />
      <Callout tone="neutral" title="Published outside this deploy pipeline">
        Static releases are shipped by the static app's release process (the symlink flip).
        There is no container build or health gate to run here.
      </Callout>
    </ModeCard>
  )
}

export function ComposeDeployment({ app }: { app: App }) {
  return (
    <ModeCard
      icon={Code2}
      title="Compose-managed app"
      description="No source checkout or image repository is recorded, so swapdock cannot build or identify a deployable version for this app."
      action={
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
          Open configuration
          <ArrowRight />
        </Button>
      }
    >
      <MetaList
        columns={2}
        items={[
          { label: "Compose directory", value: app.compose_dir ?? "—", mono: true },
          { label: "Service", value: app.compose_svc ?? "—", mono: true },
          { label: "Image repository", value: app.image_repo ?? "not set" },
          { label: "Front port", value: `:${app.front_port}`, mono: true },
        ]}
      />
      <Callout tone="info" title="Add a deployment source">
        Set a git repository to build from source, or an image repository to deploy prebuilt
        images. Then this tab offers versions instead of a dead pipeline.
      </Callout>
    </ModeCard>
  )
}

/**
 * The Pipeline tab, selected by deployment origin and lifecycle.
 * A source app uses the real build pipeline; everything else gets the action
 * that actually applies to it.
 */
export function DeploymentModeView({
  app,
  onAdopt,
  onDeploy,
  pipeline,
}: {
  app: App
  onAdopt: () => void
  onDeploy: () => void
  pipeline: React.ReactNode
}) {
  if (needsAdoption(app)) {
    return <AdoptionRequired app={app} onAdopt={onAdopt} />
  }
  const kind = deploymentKind(app)
  if (kind === "source") return <>{pipeline}</>
  if (kind === "image") return <ImageDeployment app={app} onDeploy={onDeploy} />
  if (kind === "static") return <StaticDeployment app={app} />
  return <ComposeDeployment app={app} />
}
