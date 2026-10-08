import { Link } from '@tanstack/react-router'
import { useEffect, useRef, useState } from 'react'
import {
  api,
  cfg,
  formatStatus,
  openRunEvents,
  type App,
  type AppGit,
  type AppImage,
  type RunStream,
  type TraceEvent,
} from '../api'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Skeleton } from '@/components/ui/skeleton'
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'

/** The three stages a release moves through, in order. */
type Stage = 'build' | 'transfer' | 'deploy'

type StageState = 'idle' | 'active' | 'ok' | 'failed'

interface StageInfo {
  state: StageState
  runId: string | null
  /** Step events for this stage, newest appended last. */
  steps: TraceEvent[]
  error: string | null
}

const EMPTY_STAGE: StageInfo = {
  state: 'idle',
  runId: null,
  steps: [],
  error: null,
}

/** A release is a commit SHA: hex, 4-64 chars. */
const RELEASE_RE = /^[0-9a-fA-F]{4,64}$/

/** How many recent step lines to keep visible per stage. */
const VISIBLE_STEPS = 8

/** True for the terminal statuses that mean the stage succeeded. */
function isOk(status: string | null): boolean {
  return status != null && /ok|succeed|dry.?run/i.test(status)
}

function StateBadge({ state }: { state: StageState }) {
  switch (state) {
    case 'active':
      return (
        <Badge variant="secondary" className="gap-1">
          <span className="size-1.5 animate-pulse rounded-full bg-amber-500" />
          running
        </Badge>
      )
    case 'ok':
      return (
        <Badge
          variant="outline"
          className="border-emerald-500/40 bg-emerald-500/10 text-emerald-500"
        >
          ok
        </Badge>
      )
    case 'failed':
      return <Badge variant="destructive">failed</Badge>
    default:
      return (
        <Badge variant="outline" className="text-muted-foreground">
          idle
        </Badge>
      )
  }
}

/** Compact one-line status for a streamed step. */
function StepLine({ step }: { step: TraceEvent }) {
  const failed =
    step.status === 'error' ||
    step.status === 'timeout' ||
    (step.exit_code != null && step.exit_code !== 0)
  return (
    <div className="flex items-center gap-2 font-mono text-xs">
      <span className={failed ? 'text-destructive' : 'text-muted-foreground'}>
        {step.status ? formatStatus(step.status) : '—'}
      </span>
      <span className="truncate">{step.step ?? '—'}</span>
      {step.duration_ms != null && (
        <span className="text-muted-foreground">{step.duration_ms}ms</span>
      )}
      {step.error && (
        <span className="truncate text-destructive">{step.error}</span>
      )}
    </div>
  )
}

/**
 * The Build → Transfer → Deploy pipeline for one app.
 *
 * State lives in this component only; a page reload starts clean. Each stage
 * opens a run through its configured base (build stages on the build server,
 * deploy on the deploy server) and tails it over SSE, revealing the next stage
 * when its predecessor ends `ok`.
 */
