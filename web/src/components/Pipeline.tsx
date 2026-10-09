import { Link } from '@tanstack/react-router'
import { useEffect, useRef, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import {
  api,
  cfg,
  classifyStatus,
  formatStatus,
  openRunEvents,
  type App,
  type AppGit,
  type AppImage,
  type RunStream,
  type TraceEvent,
} from '../api'
import { queryKeys } from '@/lib/query-keys'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Skeleton } from '@/components/ui/skeleton'
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'

/** The three stages a release moves through, in order. */
type Stage = 'build' | 'transfer' | 'deploy'

type StageState = 'idle' | 'active' | 'ok' | 'failed' | 'dry'

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

const EMPTY_STAGES: Record<Stage, StageInfo> = {
  build: EMPTY_STAGE,
  transfer: EMPTY_STAGE,
  deploy: EMPTY_STAGE,
}

/** A release is a commit SHA: hex, 4-64 chars. */
const RELEASE_RE = /^[0-9a-fA-F]{4,64}$/

/** How many recent step lines to keep visible per stage. */
const VISIBLE_STEPS = 8

/** Operator-facing text for a thrown value. */
function errorText(e: unknown): string {
  if (e instanceof Error) return e.message
  if (typeof e === 'string') return e
  return 'unexpected error'
}

function StateBadge({ state }: { state: StageState }) {
  switch (state) {
    case 'active':
      return (
        <Badge variant="secondary" className="gap-1">
          <span className="size-1.5 animate-pulse rounded-full bg-warning" />
          running
        </Badge>
      )
    case 'ok':
      return <Badge variant="success">ok</Badge>
    case 'dry':
      return <Badge variant="secondary">dry run</Badge>
    case 'failed':
      return <Badge variant="destructive">failed</Badge>
    default:
      return (
        <Badge variant="outline" className="text-muted-foreground">
          not started
        </Badge>
      )
  }
}

