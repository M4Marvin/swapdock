import { useEffect, useMemo, useRef, useState } from "react"
import { Link } from "@tanstack/react-router"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"
import {
  ArrowDown,
  ArrowRight,
  ChevronDown,
  GitBranch,
  Hammer,
  Lock,
  Play,
  Rocket,
  RotateCcw,
  Upload,
} from "lucide-react"
import {
  api,
  cfgState,
  classifyStatus,
  formatStatus,
  openRunEvents,
  type App,
  type AppGit,
  type AppImage,
  type RunStream,
  type TraceEvent,
} from "@/api"
import { queryKeys } from "@/lib/query-keys"
import { errorText, formatDuration, isRelease, plural, shortId } from "@/lib/format"
import { sameRelease } from "@/lib/releases"
import { cn } from "cn"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Skeleton } from "@/components/ui/skeleton"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Callout, MetaList, SectionHeading } from "@/components/kit"
import { StatusDot, StatusPill, runTone } from "@/components/Status"

type Stage = "build" | "transfer" | "deploy"
type StageState = "idle" | "active" | "ok" | "failed" | "dry"

interface StageInfo {
  state: StageState
  runId: string | null
  steps: TraceEvent[]
  error: string | null
}

const EMPTY_STAGE: StageInfo = { state: "idle", runId: null, steps: [], error: null }
const EMPTY_STAGES: Record<Stage, StageInfo> = {
  build: EMPTY_STAGE,
  transfer: EMPTY_STAGE,
  deploy: EMPTY_STAGE,
}

const VISIBLE_STEPS = 6

function statePill(state: StageState) {
  switch (state) {
    case "active":
      return <StatusPill tone="info" label="Running" pulse />
    case "ok":
      return <StatusPill tone="success" label="Done" />
    case "dry":
      return <StatusPill tone="info" label="Dry run" />
    case "failed":
      return <StatusPill tone="danger" label="Failed" />
    default:
      return <StatusPill tone="neutral" label="Not started" />
  }
}

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

function StepConsole({ steps, active }: { steps: TraceEvent[]; active: boolean }) {
  const recent = steps.slice(-VISIBLE_STEPS)
  return (
    <div
      aria-live="polite"
      className="mt-3 min-h-[58px] rounded-md border border-border bg-surface px-2 py-1.5"
    >
      {recent.length === 0 ? (
        <p className="py-1 font-mono text-2xs text-subtle-foreground">
          {active ? "Waiting for the first step…" : "Output appears here while it runs."}
        </p>
      ) : (
        recent.map((step, index) => (
          <div
            key={`${step.seq ?? index}-${step.step ?? ""}`}
            className="animate-fade-in flex items-center gap-1.5 py-[1px] font-mono text-2xs leading-4"
          >
            <StatusDot tone={runTone(step.status)} className="size-1" />
            <span className="min-w-0 flex-1 truncate text-muted-foreground">
              {step.step ?? "step"}
            </span>
            {step.duration_ms != null ? (
              <span className="shrink-0 text-subtle-foreground">
                {formatDuration(step.duration_ms)}
              </span>
            ) : null}
            {step.error ? (
              <span className="max-w-[45%] shrink-0 truncate text-destructive-strong">{step.error}</span>
            ) : null}
          </div>
        ))
      )}
    </div>
  )
}

