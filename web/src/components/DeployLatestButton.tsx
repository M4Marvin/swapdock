import { useMemo, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { api, type App, type AppImage } from '../api'
import { queryKeys } from '@/lib/query-keys'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'

/** Release tags are commit SHAs; `latest` and rollback tags are not releases. */
const RELEASE_RE = /^[0-9a-fA-F]{4,64}$/

/** Parses Docker's `YYYY-MM-DD HH:MM:SS +0000 UTC`, with a stable fallback. */
function imageTime(image: AppImage, index: number): number {
  if (!image.created) return index
  const parsed = Date.parse(image.created.replace(' UTC', ''))
  return Number.isNaN(parsed) ? index : parsed
}

/**
 * The newest local image tag that is a deployable release.
 *
 * Docker lists newest first, but the timestamps are explicit, so sort on them
 * and keep the list order as the tie-breaker. `latest` and `rollback-pre-*`
 * tags are moving/bookkeeping tags, not releases.
 */
export function newestReleaseImage(images: AppImage[]): AppImage | null {
  const candidates = images.filter((image) => RELEASE_RE.test(image.tag))
  if (candidates.length === 0) return null
  return candidates.reduce((best, current, index) => {
    const bestIndex = images.indexOf(best)
    return imageTime(current, index) > imageTime(best, bestIndex)
      ? current
      : best
  })
}

/** True when two commit names describe the same release, short or full. */
function sameRelease(a: string | null, b: string): boolean {
  if (a == null) return false
  return a === b || a.startsWith(b) || b.startsWith(a)
}

/**
 * One-click deploy of the newest image already built for this app.
 *
 * This skips Build/Transfer: it is for an operator who knows the image is on
 * the host and wants the normal deploy gate without re-entering a SHA.
 */
export function DeployLatestButton({
  app,
  onStarted,
}: {
  app: App
  onStarted?: (runId: string) => void
}) {
  const queryClient = useQueryClient()
  const [open, setOpen] = useState(false)

  const imagesQuery = useQuery({
    queryKey: queryKeys.images(app.name),
    queryFn: ({ signal }) => api.appImages(app.name, signal),
    enabled: app.kind === 'container' && app.image_repo != null,
  })

  const latest = useMemo(
    () => newestReleaseImage(imagesQuery.data?.images ?? []),
    [imagesQuery.data],
  )
  const isLive = latest != null && sameRelease(app.release, latest.tag)

  const deployMutation = useMutation({
    mutationFn: (release: string) => api.deploy(app.name, release),
    onSuccess: ({ run_id }) => {
      toast.success(`deploy started: ${run_id}`)
      setOpen(false)
      onStarted?.(run_id)
      queryClient.invalidateQueries({ queryKey: queryKeys.app(app.name) })
      queryClient.invalidateQueries({ queryKey: queryKeys.apps })
      queryClient.invalidateQueries({ queryKey: queryKeys.runs })
      queryClient.invalidateQueries({ queryKey: queryKeys.appStatus(app.name) })
      queryClient.invalidateQueries({ queryKey: queryKeys.latest(app.name) })
    },
    onError: (error) => {
      toast.error('deploy failed', {
        description: error instanceof Error ? error.message : String(error),
      })
    },
  })

  if (app.kind !== 'container' || app.image_repo == null) return null

  const reason =
    app.live_port == null
      ? 'adopt this app before deploying an image'
      : imagesQuery.isError
        ? 'could not list local images'
        : imagesQuery.isLoading
          ? 'looking for the newest image'
          : latest == null
            ? 'no built release image found'
            : isLive
              ? 'the newest image is already live'
              : undefined

  return (
    <>
      <Button
        variant="outline"
        disabled={reason != null || deployMutation.isPending}
        title={reason}
        onClick={() => setOpen(true)}
      >
        {deployMutation.isPending
          ? 'Deploying…'
          : latest
            ? `Deploy latest (${latest.tag.slice(0, 12)})`
            : 'Deploy latest'}
      </Button>

      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Deploy latest image?</DialogTitle>
            <DialogDescription>
              This deploys the newest built image for {app.name} through the
              normal replace/swap health gate. It does not build or transfer.
            </DialogDescription>
          </DialogHeader>
          {latest && (
            <div className="space-y-2 text-sm">
              <p>
                <span className="text-muted-foreground">release </span>
                <span className="font-mono">{latest.tag}</span>
              </p>
              <p>
                <span className="text-muted-foreground">image </span>
                <span className="font-mono">{latest.ref}</span>
              </p>
              <p className="flex flex-wrap items-center gap-2">
                <Badge variant="outline">strategy {app.strategy}</Badge>
                <Badge variant="outline">
                  live {app.release ?? 'not deployed'}
                </Badge>
                {latest.created && (
                  <span className="text-xs text-muted-foreground">
                    built {latest.created}
                  </span>
                )}
              </p>
            </div>
          )}
          <DialogFooter>
            <Button variant="outline" onClick={() => setOpen(false)}>
              Cancel
            </Button>
            <Button
              disabled={!latest || deployMutation.isPending}
              onClick={() => latest && deployMutation.mutate(latest.tag)}
            >
              {deployMutation.isPending ? 'Deploying…' : 'Deploy'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}