/** Compact one-line status for a streamed step. */
function StepLine({ step }: { step: TraceEvent }) {
  const failed =
    classifyStatus(step.status) === 'failed' ||
    (step.exit_code != null && step.exit_code !== 0)
  return (
    <div className="flex items-center gap-2 font-mono text-xs animate-in fade-in slide-in-from-bottom-1 duration-150">
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
 *
 * The release is pinned when the flow starts, so a later edit to the field can
 * never ship a different commit SHA than the one that was built.
 */
export function Pipeline({
  app,
  onDeployed,
}: {
  app: App
  /** Called after a successful deploy, so the page can re-read the registry. */
  onDeployed?: () => void
}) {
  const queryClient = useQueryClient()
  const [release, setRelease] = useState('')
  const [stages, setStages] = useState<Record<Stage, StageInfo>>(EMPTY_STAGES)
  const [pinnedRelease, setPinnedRelease] = useState<string | null>(null)
  const [confirmDeploy, setConfirmDeploy] = useState(false)

  const gitQuery = useQuery({
    queryKey: queryKeys.git(app.name),
    queryFn: ({ signal }) => api.appGit(app.name, signal),
  })
  const imagesQuery = useQuery({
    queryKey: queryKeys.images(app.name),
    queryFn: ({ signal }) => api.appImages(app.name, signal),
  })
  const configQuery = useQuery({
    queryKey: queryKeys.config,
    queryFn: () => cfg(),
  })

  const git: AppGit | null = gitQuery.data ?? null
  const images: AppImage[] = imagesQuery.data?.images ?? []
  const transferTarget = configQuery.data?.transferTarget ?? null

  const streams = useRef<Record<Stage, RunStream | null>>({
    build: null,
    transfer: null,
    deploy: null,
  })
  // Flips false on unmount so an in-flight POST cannot open a stream or fire
  // toasts from a component that is already gone.
  const alive = useRef(true)
  // Set once the operator edits the release field, so a background git refetch
  // never clobbers a value they cleared or changed.
  const releaseTouched = useRef(false)

  const closeStreams = () => {
    for (const stream of Object.values(streams.current)) stream?.close()
    streams.current = { build: null, transfer: null, deploy: null }
  }

  const resetFlow = () => {
    setStages(EMPTY_STAGES)
    setPinnedRelease(null)
    closeStreams()
  }

  useEffect(() => {
    setStages(EMPTY_STAGES)
    setRelease('')
    setPinnedRelease(null)
    releaseTouched.current = false
    closeStreams()
  }, [app.name])

  // Seed the release field from the remote ref once the checkout state loads,
  // without clobbering a value the user has already typed.
  useEffect(() => {
    const remote = gitQuery.data?.remote_sha
    if (remote && !releaseTouched.current) setRelease(remote)
  }, [gitQuery.data])

  useEffect(() => {
    alive.current = true
    return () => {
      alive.current = false
      closeStreams()
    }
  }, [])

  const buildMutation = useMutation({
    mutationFn: (vars: { release: string }) =>
      api.build(app.name, vars.release),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.runs })
    },
  })
  const transferMutation = useMutation({
    mutationFn: (vars: { release: string; target: string }) =>
      api.transfer(app.name, vars.release, vars.target),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.runs })
    },
  })

  const releaseValue = release.trim()
  const releaseInvalid = releaseValue === '' || !RELEASE_RE.test(releaseValue)
  // The release the flow is bound to once it starts.
  const releaseForFlow = pinnedRelease ?? releaseValue
  const imageRef =
    app.image_repo && releaseForFlow
      ? `${app.image_repo}:${releaseForFlow}`
      : null

  const anyStageActive = (Object.keys(stages) as Stage[]).some(
    (s) => stages[s].state === 'active',
  )
  const flowActive = pinnedRelease !== null
  const releaseLocked = flowActive || anyStageActive

  /** POST a stage's run, then tail it, enabling the next stage on `ok`. */
  const start = (
    stage: Stage,
    releaseForStage: string,
    fn: () => Promise<{ run_id: string }>,
  ) => {
    if (pinnedRelease === null) setPinnedRelease(releaseForStage)
    const toastId = `${app.name}:${stage}`
    setStages((cur) => ({
      ...cur,
      [stage]: { state: 'active', runId: null, steps: [], error: null },
    }))
    fn().then(
      ({ run_id }) => {
        if (!alive.current) return
        setStages((cur) => ({
          ...cur,
          [stage]: { ...cur[stage], runId: run_id },
        }))
        queryClient.invalidateQueries({ queryKey: queryKeys.runs })
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
              toast.error(`${stage} failed`, { id: toastId, description: message })
            },
            onEnd: (status) => {
              const outcome = classifyStatus(status)
              const ok = outcome === 'ok'
              const dry = outcome === 'dry'
              if (ok) {
                if (stage === 'build' && imageRef) {
                  toast.success(`build ok: ${imageRef}`, { id: toastId })
                  queryClient.invalidateQueries({
                    queryKey: queryKeys.images(app.name),
                  })
                }
                if (stage === 'transfer')
                  toast.success('transfer ok', { id: toastId })
                if (stage === 'deploy') {
                  toast.success('deploy ok', { id: toastId })
                  onDeployed?.()
                }
              } else if (dry) {
                toast.message(`${stage} finished as a dry run`, {
                  id: toastId,
                  description: 'Nothing was changed.',
                })
              } else {
                const detail = status
                  ? `ended: ${formatStatus(status)}`
                  : 'stream closed'
                toast.error(`${stage} failed`, { id: toastId, description: detail })
              }
              setStages((cur) => ({
                ...cur,
                [stage]: {
                  ...cur[stage],
                  state: ok ? 'ok' : dry ? 'dry' : 'failed',
                  error:
                    ok || dry
                      ? null
                      : (cur[stage].error ??
                        (status
                          ? `ended: ${formatStatus(status)}`
                          : 'stream closed')),
                },
              }))
            },
          },
          stage === 'deploy' ? 'deploy' : 'build',
        )
        // The component may have unmounted while the POST was in flight.
        if (!alive.current) {
          stream.close()
          return
        }
        streams.current[stage]?.close()
        streams.current[stage] = stream
      },
      (e) => {
        if (!alive.current) return
        const message = errorText(e)
        toast.error(`${stage} failed`, { id: toastId, description: message })
        setStages((cur) => ({
          ...cur,
          [stage]: { ...cur[stage], state: 'failed', error: message },
        }))
      },
    )
  }

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

  // Why a stage button is disabled, surfaced through `title`.
  const releaseHint = releaseInvalid ? 'enter a valid commit SHA first' : undefined
  const transferHint = releaseInvalid
    ? releaseHint
    : transferTarget == null
      ? 'no transfer target configured'
      : !(buildOk || imageExists)
        ? 'build the release first, or make sure its image exists'
        : undefined
  const deployHint = releaseInvalid
    ? releaseHint
    : !transferOk
      ? 'transfer this release first'
      : undefined

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
            <Label htmlFor="pipeline-release" className="text-muted-foreground">
              Target release
            </Label>
            <Input
              id="pipeline-release"
              value={release}
              onChange={(e) => {
                releaseTouched.current = true
                setRelease(e.target.value)
              }}
              disabled={releaseLocked}
              title={
                releaseLocked
                  ? 'locked while a pipeline flow is in progress'
                  : undefined
              }
              placeholder="commit sha"
              className="w-64 font-mono"
              aria-invalid={releaseValue !== '' && releaseInvalid}
              aria-describedby={
                releaseValue !== '' && releaseInvalid
                  ? 'pipeline-release-error'
                  : undefined
              }
            />
          </div>
          {releaseValue !== '' && releaseInvalid && (
            <p id="pipeline-release-error" className="text-xs text-destructive">
              must be a commit SHA (hex, 4-64 chars)
            </p>
          )}
          {flowActive && !anyStageActive && (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              onClick={resetFlow}
            >
              Start over
            </Button>
          )}
          {git ? (
            <p className="font-mono text-xs text-muted-foreground">
              {git.branch}
              {git.remote_sha ? ` · remote ${git.remote_sha}` : ''}
              {git.head_sha ? ` · head ${git.head_sha}` : ''}
              {git.dirty ? ' · dirty' : ''}
            </p>
          ) : gitQuery.isError ? (
            <p className="text-xs text-muted-foreground">
              source state unavailable
            </p>
          ) : (
            <Skeleton className="h-4 w-48" />
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
                    title={info.state === 'active' ? undefined : releaseHint}
                    onClick={() =>
                      start('build', releaseForFlow, () =>
                        buildMutation.mutateAsync({ release: releaseForFlow }),
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
                    title={transferEnabled ? undefined : transferHint}
                    onClick={() =>
                      start('transfer', releaseForFlow, () =>
                        transferMutation.mutateAsync({
                          release: releaseForFlow,
                          target: transferTarget ?? '',
                        }),
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
                    title={deployEnabled ? undefined : deployHint}
                    onClick={() => setConfirmDeploy(true)}
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
              {stage === 'build' && imagesQuery.isError && (
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
                <div
                  aria-hidden="true"
                  className="pl-1 text-xs text-muted-foreground"
                >
                  ↓
                </div>
              )}
            </div>
          )
        })}
      </CardContent>

      <Dialog open={confirmDeploy} onOpenChange={setConfirmDeploy}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Deploy {app.name}?</DialogTitle>
            <DialogDescription>
              This runs the deploy for release{' '}
              <span className="font-mono">{releaseForFlow}</span>. It ships to
              production and cannot be undone from here.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => setConfirmDeploy(false)}
            >
              Cancel
            </Button>
            <Button
              type="button"
              onClick={() => {
                setConfirmDeploy(false)
                start('deploy', releaseForFlow, () =>
                  api.deploy(app.name, releaseForFlow),
                )
              }}
            >
              Deploy
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </Card>
  )
}