export function Pipeline({
  app,
  onDeployed,
}: {
  app: App
  /** Called after a successful deploy, so the page can re-read the registry. */
  onDeployed?: () => void
}) {
  const [release, setRelease] = useState('')
  const [git, setGit] = useState<AppGit | null>(null)
  const [gitError, setGitError] = useState<string | null>(null)
  const [images, setImages] = useState<AppImage[]>([])
  const [imagesError, setImagesError] = useState<string | null>(null)
  const [transferTarget, setTransferTarget] = useState<string | null>(null)
  const [stages, setStages] = useState<Record<Stage, StageInfo>>({
    build: EMPTY_STAGE,
    transfer: EMPTY_STAGE,
    deploy: EMPTY_STAGE,
  })

  const streams = useRef<Record<Stage, RunStream | null>>({
    build: null,
    transfer: null,
    deploy: null,
  })

  const closeStreams = () => {
    for (const stream of Object.values(streams.current)) stream?.close()
    streams.current = { build: null, transfer: null, deploy: null }
  }

  useEffect(() => {
    const controller = new AbortController()
    setGit(null)
    setGitError(null)
    setImages([])
    setImagesError(null)
    setStages({ build: EMPTY_STAGE, transfer: EMPTY_STAGE, deploy: EMPTY_STAGE })
    setRelease('')
    closeStreams()

    api.appGit(app.name, controller.signal).then(
      (g) => {
        if (controller.signal.aborted) return
        setGit(g)
        if (g.remote_sha) setRelease((cur) => cur || g.remote_sha!)
      },
      (e) => {
        if (!controller.signal.aborted) setGitError(String(e))
      },
    )
    api.appImages(app.name, controller.signal).then(
      (r) => {
        if (!controller.signal.aborted) setImages(r.images)
      },
      (e) => {
        if (!controller.signal.aborted) setImagesError(String(e))
      },
    )
    cfg().then(
      (c) => {
        if (!controller.signal.aborted) setTransferTarget(c.transferTarget)
      },
      () => undefined,
    )

    return () => {
      controller.abort()
      closeStreams()
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [app.name])

  /** POST a stage's run, then tail it, enabling the next stage on `ok`. */
  const start = (stage: Stage, fn: () => Promise<{ run_id: string }>) => {
    setStages((cur) => ({
      ...cur,
      [stage]: { state: 'active', runId: null, steps: [], error: null },
    }))
    fn().then(
      ({ run_id }) => {
        setStages((cur) => ({
          ...cur,
          [stage]: { ...cur[stage], runId: run_id },
        }))
        const stream = openRunEvents(
          run_id,
          {
            onEvent: (ev) => {
              if (ev.event !== 'step') return
              setStages((cur) => ({
                ...cur,
                [stage]: { ...cur[stage], steps: [...cur[stage].steps, ev] },
              }))
            },
            onError: (message) => {
              setStages((cur) => ({
                ...cur,
                [stage]: { ...cur[stage], state: 'failed', error: message },
              }))
            },
            onEnd: (status) => {
              const ok = isOk(status)
              setStages((cur) => ({
                ...cur,
                [stage]: {
                  ...cur[stage],
                  state: ok ? 'ok' : 'failed',
                  error: ok
                    ? null
                    : (cur[stage].error ??
                      (status ? `ended: ${status}` : 'stream closed')),
                },
              }))
              if (ok && stage === 'deploy') onDeployed?.()
            },
          },
          stage === 'deploy' ? 'deploy' : 'build',
        )
        streams.current[stage]?.close()
        streams.current[stage] = stream
      },
      (e) => {
        setStages((cur) => ({
          ...cur,
          [stage]: { ...cur[stage], state: 'failed', error: String(e) },
        }))
      },
    )
  }

  const releaseValue = release.trim()
  const releaseInvalid =
    releaseValue === '' || !RELEASE_RE.test(releaseValue)
  const imageRef =
    app.image_repo && releaseValue ? `${app.image_repo}:${releaseValue}` : null
  const imageExists =
    imageRef != null && images.some((i) => i.ref === imageRef)
  const buildOk = stages.build.state === 'ok'
  const transferOk = stages.transfer.state === 'ok'

  const transferEnabled =
    !releaseInvalid &&
    transferTarget != null &&
    (buildOk || imageExists) &&
    stages.transfer.state !== 'active'
  const deployEnabled =
    !releaseInvalid && transferOk && stages.deploy.state !== 'active'

  const stageOrder: Stage[] = ['build', 'transfer', 'deploy']

  return (
    <Card>
      <CardHeader>
        <CardTitle>Pipeline</CardTitle>
        <p className="text-sm text-muted-foreground">
          Build on the build server, ship the image to{' '}
          <span className="font-mono">{transferTarget ?? '…'}</span>, then
          deploy. Each stage unlocks the next when it ends ok.
        </p>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex flex-wrap items-end gap-3">
          <div className="flex flex-col gap-1">
            <label className="text-xs text-muted-foreground">
              Target release
            </label>
            <Input
              value={release}
              onChange={(e) => setRelease(e.target.value)}
              placeholder="commit sha"
              className="w-64 font-mono"
              aria-invalid={releaseValue !== '' && releaseInvalid}
            />
          </div>
          {releaseValue !== '' && releaseInvalid && (
            <p className="pb-1 text-xs text-destructive">
              must be a commit SHA (hex, 4-64 chars)
            </p>
          )}
          {git ? (
            <p className="pb-1 font-mono text-xs text-muted-foreground">
              {git.branch}
              {git.remote_sha ? ` · remote ${git.remote_sha}` : ''}
              {git.head_sha ? ` · head ${git.head_sha}` : ''}
              {git.dirty ? ' · dirty' : ''}
            </p>
          ) : gitError ? (
            <p className="pb-1 text-xs text-muted-foreground">
              source state unavailable
            </p>
          ) : (
            <Skeleton className="mb-1 h-4 w-48" />
          )}
        </div>

        {stageOrder.map((stage, i) => {
          const info = stages[stage]
          const recent = info.steps.slice(-VISIBLE_STEPS)
          return (
            <div key={stage} className="space-y-2">
              <div className="flex flex-wrap items-center gap-2">
                <span className="text-sm font-medium capitalize">{stage}</span>
                <StateBadge state={info.state} />
                {info.runId && (
                  <Link
                    to="/runs/$runId"
                    params={{ runId: info.runId }}
                    className="font-mono text-xs underline-offset-4 hover:underline"
                  >
                    {info.runId.slice(0, 12)}
                  </Link>
                )}
                <span className="flex-1" />
                {stage === 'build' && (
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={releaseInvalid || info.state === 'active'}
                    onClick={() =>
                      start('build', () =>
                        api.build(app.name, releaseValue),
                      )
                    }
                  >
                    {info.state === 'active' ? 'Building…' : 'Build'}
                  </Button>
                )}
                {stage === 'transfer' && (
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={!transferEnabled}
                    title={
                      buildOk || imageExists
                        ? undefined
                        : 'build the release first, or make sure its image exists'
                    }
                    onClick={() =>
                      start('transfer', () =>
                        api.transfer(
                          app.name,
                          releaseValue,
                          transferTarget ?? '',
                        ),
                      )
                    }
                  >
                    {info.state === 'active' ? 'Transferring…' : 'Transfer'}
                  </Button>
                )}
                {stage === 'deploy' && (
                  <Button
                    size="sm"
                    disabled={!deployEnabled}
                    onClick={() =>
                      start('deploy', () =>
                        api.deploy(app.name, releaseValue),
                      )
                    }
                  >
                    {info.state === 'active' ? 'Deploying…' : 'Deploy'}
                  </Button>
                )}
              </div>

              {stage === 'transfer' && !buildOk && imageExists && (
                <p className="text-xs text-muted-foreground">
                  image {imageRef} already present on the build server
                </p>
              )}
              {stage === 'build' && imagesError && (
                <p className="text-xs text-muted-foreground">
                  image list unavailable
                </p>
              )}

              {recent.length > 0 && (
                <div className="space-y-0.5 rounded-lg border bg-muted/30 p-2">
                  {recent.map((s, j) => (
                    <StepLine key={`${s.seq ?? j}-${s.step ?? ''}`} step={s} />
                  ))}
                </div>
              )}
              {info.error && (
                <p className="text-xs text-destructive">{info.error}</p>
              )}

              {i < stageOrder.length - 1 && (
                <div className="text-center text-muted-foreground">↓</div>
              )}
            </div>
          )
        })}
      </CardContent>
    </Card>
  )
}