function StageCard({
  index,
  title,
  description,
  subtitle,
  icon: Icon,
  info,
  action,
  hint,
  children,
}: {
  index: number
  title: string
  description: string
  subtitle?: React.ReactNode
  icon: typeof Hammer
  info: StageInfo
  action: React.ReactNode
  hint?: string
  children?: React.ReactNode
}) {
  return (
    <div
      className={cn(
        "flex h-full flex-col rounded-xl border bg-card p-3.5 transition-colors duration-200",
        info.state === "active"
          ? "border-accent/35 shadow-[0_0_0_1px_var(--accent-soft)]"
          : info.state === "failed"
            ? "border-destructive/30"
            : info.state === "ok"
              ? "border-success/25"
              : "border-border"
      )}
    >
      <div className="flex items-start gap-2.5">
        <span
          className={cn(
            "mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-md border",
            info.state === "ok"
              ? "border-success/25 bg-success/10 text-success"
              : info.state === "failed"
                ? "border-destructive/25 bg-destructive/10 text-destructive-strong"
                : info.state === "active"
                  ? "border-accent/25 bg-accent-soft text-accent"
                  : "border-border bg-surface text-subtle-foreground"
          )}
        >
          <Icon className="size-3.5" />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            <span className="font-mono text-2xs text-subtle-foreground">0{index}</span>
            <h3 className="text-sm font-medium tracking-[-0.01em] text-foreground">{title}</h3>
          </div>
          <p className="mt-0.5 text-2xs leading-relaxed text-muted-foreground">{description}</p>
        </div>
        <div className="shrink-0">{statePill(info.state)}</div>
      </div>

      <div className="mt-2.5 min-w-0">
        {subtitle ? <div className="truncate font-mono text-2xs text-subtle-foreground">{subtitle}</div> : null}
      </div>

      {info.runId ? (
        <Link
          to="/runs/$runId"
          params={{ runId: info.runId }}
          className="mt-2 inline-flex w-fit items-center gap-1 font-mono text-2xs text-muted-foreground transition-colors hover:text-foreground"
        >
          run {shortId(info.runId)}
          <ArrowRight className="size-2.5" />
        </Link>
      ) : null}

      {children ?? <StepConsole steps={info.steps} active={info.state === "active"} />}

      {info.error ? (
        <p className="mt-2 text-2xs leading-relaxed text-destructive-strong">{info.error}</p>
      ) : null}

      <div className="mt-auto pt-3">
        {action}
        {hint ? <p className="mt-1.5 text-2xs leading-relaxed text-subtle-foreground">{hint}</p> : null}
      </div>
    </div>
  )
}

/**
 * The release pipeline: pin a release, build its image, (optionally) ship it,
 * then deploy it through the health gate. Each stage streams its run over SSE;
 * a quiet stream falls back to polling so a long Docker build is not mistaken
 * for a failure.
 */
