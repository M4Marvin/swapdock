import { useMemo } from "react"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"
import { Link } from "@tanstack/react-router"
import {
  ArrowRight,
  Box,
  GitBranch,
  GitCommitHorizontal,
  RefreshCw,
  TriangleAlert,
} from "lucide-react"
import { api, type App } from "@/api"
import { queryKeys } from "@/lib/query-keys"
import { errorText, formatRelative, plural, shortId, shortRelease } from "@/lib/format"
import { cn } from "cn"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { SectionHeading } from "@/components/kit"

const RELEASE_RE = /^[0-9a-fA-F]{4,64}$/

/** Docker reports `YYYY-MM-DD HH:MM:SS +0000 UTC`; Date.parse needs the suffix trimmed. */
function dockerTime(value: string | null): number | null {
  if (!value) return null
  const parsed = Date.parse(value.replace(" UTC", ""))
  return Number.isNaN(parsed) ? null : parsed
}

/**
 * Source state and local images, side by side: what commit the checkout is on,
 * what upstream has, and which release images actually exist to deploy.
 */
export function SourceCard({ app }: { app: App }) {
  const queryClient = useQueryClient()

  const gitQuery = useQuery({
    queryKey: queryKeys.git(app.name),
    queryFn: ({ signal }) => api.appGit(app.name, signal),
    enabled: app.repo != null,
    retry: 0,
  })
  const imagesQuery = useQuery({
    queryKey: queryKeys.images(app.name),
    queryFn: ({ signal }) => api.appImages(app.name, signal),
    enabled: app.kind === "container" && app.image_repo != null,
    retry: 0,
  })
  const latestQuery = useQuery({
    queryKey: queryKeys.latest(app.name),
    queryFn: ({ signal }) => api.appLatest(app.name, signal),
    enabled: app.live_port != null,
    retry: 0,
  })

  const sync = useMutation({
    mutationFn: () => api.sync(app.name),
    onSuccess: ({ run_id }) => {
      toast.success("Source sync started", { description: `run ${shortId(run_id)}` })
      void queryClient.invalidateQueries({ queryKey: queryKeys.git(app.name) })
      void queryClient.invalidateQueries({ queryKey: queryKeys.app(app.name) })
      void queryClient.invalidateQueries({ queryKey: queryKeys.runs })
      void queryClient.invalidateQueries({ queryKey: queryKeys.latest(app.name) })
    },
    onError: (error) => {
      toast.error("Sync did not start", { description: errorText(error) })
    },
  })

  const git = gitQuery.data
  const images = useMemo(
    () =>
      (imagesQuery.data?.images ?? [])
        .filter((image) => RELEASE_RE.test(image.tag))
        .sort((a, b) => (dockerTime(b.created) ?? 0) - (dockerTime(a.created) ?? 0))
        .slice(0, 4),
    [imagesQuery.data]
  )
  const upstream = latestQuery.data?.release
  const behind = upstream != null && app.release != null && !app.release.startsWith(upstream) && !upstream.startsWith(app.release)
  const canSync = app.repo != null && !sync.isPending

  return (
    <section className="overflow-hidden rounded-xl border border-border bg-card shadow-[var(--shadow-card)]">
      <div className="flex items-start justify-between gap-3 border-b border-border/60 px-4 py-3.5">
        <SectionHeading
          title={app.kind === "static" ? "Source & release" : "Source & images"}
          description={
            app.kind === "static"
              ? "The checkout and the symlink root"
              : "The checkout and what is built"
          }
        />
        <Button
          variant="outline"
          size="xs"
          disabled={!canSync}
          title={
            app.repo == null
              ? "No repo is recorded for this app"
              : "Fetch and fast-forward the app checkout"
          }
          onClick={() => sync.mutate()}
        >
          <RefreshCw className={cn(sync.isPending && "animate-spin")} />
          Sync
        </Button>
      </div>

      <div className="space-y-4 p-4">
        {app.repo == null ? (
          <p className="text-xs text-muted-foreground">No source checkout is recorded.</p>
        ) : gitQuery.isPending ? (
          <div className="space-y-2">
            <Skeleton className="h-3.5 w-24" />
            <Skeleton className="h-3 w-full" />
            <Skeleton className="h-3 w-3/4" />
          </div>
        ) : gitQuery.isError ? (
          <div className="flex items-start gap-2 text-xs text-muted-foreground">
            <TriangleAlert className="mt-0.5 size-3.5 shrink-0 text-warning" />
            <span>Source state unavailable: {errorText(gitQuery.error)}</span>
          </div>
        ) : git ? (
          <div className="space-y-2.5">
            <div className="flex flex-wrap items-center gap-2">
              <span className="inline-flex items-center gap-1.5 text-sm text-foreground">
                <GitBranch className="size-3.5 text-subtle-foreground" />
                <span className="font-mono text-xs">{git.branch}</span>
              </span>
              {git.dirty === true ? (
                <Badge variant="warning">dirty checkout</Badge>
              ) : git.dirty === false ? (
                <Badge variant="outline">clean</Badge>
              ) : null}
              {git.error ? <Badge variant="destructive">git error</Badge> : null}
            </div>
            <div className="grid grid-cols-2 gap-2">
              <div className="rounded-lg border border-border bg-surface px-2.5 py-2">
                <div className="label-eyebrow">HEAD</div>
                <div className="mt-0.5 truncate font-mono text-xs" title={git.head_sha ?? undefined}>
                  {git.head_sha ? shortRelease(git.head_sha) : "—"}
                </div>
              </div>
              <div className="rounded-lg border border-border bg-surface px-2.5 py-2">
                <div className="label-eyebrow">Upstream</div>
                <div className="mt-0.5 truncate font-mono text-xs" title={git.remote_sha ?? undefined}>
                  {git.remote_sha ? shortRelease(git.remote_sha) : "—"}
                </div>
              </div>
            </div>
            {git.error ? (
              <p className="text-xs text-destructive-strong">{git.error}</p>
            ) : behind ? (
              <div className="flex items-center gap-1.5 text-2xs text-accent">
                <ArrowRight className="size-3" />
                Upstream moved to {shortRelease(upstream)}
              </div>
            ) : app.release && git.remote_sha && app.release === git.remote_sha ? (
              <p className="text-2xs text-muted-foreground">The live release matches upstream.</p>
            ) : null}
          </div>
        ) : null}

        {app.kind === "static" ? (
          <div className="space-y-2 border-t border-border/60 pt-3.5">
            <div className="flex items-center justify-between gap-3">
              <span className="inline-flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
                <Box className="size-3.5" />
                Static root
              </span>
              {app.spa ? <Badge variant="outline">spa</Badge> : null}
            </div>
            <div className="rounded-lg border border-border bg-surface px-2.5 py-2">
              <div className="label-eyebrow">Served directory</div>
              <div className="mt-0.5 truncate font-mono text-xs text-foreground" title={app.root ?? undefined}>
                {app.root ?? "not configured"}
              </div>
            </div>
            <p className="text-2xs text-muted-foreground">
              {app.release
                ? `Release ${shortRelease(app.release)} is pointed at by the root symlink.`
                : "The first deploy creates the release symlink this root resolves through."}
            </p>
          </div>
        ) : (
        <div className="space-y-2 border-t border-border/60 pt-3.5">
          <div className="flex items-center justify-between gap-3">
            <span className="inline-flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
              <Box className="size-3.5" />
              Local release images
            </span>
            <span className="text-2xs text-subtle-foreground">
              {imagesQuery.data ? plural(imagesQuery.data.images.length, "tag") : ""}
            </span>
          </div>

          {imagesQuery.isPending && app.kind === "container" ? (
            <div className="space-y-1.5">
              <Skeleton className="h-8 w-full" />
              <Skeleton className="h-8 w-full" />
            </div>
          ) : imagesQuery.isError ? (
            <p className="text-xs text-muted-foreground">
              Image list unavailable. {errorText(imagesQuery.error)}
            </p>
          ) : images.length === 0 ? (
            <div className="rounded-lg border border-dashed border-border px-3 py-3">
              <p className="text-xs text-muted-foreground">No built release images found.</p>
              <Link
                to="/apps/$appName"
                params={{ appName: app.name }}
                search={{ tab: "pipeline" }}
                className="mt-1 inline-flex items-center gap-1 text-2xs text-accent hover:underline"
              >
                Build the first release
                <ArrowRight className="size-2.5" />
              </Link>
            </div>
          ) : (
            <div className="space-y-1">
              {images.map((image) => {
                const live =
                  app.release != null &&
                  (image.tag === app.release ||
                    app.release.startsWith(image.tag) ||
                    image.tag.startsWith(app.release))
                return (
                  <div
                    key={image.tag}
                    className="flex items-center gap-2 rounded-md px-1.5 py-1.5 transition-colors hover:bg-muted/50"
                  >
                    <GitCommitHorizontal className="size-3 shrink-0 text-subtle-foreground" />
                    <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-foreground">
                      {image.tag}
                    </span>
                    {live ? <Badge variant="success">live</Badge> : null}
                    <span className="shrink-0 text-2xs text-subtle-foreground">
                      {image.size ?? "—"}
                      {dockerTime(image.created)
                        ? ` · ${formatRelative(dockerTime(image.created) ?? 0)}`
                        : ""}
                    </span>
                  </div>
                )
              })}
            </div>
          )}
        </div>
        )}

        {sync.isError ? (
          <p className="text-xs text-destructive-strong">{errorText(sync.error)}</p>
        ) : null}
      </div>
    </section>
  )
}