export function Pipeline({
  app,
  initialRelease,
  onDeployed,
}: {
  app: App
  /** A release preselected by the version card. */
  initialRelease?: string
  onDeployed?: () => void
}) {
  const queryClient = useQueryClient()
  const [release, setRelease] = useState(initialRelease ?? "")
  const [stages, setStages] = useState<Record<Stage, StageInfo>>(EMPTY_STAGES)
  const [pinnedRelease, setPinnedRelease] = useState<string | null>(null)
  const [confirmDeploy, setConfirmDeploy] = useState(false)
  const [editingRelease, setEditingRelease] = useState(false)

  const gitQuery = useQuery({
    queryKey: queryKeys.git(app.name),
    queryFn: ({ signal }) => api.appGit(app.name, signal),
    retry: 0,
  })
  const imagesQuery = useQuery({
    queryKey: queryKeys.images(app.name),
    queryFn: ({ signal }) => api.appImages(app.name, signal),
    retry: 0,
  })
  const configQuery = useQuery({ queryKey: queryKeys.config, queryFn: () => cfgState() })
  const latestQuery = useQuery({
    queryKey: queryKeys.latest(app.name),
    queryFn: ({ signal }) => api.appLatest(app.name, signal),
    enabled: app.repo != null || app.build_repo != null,
    retry: 0,
  })

  const git: AppGit | null = gitQuery.data ?? null
  const images: AppImage[] = imagesQuery.data?.images ?? []
  const transferTarget = configQuery.data?.config.transferTarget?.trim() || null

  const sameHost = useMemo(() => {
    const build = configQuery.data?.config.buildApi
    const deploy = configQuery.data?.config.deployApi
    if (!build || !deploy) return false
    try {
      return new URL(build, window.location.origin).origin === new URL(deploy, window.location.origin).origin
    } catch {
      return build === deploy
    }
  }, [configQuery.data])

  const streams = useRef<Record<Stage, RunStream | null>>({
    build: null,
    transfer: null,
    deploy: null,
  })
  const alive = useRef(true)
  const releaseTouched = useRef(initialRelease != null)

  const closeStreams = () => {
    for (const stream of Object.values(streams.current)) stream?.close()
    streams.current = { build: null, transfer: null, deploy: null }
  }

  const resetFlow = () => {
    setStages(EMPTY_STAGES)
    setPinnedRelease(null)
    setEditingRelease(false)
    closeStreams()
  }

  useEffect(() => {
    setStages(EMPTY_STAGES)
    setRelease(initialRelease ?? "")
    setPinnedRelease(null)
    setEditingRelease(false)
    releaseTouched.current = initialRelease != null
    closeStreams()
  }, [app.name, initialRelease])

  const upstreamRelease = gitQuery.data?.remote_sha ?? latestQuery.data?.release ?? null

  useEffect(() => {
    if (upstreamRelease && !releaseTouched.current) setRelease(upstreamRelease)
  }, [upstreamRelease])

  useEffect(() => {
    alive.current = true
    return () => {
      alive.current = false
      closeStreams()
    }
  }, [])

  const buildMutation = useMutation({
    mutationFn: (vars: { release: string }) => api.build(app.name, vars.release),
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: queryKeys.runs }),
  })
  const transferMutation = useMutation({
    mutationFn: (vars: { release: string; target: string }) =>
      api.transfer(app.name, vars.release, vars.target),
    onSuccess: () => void queryClient.invalidateQueries({ queryKey: queryKeys.runs }),
  })

  const releaseValue = release.trim()
  const releaseInvalid = releaseValue === "" || !isRelease(releaseValue)
  const releaseIsLatest = sameRelease(releaseValue, upstreamRelease)
  const releaseIsLive = sameRelease(releaseValue, app.release)
  const releaseLabel =
    releaseValue === ""
      ? "No version resolved"
      : releaseIsLatest
        ? `Latest from ${git?.branch ?? app.branch ?? "main"}`
        : releaseIsLive
          ? "Current live version"
          : "Custom version"
  const releaseForFlow = pinnedRelease ?? releaseValue
  const imageRef =
    app.image_repo && releaseForFlow ? `${app.image_repo}:${releaseForFlow}` : null
  const imageExists = imageRef != null && images.some((image) => image.ref === imageRef)

  const anyStageActive = (Object.keys(stages) as Stage[]).some((stage) => stages[stage].state === "active")
  const flowActive = pinnedRelease !== null
  const releaseLocked = flowActive || anyStageActive
  const deployOk = stages.deploy.state === "ok"
  const buildOk = stages.build.state === "ok"
  const transferOk = stages.transfer.state === "ok"

  const start = (
    stage: Stage,
    releaseForStage: string,
    action: () => Promise<{ run_id: string }>
  ) => {
    setEditingRelease(false)
    if (pinnedRelease === null) setPinnedRelease(releaseForStage)
    const toastId = `${app.name}:${stage}`
    setStages((current) => ({
      ...current,
      [stage]: { state: "active", runId: null, steps: [], error: null },
    }))

    const settle = (status: string | null) => {
      if (!alive.current) return
      const outcome = classifyStatus(status)
      const ok = outcome === "ok"
      const dry = outcome === "dry"
      if (ok) {
        if (stage === "build") {
          toast.success(`Build finished${imageRef ? `: ${imageRef}` : ""}`, { id: toastId })
          void queryClient.invalidateQueries({ queryKey: queryKeys.images(app.name) })
        } else if (stage === "transfer") {
          toast.success("Image transferred", { id: toastId })
        } else {
          toast.success("Deploy finished", { id: toastId })
          onDeployed?.()
          void queryClient.invalidateQueries({ queryKey: queryKeys.app(app.name) })
          void queryClient.invalidateQueries({ queryKey: queryKeys.apps })
          void queryClient.invalidateQueries({ queryKey: queryKeys.runs })
          void queryClient.invalidateQueries({ queryKey: queryKeys.appStatus(app.name) })
          void queryClient.invalidateQueries({ queryKey: queryKeys.latest(app.name) })
        }
      } else if (dry) {
        toast.message(`${stage} finished as a dry run`, {
          id: toastId,
          description: "Nothing on the host was changed.",
        })
      } else {
        toast.error(`${stage} failed`, {
          id: toastId,
          description: status ? `ended: ${formatStatus(status)}` : "the stream closed",
        })
      }
      setStages((current) => ({
        ...current,
        [stage]: {
          ...current[stage],
          state: ok ? "ok" : dry ? "dry" : "failed",
          error: ok || dry ? null : (current[stage].error ?? (status ? `Ended ${formatStatus(status)}` : "Stream closed")),
        },
      }))
    }

    action().then(
      ({ run_id }) => {
        if (!alive.current) return
        setStages((current) => ({
          ...current,
          [stage]: { ...current[stage], runId: run_id },
        }))
        void queryClient.invalidateQueries({ queryKey: queryKeys.runs })

        const stream = openRunEvents(
          run_id,
          {
            onEvent: (event) => {
              if (event.event !== "step") return
              setStages((current) => ({
                ...current,
                [stage]: {
                  ...current[stage],
                  steps: [...current[stage].steps, event].slice(-50),
                },
              }))
            },
            onError: (message) => {
              if (!alive.current) return
              setStages((current) => ({ ...current, [stage]: { ...current[stage], error: message } }))
            },
            onEnd: (status) => {
              if (status != null) {
                settle(status)
                return
              }
              if (!alive.current) return
              toast.message(`${stage} stream closed; checking the run…`, { id: toastId })
              const started = Date.now()
              let failures = 0
              const poll = async () => {
                while (alive.current) {
                  try {
                    const info = await api.resume(run_id, stage === "deploy" ? "deploy" : "build")
                    failures = 0
                    if (info.ended) {
                      settle(info.ended)
                      return
                    }
                  } catch {
                    failures += 1
                    if (failures >= 3) {
                      settle(null)
                      return
                    }
                  }
                  if (Date.now() - started > 45 * 60 * 1000) {
                    settle(null)
                    return
                  }
                  await new Promise((resolve) => setTimeout(resolve, 3000))
                }
              }
              void poll()
            },
          },
          stage === "deploy" ? "deploy" : "build"
        )
        if (!alive.current) {
          stream.close()
          return
        }
        streams.current[stage]?.close()
        streams.current[stage] = stream
      },
      (error) => {
        if (!alive.current) return
        const message = errorText(error)
        toast.error(`${stage} failed`, { id: toastId, description: message })
        setStages((current) => ({
          ...current,
          [stage]: { ...current[stage], state: "failed", error: message },
        }))
      }
    )
  }

  const flowState = anyStageActive
    ? "running"
    : deployOk
      ? "done"
      : stages.build.state === "failed" || stages.transfer.state === "failed" || stages.deploy.state === "failed"
        ? "failed"
        : flowActive
          ? "pinned"
          : "idle"

  const buildDisabled =
    releaseInvalid || stages.build.state === "active" || (flowActive && stages.build.state === "ok")
  const transferEnabled =
    !sameHost &&
    !releaseInvalid &&
    transferTarget != null &&
    (buildOk || imageExists) &&
    stages.transfer.state !== "active" &&
    !deployOk
  const deployEnabled =
    !releaseInvalid &&
    (sameHost ? buildOk || imageExists : transferOk) &&
    stages.deploy.state !== "active" &&
    !deployOk

  const transferHint = releaseInvalid
    ? "Enter a valid commit SHA first."
    : transferTarget == null
      ? "Configure a transfer target in config.json before sending an image between hosts."
      : !(buildOk || imageExists)
        ? "Build the release first, or make sure its image already exists."
        : undefined
  const deployHint = releaseInvalid
    ? "Enter a valid commit SHA first."
    : !sameHost && !transferOk
      ? "Send the image to the deploy host first."
      : !sameHost
        ? undefined
        : !(buildOk || imageExists)
          ? "Build the release first, or make sure its image already exists."
          : undefined

  const stageOrder: Stage[] = sameHost ? ["build", "deploy"] : ["build", "transfer", "deploy"]

  return (
    <section className="space-y-4">
      <div className="overflow-hidden rounded-xl border border-border bg-card shadow-[var(--shadow-card)]">
        <div className="flex flex-wrap items-start justify-between gap-3 border-b border-border/60 px-4 py-3.5">
          <SectionHeading
            title="Release pipeline"
            description={
              sameHost
                ? "Build and deploy share this host. No image transfer is needed."
                : `Build, ship to ${transferTarget ?? "the transfer target"}, then deploy through the health gate.`
            }
          />
          <div className="flex items-center gap-2">
            {flowState === "running" ? (
              <StatusPill tone="info" label="In progress" pulse />
            ) : flowState === "done" ? (
              <StatusPill tone="success" label="Release deployed" />
            ) : flowState === "failed" ? (
              <StatusPill tone="danger" label="Needs attention" />
            ) : (
              <StatusPill tone="neutral" label={flowActive ? "Pinned" : "Ready"} />
            )}
            {flowActive && !anyStageActive ? (
              <Button variant="ghost" size="xs" onClick={resetFlow}>
                <RotateCcw />
                Start over
              </Button>
            ) : null}
          </div>
        </div>

        <div className="space-y-4 p-4">
          <div className="rounded-lg border border-border bg-surface/60 p-3">
            <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-3">
              <div className="min-w-[260px] flex-1">
                <div className="flex items-center gap-2">
                  <span className="text-2xs font-medium text-muted-foreground">Version to ship</span>
                  {releaseLocked ? (
                    <span className="inline-flex items-center gap-1 text-2xs text-subtle-foreground">
                      <Lock className="size-3" />
                      pinned
                    </span>
                  ) : null}
                </div>

                {editingRelease && !releaseLocked ? (
                  <div className="mt-1 flex flex-wrap items-center gap-2">
                    <Input
                      id="pipeline-release"
                      value={release}
                      onChange={(event) => {
                        releaseTouched.current = true
                        setRelease(event.target.value)
                      }}
                      placeholder="commit reference"
                      className="h-8 w-full max-w-[280px] font-mono"
                      aria-invalid={releaseInvalid}
                      spellCheck={false}
                      autoComplete="off"
                      autoFocus
                    />
                    {upstreamRelease && !sameRelease(releaseValue, upstreamRelease) ? (
                      <Button
                        variant="ghost"
                        size="xs"
                        className="shrink-0"
                        onClick={() => {
                          releaseTouched.current = true
                          setRelease(upstreamRelease ?? "")
                        }}
                      >
                        Use latest
                      </Button>
                    ) : null}
                    <Button
                      variant="ghost"
                      size="xs"
                      className="shrink-0"
                      onClick={() => setEditingRelease(false)}
                    >
                      Done
                    </Button>
                  </div>
                ) : (
                  <button
                    type="button"
                    disabled={releaseLocked}
                    onClick={() => setEditingRelease(true)}
                    className={cn(
                      "mt-1 flex h-8 w-full max-w-[320px] items-center gap-2 rounded-lg border border-border bg-surface px-2.5 text-left text-sm transition-colors duration-150",
                      releaseLocked ? "cursor-default" : "hover:border-border-strong hover:bg-muted/40"
                    )}
                  >
                    <GitBranch className="size-3.5 shrink-0 text-subtle-foreground" />
                    <span
                      className={cn(
                        "min-w-0 flex-1 truncate",
                        releaseInvalid ? "text-destructive-strong" : "text-foreground"
                      )}
                    >
                      {releaseLabel}
                    </span>
                    {releaseValue && !releaseIsLatest && !releaseIsLive ? (
                      <span className="font-mono text-2xs text-subtle-foreground">
                        {releaseValue.slice(0, 7)}
                      </span>
                    ) : null}
                    {!releaseLocked ? (
                      <>
                        <span className="shrink-0 text-2xs text-subtle-foreground">Change</span>
                        <ChevronDown className="size-3 shrink-0 text-subtle-foreground" />
                      </>
                    ) : null}
                  </button>
                )}

                {editingRelease && !releaseLocked ? (
                  <p className="mt-1 text-2xs text-subtle-foreground">
                    Advanced: edit the commit reference. Normal deploys use the version resolved
                    above.
                  </p>
                ) : releaseValue === "" ? (
                  <p className="mt-1 text-2xs text-destructive-strong">
                    No version resolved. Open the version control to set one.
                  </p>
                ) : releaseIsLatest ? (
                  <p className="mt-1 text-2xs text-subtle-foreground">
                    Resolved automatically from {git?.branch ?? app.branch ?? "the source"}. You
                    never need to type a commit hash.
                  </p>
                ) : releaseIsLive ? (
                  <p className="mt-1 text-2xs text-subtle-foreground">
                    This is the version currently serving.
                  </p>
                ) : (
                  <p className="mt-1 text-2xs text-subtle-foreground">
                    Custom version. Normal deploys resolve the newest upstream version
                    automatically.
                  </p>
                )}

                {releaseValue !== "" && releaseInvalid ? (
                  <p className="mt-1 text-2xs text-destructive-strong">
                    A commit reference is 7–64 hexadecimal characters.
                  </p>
                ) : null}

                <div className="mt-1.5 flex flex-wrap items-center gap-2 text-2xs text-subtle-foreground">
                  {gitQuery.isPending && !latestQuery.data ? (
                    <Skeleton className="h-2.5 w-40" />
                  ) : git ? (
                    <>
                      <span className="inline-flex items-center gap-1">
                        <GitBranch className="size-2.5" />
                        <span className="font-mono">{git.branch}</span>
                      </span>
                      <span>·</span>
                      <span>{git.dirty ? "dirty checkout" : "clean checkout"}</span>
                      {releaseIsLatest ? <Badge variant="success">upstream</Badge> : null}
                      {releaseIsLive && !releaseIsLatest ? <Badge variant="outline">live</Badge> : null}
                    </>
                  ) : latestQuery.data?.release ? (
                    <span>Resolved from the latest fetched source.</span>
                  ) : (
                    <span>Source state unavailable. Open the version control to set a commit.</span>
                  )}
                </div>
              </div>

              <div className="flex flex-wrap items-center gap-2 pt-5">
                {imageRef ? (
                  <Badge variant={imageExists ? "success" : "outline"}>
                    {imageExists ? "image ready" : "image not built"}
                  </Badge>
                ) : null}
                {app.kind === "static" ? (
                  <Badge variant="outline">static root flip</Badge>
                ) : null}
              </div>
            </div>
          </div>

          {flowState === "done" ? (
            <Callout
              tone="success"
              title="Release deployed"
              action={
                stages.deploy.runId ? (
                  <Button
                    variant="outline"
                    size="xs"
                    render={<Link to="/runs/$runId" params={{ runId: stages.deploy.runId }} />}
                  >
                    View run
                    <ArrowRight />
                  </Button>
                ) : undefined
              }
            >
              {releaseForFlow} is live behind the front door. The previous slot is kept for
              rollback.
            </Callout>
          ) : null}

          <div
            className={cn(
              "grid gap-2",
              sameHost
                ? "md:grid-cols-[minmax(0,1fr)_28px_minmax(0,1fr)]"
                : "md:grid-cols-[minmax(0,1fr)_28px_minmax(0,1fr)_28px_minmax(0,1fr)]"
            )}
          >
            <StageCard
              index={1}
              title="Build"
              description="Produce the release image on the build host."
              subtitle={imageRef ?? "No image repo configured"}
              icon={Hammer}
              info={stages.build}
              hint={
                releaseInvalid
                  ? "Enter a commit SHA to build."
                  : imageExists
                    ? "The image already exists; building refreshes the tag."
                    : undefined
              }
              action={
                <Button
                  variant="outline"
                  size="sm"
                  className="w-full"
                  disabled={buildDisabled}
                  onClick={() =>
                    start("build", releaseForFlow, () =>
                      buildMutation.mutateAsync({ release: releaseForFlow })
                    )
                  }
                >
                  <Play />
                  {stages.build.state === "active"
                    ? "Building…"
                    : stages.build.state === "ok"
                      ? "Built"
                      : "Build image"}
                </Button>
              }
            />
            <Connector />
            {sameHost ? null : (
              <>
                <StageCard
                  index={2}
                  title="Transfer"
                  description={`Send the image to ${transferTarget ?? "the deploy host"}.`}
                  subtitle={transferTarget ? `target ${transferTarget}` : "no target configured"}
                  icon={Upload}
                  info={stages.transfer}
                  hint={transferHint}
                  action={
                    <Button
                      variant="outline"
                      size="sm"
                      className="w-full"
                      disabled={!transferEnabled}
                      title={transferEnabled ? undefined : transferHint}
                      onClick={() =>
                        start("transfer", releaseForFlow, () =>
                          transferMutation.mutateAsync({
                            release: releaseForFlow,
                            target: transferTarget ?? "",
                          })
                        )
                      }
                    >
                      <Upload />
                      {stages.transfer.state === "active" ? "Sending…" : "Send image"}
                    </Button>
                  }
                />
                <Connector />
              </>
            )}
            <StageCard
              index={stageOrder.length}
              title="Deploy"
              description={`Flip nginx and run the ${app.strategy} health gate.`}
              subtitle={`${app.strategy} · health ${app.health_url ?? "not configured"}`}
              icon={Rocket}
              info={stages.deploy}
              hint={deployHint}
              action={
                <Button
                  size="sm"
                  className="w-full"
                  disabled={!deployEnabled}
                  title={deployEnabled ? undefined : deployHint}
                  onClick={() => setConfirmDeploy(true)}
                >
                  <Rocket />
                  {stages.deploy.state === "active" ? "Deploying…" : "Deploy release"}
                </Button>
              }
            />
          </div>

          <div className="flex flex-wrap items-center justify-between gap-2 border-t border-border/60 pt-3 text-2xs text-subtle-foreground">
            <span>
              The release stays pinned for the whole flow. The server refuses a deploy whose
              image was never built.
            </span>
            <span>{plural(stageOrder.length, "stage")} · {sameHost ? "single host" : "split host"}</span>
          </div>
        </div>
      </div>

      <Dialog open={confirmDeploy} onOpenChange={setConfirmDeploy}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Deploy to {app.name}?</DialogTitle>
            <DialogDescription>
              This ships to production through nginx. {app.strategy === "replace"
                ? "The container is replaced in place, so the app restarts on the same port."
                : "Traffic flips to the idle slot after the candidate answers its health check."}
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-3">
            <div className="rounded-xl border border-border bg-muted/25 p-3">
              <MetaList
                columns={2}
                items={[
                  { label: "Version to deploy", value: releaseForFlow, mono: true },
                  { label: "Current version", value: app.release ?? "not deployed", mono: true },
                  { label: "Strategy", value: app.strategy },
                  { label: "Front port", value: `:${app.front_port}`, mono: true },
                ]}
              />
            </div>
            {releaseForFlow === app.release ? (
              <Callout tone="warning" title="This release is already live">
                The deploy runs with <span className="font-mono">force</span> and re-runs the
                health gate.
              </Callout>
            ) : (
              <Callout tone="info" title="Rollback stays available">
                The previous release is retained until the next deploy. You can roll back from
                the app menu.
              </Callout>
            )}
          </div>

          <DialogFooter>
            <Button variant="outline" onClick={() => setConfirmDeploy(false)}>
              Cancel
            </Button>
            <Button
              onClick={() => {
                setConfirmDeploy(false)
                start("deploy", releaseForFlow, () =>
                  api.deploy(app.name, releaseForFlow, releaseForFlow === app.release)
                )
              }}
            >
              <Rocket />
              Deploy now
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  )
}
