import { useEffect, useMemo, useRef, useState } from "react"
import { Link } from "@tanstack/react-router"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"
import {
  ArrowRight,
  Box,
  Check,
  ChevronDown,
  GitBranch,
  Hammer,
  Loader2,
  Rocket,
  Sparkles,
  Terminal,
  Upload,
  X,
} from "lucide-react"
import { api, cfgState, classifyStatus, type App } from "@/api"
import { queryKeys } from "@/lib/query-keys"
import {
  errorText,
  formatDuration,
  formatRelative,
  formatShortDateTime,
  isRelease,
  shortId,
} from "@/lib/format"
import {
  dockerTime,
  imageForRelease,
  newestReleaseImage,
  releaseImages,
  sameRelease,
} from "@/lib/releases"
import { followRun, type RunPhase } from "@/lib/run-tail"
import { cn } from "cn"
import { deploymentKind } from "@/lib/deployment"
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
import { Callout, CopyButton } from "@/components/kit"
import { StatusDot, StatusPill, runTone } from "@/components/Status"

type Choice = "latest" | "image" | "live" | "specific"
type Phase = "choose" | "running" | "done" | "failed"
type StageId = "build" | "transfer" | "deploy"
type StageState = "pending" | "active" | "ok" | "failed" | "skipped"

interface StageInfo {
  state: StageState
  runId: string | null
  current: string | null
  error: string | null
}

interface LogLine {
  stage: StageId
  step: string
  status: string | null | undefined
  durationMs: number | null | undefined
  error: string | null | undefined
}

const STAGE_LABELS: Record<StageId, string> = {
  build: "Build image",
  transfer: "Send image",
  deploy: "Deploy & health gate",
}

/** Step names a build step is recorded under. */
const BUILD_STEPS = new Set([
  "build-clone",
  "build-checkout",
  "docker-build",
  "docker-push",
  "ssh-build",
])

/** Which plan stage a step belongs to, so one run can drive all three rows. */
function stageOf(step: string | undefined): StageId {
  if (step && BUILD_STEPS.has(step)) return "build"
  if (step?.startsWith("transfer")) return "transfer"
  return "deploy"
}

function emptyStage(): StageInfo {
  return { state: "pending", runId: null, current: null, error: null }
}

function emptyStages(): Record<StageId, StageInfo> {
  return { build: emptyStage(), transfer: emptyStage(), deploy: emptyStage() }
}

function stagePill(state: StageState) {
  switch (state) {
    case "active":
      return <StatusPill tone="info" label="Running" pulse />
    case "ok":
      return <StatusPill tone="success" label="Done" />
    case "failed":
      return <StatusPill tone="danger" label="Failed" />
    case "skipped":
      return <StatusPill tone="neutral" label="Not needed" />
    default:
      return <StatusPill tone="neutral" label="Waiting" />
  }
}

function PlanStage({
  id,
  info,
  label,
  description,
}: {
  id: StageId
  info: StageInfo
  label: string
  description: string
}) {
  const Icon = id === "build" ? Hammer : id === "transfer" ? Upload : Rocket
  return (
    <div className="flex items-start gap-3 border-b border-border/60 px-4 py-2.5 last:border-0">
      <span
        className={cn(
          "mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-md border",
          info.state === "active"
            ? "border-accent/25 bg-accent-soft text-accent"
            : info.state === "ok"
              ? "border-success/25 bg-success/10 text-success"
              : info.state === "failed"
                ? "border-destructive/25 bg-destructive/10 text-destructive-strong"
                : "border-border bg-surface text-subtle-foreground"
        )}
      >
        {info.state === "active" ? (
          <Loader2 className="size-3.5 animate-spin" />
        ) : info.state === "ok" ? (
          <Check className="size-3.5" />
        ) : info.state === "failed" ? (
          <X className="size-3.5" />
        ) : (
          <Icon className="size-3.5" />
        )}
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm font-medium text-foreground">{label}</span>
          {stagePill(info.state)}
          {info.runId ? (
            <Link
              to="/runs/$runId"
              params={{ runId: info.runId }}
              className="ml-auto font-mono text-2xs text-subtle-foreground transition-colors hover:text-foreground"
            >
              {shortId(info.runId, 7)}
            </Link>
          ) : null}
        </div>
        <p
          className={cn(
            "mt-0.5 text-pretty text-2xs leading-relaxed",
            info.error ? "text-destructive-strong" : "text-muted-foreground"
          )}
        >
          {info.error ?? info.current ?? description}
        </p>
      </div>
    </div>
  )
}

function OptionRow({
  icon: Icon,
  title,
  subtitle,
  selected,
  onSelect,
  badge,
  disabled,
  children,
}: {
  icon: typeof GitBranch
  title: string
  subtitle: string
  selected: boolean
  onSelect: () => void
  badge?: React.ReactNode
  disabled?: boolean
  children?: React.ReactNode
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      aria-disabled={disabled}
      disabled={disabled}
      onClick={onSelect}
      className={cn(
        "flex w-full items-start gap-3 rounded-xl border p-3 text-left transition-[background-color,border-color] duration-150",
        selected
          ? "border-accent/35 bg-accent-soft/40"
          : "border-border bg-surface/40 hover:border-border-strong hover:bg-muted/40",
        disabled && "cursor-not-allowed opacity-45"
      )}
    >
      <span
        className={cn(
          "mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-md border",
          selected
            ? "border-accent/25 bg-accent-soft text-accent"
            : "border-border bg-surface text-subtle-foreground"
        )}
      >
        <Icon className="size-3.5" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="flex flex-wrap items-center gap-2">
          <span className="text-sm font-medium text-foreground">{title}</span>
          {badge}
        </span>
        <span className="mt-0.5 block text-pretty text-xs leading-relaxed text-muted-foreground">
          {subtitle}
        </span>
        {children}
      </span>
      <span
        aria-hidden="true"
        className={cn(
          "mt-1 flex size-3.5 shrink-0 items-center justify-center rounded-full border",
          selected ? "border-accent bg-accent text-accent-foreground" : "border-border-strong"
        )}
      >
        {selected ? <Check className="size-2.5" /> : null}
      </span>
    </button>
  )
}

/**
 * The deploy flow, in the operator's terms.
 *
 * A release is still an immutable commit inside the system, but the operator
 * should never have to name one. This dialog resolves the newest source
 * version, offers built versions by build time, picks up the hex only as a
 * hidden technical detail, and can build a version before deploying it.
 */
export function DeployDialog({
  app,
  open,
  initialRelease,
  onOpenChange,
  onStarted,
}: {
  app: App
  open: boolean
  /** A release preselected by the version card; still requires confirmation. */
  initialRelease?: string | null
  onOpenChange: (open: boolean) => void
  onStarted?: (runId: string) => void
}) {
  const queryClient = useQueryClient()
  const aliveRef = useRef(true)
  const abortRef = useRef<AbortController | null>(null)

  const [choice, setChoice] = useState<Choice | null>(null)
  const [selectedImage, setSelectedImage] = useState<string | null>(null)
  const [specific, setSpecific] = useState("")
  const [phase, setPhase] = useState<Phase>("choose")
  const [plan, setPlan] = useState<{ build: boolean; transfer: boolean } | null>(null)
  const [stages, setStages] = useState<Record<StageId, StageInfo>>(emptyStages)
  const [logs, setLogs] = useState<LogLine[]>([])
  const [error, setError] = useState<string | null>(null)
  const [lastRun, setLastRun] = useState<string | null>(null)
  const [runPhase, setRunPhase] = useState<RunPhase | null>(null)

  const kind = deploymentKind(app)
  const isSource = kind === "source"
  const hasRepo = app.repo != null || app.build_repo != null
  const latestQuery = useQuery({
    queryKey: queryKeys.latest(app.name),
    queryFn: ({ signal }) => api.appLatest(app.name, signal),
    enabled: open && hasRepo,
    retry: 0,
  })
  const gitQuery = useQuery({
    queryKey: queryKeys.git(app.name),
    queryFn: ({ signal }) => api.appGit(app.name, signal),
    enabled: open && hasRepo,
    retry: 0,
  })
  const imagesQuery = useQuery({
    queryKey: queryKeys.images(app.name),
    queryFn: ({ signal }) => api.appImages(app.name, signal),
    enabled: open && app.kind === "container" && app.image_repo != null,
    retry: 0,
  })
  const configQuery = useQuery({ queryKey: queryKeys.config, queryFn: () => cfgState() })

  useEffect(() => {
    if (!open) return
    abortRef.current?.abort()
    abortRef.current = null
    aliveRef.current = true
    setChoice(initialRelease ? "specific" : null)
    setSelectedImage(null)
    setSpecific(initialRelease ?? "")
    setPhase("choose")
    setPlan(null)
    setStages(emptyStages())
    setLogs([])
    setError(null)
    setLastRun(null)
    setRunPhase(null)
  }, [open, initialRelease])

  useEffect(() => {
    aliveRef.current = true
    return () => {
      aliveRef.current = false
      abortRef.current?.abort()
    }
  }, [])

  const images = imagesQuery.data?.images ?? []
  const git = gitQuery.data
  const latestRelease = latestQuery.data?.release ?? git?.remote_sha ?? git?.head_sha ?? null
  const latestSource =
    latestQuery.data?.source ??
    (git?.remote_sha ? `origin/${app.branch ?? "main"}` : git?.head_sha ? "HEAD" : null)
  const latestImage = imageForRelease(images, latestRelease)
  const newestImage = newestReleaseImage(images)
  const builtVersions = releaseImages(images)
  const otherBuilt = builtVersions.filter(
    (image) => !sameRelease(image.tag, newestImage?.tag)
  )
  const selectedImageMeta = imageForRelease(images, selectedImage ?? newestImage?.tag)
  const liveRelease = app.release
  const branch = app.branch ?? "main"
  const latestBranch = latestSource?.startsWith("origin/")
    ? latestSource.slice("origin/".length)
    : branch
  const latestTitle = latestSource === "HEAD" ? "Latest from the checkout" : `Latest from ${latestBranch}`

  const fallbackChoice: Choice | null = latestRelease
    ? "latest"
    : newestImage
      ? "image"
      : liveRelease
        ? "live"
        : null
  const activeChoice = choice ?? fallbackChoice
  const selectedImageTag = selectedImage ?? newestImage?.tag ?? null

  const targetRelease = useMemo(() => {
    switch (activeChoice) {
      case "latest":
        return latestRelease
      case "image":
        return selectedImageTag
      case "live":
        return liveRelease
      case "specific":
        return specific.trim() === "" ? null : specific.trim()
      default:
        return null
    }
  }, [activeChoice, latestRelease, selectedImageTag, liveRelease, specific])

  const targetImage = imageForRelease(images, targetRelease)
  const targetInvalid = activeChoice === "specific" && specific.trim() !== "" && !isRelease(specific.trim())
  const isLiveTarget = sameRelease(targetRelease, liveRelease)
  const imageCheckApplies = app.kind === "container" && app.registry === "local"
  const imageKnown = imagesQuery.isSuccess
  const needsBuild =
    imageCheckApplies && imageKnown && targetRelease != null && !targetImage && !isLiveTarget
  const canBuild = app.image_repo != null && app.git_remote != null
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
  const configReady = configQuery.isSuccess || configQuery.isError
  const transferTarget = configQuery.data?.config.transferTarget?.trim() || null
  const needsTransfer =
    app.kind === "container" &&
    configReady &&
    !sameHost &&
    targetRelease != null &&
    !isLiveTarget

  const blocker = !targetRelease
    ? "Choose a version to deploy."
    : targetInvalid
      ? "That is not a commit reference: use 7–64 hexadecimal characters."
      : !configReady
        ? "Reading the transfer settings…"
        : needsBuild && !canBuild
          ? "This version has not been built, and the app has no source checkout to build from."
          : needsTransfer && !transferTarget
            ? "Configure a transfer target in config.json before sending an image between hosts."
            : null

  const chooseLabel =
    activeChoice === "latest"
      ? latestTitle
      : activeChoice === "image"
        ? sameRelease(selectedImageTag, newestImage?.tag)
          ? "Newest built version"
          : selectedImageMeta?.created
            ? `Version built ${formatRelative(dockerTime(selectedImageMeta.created) ?? 0)}`
            : "Built version"
        : activeChoice === "live"
          ? "Current live version"
          : activeChoice === "specific"
            ? "Specific commit"
            : "No version selected"

  const loading =
    (hasRepo && latestQuery.isLoading) ||
    (app.kind === "container" && app.image_repo != null && imagesQuery.isLoading)

  const canShip = phase === "choose" && blocker == null && !loading
  const shipLabel = needsBuild
    ? "Build & deploy"
    : needsTransfer
      ? "Send & deploy"
      : isLiveTarget
        ? "Redeploy"
        : "Deploy"

  const showLatest = latestRelease != null
  const showNewest =
    newestImage != null && !sameRelease(newestImage.tag, latestRelease)
  const showLive =
    liveRelease != null &&
    !sameRelease(liveRelease, latestRelease) &&
    !sameRelease(liveRelease, newestImage?.tag)

  const latestBuilt = dockerTime(latestImage?.created)
  const newestBuilt = dockerTime(newestImage?.created)

  const latestSubtitle = isLiveTarget
    ? "Already live. Redeploying runs the health gate again."
    : latestImage
      ? `Built ${latestBuilt ? formatRelative(latestBuilt) : "earlier"}${latestImage.size ? ` · ${latestImage.size}` : ""}.`
      : needsBuild
        ? "This version has not been built yet; the image is built first and then deployed."
        : imageCheckApplies && !imageKnown
          ? "The built-image state could not be checked. Deploying now will be refused if it is missing."
          : imageCheckApplies
            ? "Ready to deploy."
            : `The image is pulled from ${app.registry ?? "the registry"} during deploy.`

  const deployStarted = async () => {
    if (!canShip || !targetRelease) return
    const release = targetRelease
    const build = needsBuild
    const transfer = needsTransfer
    const force = sameRelease(release, app.release)

    const controller = new AbortController()
    abortRef.current?.abort()
    abortRef.current = controller
    setPlan({ build, transfer })
    setStages({
      build: { ...emptyStage(), state: build ? "pending" : "skipped" },
      transfer: { ...emptyStage(), state: transfer ? "pending" : "skipped" },
      deploy: { ...emptyStage(), state: "pending" },
    })
    setLogs([])
    setError(null)
    setLastRun(null)
    setRunPhase(null)
    setPhase("running")

    try {
      // Build, transfer and deploy are one job on the server, so the release
      // still moves after this tab is closed. Watching it is optional.
      const started = await api.deploy(app.name, release, force, {
        build,
        transfer: needsTransfer ? (transferTarget ?? undefined) : undefined,
      }, controller.signal)
      const runId = started.run_id
      setLastRun(runId)
      onStarted?.(runId)

      let active: StageId = build ? "build" : transfer ? "transfer" : "deploy"
      setStages((current) => ({
        ...current,
        [active]: { ...current[active], state: "active", runId },
      }))

      const result = await followRun(
        runId,
        "deploy",
        {
          onStep: (event) => {
            if (!aliveRef.current) return
            const next = stageOf(event.step)
            if (next !== active) {
              const previous = active
              active = next
              setStages((current) => ({
                ...current,
                [previous]: { ...current[previous], state: "ok", current: null },
                [next]: { ...current[next], state: "active", current: event.step ?? null },
              }))
            } else {
              setStages((current) => ({
                ...current,
                [next]: { ...current[next], current: event.step ?? null },
              }))
            }
            setLogs((current) =>
              [
                ...current,
                {
                  stage: next,
                  step: event.step ?? "step",
                  status: event.status,
                  durationMs: event.duration_ms,
                  error: event.error,
                },
              ].slice(-80)
            )
          },
          onPhase: (next) => {
            if (aliveRef.current) setRunPhase(next)
          },
        },
        controller.signal
      )

      if (result.status === null && result.error === "cancelled") return
      const outcome = classifyStatus(result.status)
      if (outcome === "ok" || outcome === "dry") {
        if (!aliveRef.current) return
        setStages((current) => ({
          build: { ...current.build, state: build ? "ok" : "skipped", current: null },
          transfer: { ...current.transfer, state: transfer ? "ok" : "skipped", current: null },
          deploy: { ...current.deploy, state: "ok", current: null },
        }))
        setPhase("done")
        setRunPhase(null)
        toast.success(`${app.name} is deployed`, {
          description: `${chooseLabel}${force ? " · health gate re-run" : ""}`,
        })
        void queryClient.invalidateQueries({ queryKey: queryKeys.app(app.name) })
        void queryClient.invalidateQueries({ queryKey: queryKeys.apps })
        void queryClient.invalidateQueries({ queryKey: queryKeys.runs })
        void queryClient.invalidateQueries({ queryKey: queryKeys.appStatus(app.name) })
        void queryClient.invalidateQueries({ queryKey: queryKeys.latest(app.name) })
        return
      }

      const message = result.error ?? `The run ended ${result.status ?? "without a status"}.`
      if (aliveRef.current) {
        setStages((current) => ({
          ...current,
          [active]: { ...current[active], state: "failed", error: message },
        }))
        setPhase("failed")
        setRunPhase(null)
        setError(message)
        toast.error("Deploy did not finish", { description: message })
      }
      void queryClient.invalidateQueries({ queryKey: queryKeys.runs })
    } catch (caught) {
      const message = errorText(caught)
      if (message === "cancelled") return
      if (!aliveRef.current) return
      setPhase("failed")
      setRunPhase(null)
      setError(message)
      toast.error("Deploy did not finish", { description: message })
    }
  }

  const handleOpenChange = (next: boolean) => {
    // Closing only stops the watch. The job is on the server and finishes
    // whether or not this dialog is still on screen.
    if (!next && phase === "running") {
      abortRef.current?.abort()
      if (lastRun) {
        toast.message("Deploy continues in the background", {
          description: "Follow it under Runs whenever you like.",
        })
      }
    }
    onOpenChange(next)
  }

  const retry = () => {
    abortRef.current?.abort()
    abortRef.current = null
    setPhase("choose")
    setPlan(null)
    setStages(emptyStages())
    setLogs([])
    setError(null)
    setRunPhase(null)
  }

  const planStages: { id: StageId; description: string }[] = []
  if (needsBuild) {
    planStages.push({
      id: "build",
      description: `Build ${app.image_repo ?? "the image"} from ${branch}.`,
    })
  }
  if (needsTransfer) {
    planStages.push({
      id: "transfer",
      description: `Send the built image to ${transferTarget ?? "the deploy host"}.`,
    })
  }
  planStages.push({
    id: "deploy",
    description:
      app.kind === "static"
        ? "Point the front door at the new release."
        : `Flip nginx and run the ${app.strategy} health gate.`,
  })

  const visibleStages = plan
    ? [
        ...(plan.build ? (["build"] as StageId[]) : []),
        ...(plan.transfer ? (["transfer"] as StageId[]) : []),
        "deploy" as StageId,
      ]
    : planStages.map((stage) => stage.id)

  const choiceIcon =
    activeChoice === "latest"
      ? Sparkles
      : activeChoice === "image"
        ? Box
        : activeChoice === "live"
          ? Rocket
          : Terminal

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>
            {phase === "done"
              ? `${app.name} is deployed`
              : phase === "failed"
                ? "Deploy did not finish"
                : phase === "running"
                  ? `Deploying ${app.name}`
                  : `Deploy ${app.name}`}
          </DialogTitle>
          <DialogDescription>
            {phase === "choose"
              ? isSource
                ? "Pick a version. swapdock resolves the commit, builds the image if it is missing, and runs the health gate. You never need the commit hash."
                : "Pick a built image version. This app runs a prebuilt image, so there is nothing to build and no commit hash is involved."
              : phase === "running"
                ? "The release is moving through build, transfer and the health gate. This runs on the server — you can close the dialog and follow the run later."
                : phase === "done"
                  ? "The health gate passed and traffic is on the new release."
                  : "The release stopped before it finished. Nothing further will be attempted."}
          </DialogDescription>
        </DialogHeader>

        {phase === "choose" ? (
          <div className="space-y-3">
            {loading ? (
              <div className="space-y-2">
                <div className="flex items-center gap-2 text-xs text-muted-foreground">
                  <Loader2 className="size-3.5 animate-spin" />
                  Finding the newest version…
                </div>
                <Skeleton className="h-[62px] w-full rounded-xl" />
                <Skeleton className="h-[62px] w-full rounded-xl" />
              </div>
            ) : (
              <div role="radiogroup" aria-label="Version to deploy" className="space-y-2">
                {showLatest ? (
                  <OptionRow
                    icon={GitBranch}
                    title={latestTitle}
                    subtitle={latestSubtitle}
                    selected={activeChoice === "latest"}
                    onSelect={() => setChoice("latest")}
                    badge={
                      isLiveTarget ? (
                        <Badge variant="outline">Live</Badge>
                      ) : latestImage ? (
                        <Badge variant="success">Built</Badge>
                      ) : needsBuild ? (
                        <Badge variant="info">Will build</Badge>
                      ) : (
                        <Badge variant="outline">Ready</Badge>
                      )
                    }
                  />
                ) : null}

                {showNewest && newestImage ? (
                  <OptionRow
                    icon={Box}
                    title="Newest built version"
                    subtitle={`Built ${newestBuilt ? formatRelative(newestBuilt) : "earlier"}${newestImage.size ? ` · ${newestImage.size}` : ""}. No build needed.`}
                    selected={
                      activeChoice === "image" &&
                      sameRelease(selectedImageTag, newestImage.tag)
                    }
                    onSelect={() => {
                      setChoice("image")
                      setSelectedImage(newestImage.tag)
                    }}
                    badge={<Badge variant="success">Built</Badge>}
                  />
                ) : null}

                {showLive ? (
                  <OptionRow
                    icon={Rocket}
                    title="Current live version"
                    subtitle={`Serving on :${app.live_port ?? "—"}. Redeploy what is live now; the health gate runs again.`}
                    selected={activeChoice === "live"}
                    onSelect={() => setChoice("live")}
                    badge={<Badge variant="outline">Live</Badge>}
                  />
                ) : null}

                {otherBuilt.length > 0 ? (
                  <details className="group rounded-xl border border-border bg-surface/30">
                    <summary className="flex cursor-pointer list-none items-center gap-2 px-3 py-2.5 text-xs text-muted-foreground select-none">
                      <Box className="size-3.5" />
                      Choose another built version
                      <span className="text-2xs text-subtle-foreground">
                        {otherBuilt.length}
                      </span>
                      <ChevronDown className="ml-auto size-3 transition-transform duration-150 group-open:rotate-180" />
                    </summary>
                    <div className="space-y-1 border-t border-border/60 p-2">
                      {otherBuilt.slice(0, 6).map((image) => {
                        const created = dockerTime(image.created)
                        const selected =
                          activeChoice === "image" &&
                          sameRelease(selectedImageTag, image.tag)
                        return (
                          <button
                            key={image.tag}
                            type="button"
                            onClick={() => {
                              setChoice("image")
                              setSelectedImage(image.tag)
                            }}
                            className={cn(
                              "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs transition-colors duration-150",
                              selected ? "bg-accent-soft text-foreground" : "hover:bg-muted/50"
                            )}
                          >
                            <span className="min-w-0 flex-1 truncate">
                              {created ? formatShortDateTime(created) : "Build time unknown"}
                              <span className="ml-1.5 text-2xs text-subtle-foreground">
                                {created ? formatRelative(created) : ""}
                              </span>
                            </span>
                            {image.size ? (
                              <span className="shrink-0 text-2xs text-subtle-foreground">
                                {image.size}
                              </span>
                            ) : null}
                            <span
                              className="shrink-0 font-mono text-2xs text-subtle-foreground"
                              title={image.tag}
                            >
                              {image.tag.slice(0, 7)}
                            </span>
                            {sameRelease(image.tag, app.release) ? (
                              <Badge variant="outline">Live</Badge>
                            ) : null}
                            {selected ? <Check className="size-3 text-accent" /> : null}
                          </button>
                        )
                      })}
                    </div>
                  </details>
                ) : null}

                {isSource ? (
                  <>
                    <button
                      type="button"
                      onClick={() => setChoice("specific")}
                      className={cn(
                        "flex w-full items-center gap-2 rounded-xl border p-2.5 text-left text-xs transition-colors duration-150",
                        activeChoice === "specific"
                          ? "border-accent/35 bg-accent-soft/40 text-foreground"
                          : "border-border bg-surface/30 text-muted-foreground hover:border-border-strong hover:bg-muted/40"
                      )}
                    >
                      <Terminal className="size-3.5" />
                      <span className="flex-1">Deploy a specific commit</span>
                      <span className="text-2xs text-subtle-foreground">Advanced</span>
                      <ChevronDown
                        className={cn(
                          "size-3 transition-transform duration-150",
                          activeChoice === "specific" && "rotate-180"
                        )}
                      />
                    </button>

                    {activeChoice === "specific" ? (
                      <div className="animate-fade-in space-y-1.5 rounded-xl border border-border bg-surface/40 p-3">
                        <label
                          htmlFor="deploy-specific"
                          className="text-xs font-medium text-muted-foreground"
                        >
                          Commit
                        </label>
                        <Input
                          id="deploy-specific"
                          value={specific}
                          onChange={(event) => setSpecific(event.target.value)}
                          placeholder="a commit you were given"
                          className="font-mono"
                          autoComplete="off"
                          spellCheck={false}
                          aria-invalid={targetInvalid}
                        />
                        <p className="text-2xs text-muted-foreground">
                          Only when you have a reason. Normal deploys use the versions above.
                        </p>
                      </div>
                    ) : null}
                  </>
                ) : null}
              </div>
            )}

            {!loading && !showLatest && !showNewest && !showLive && activeChoice !== "specific" ? (
              <Callout tone="warning" title="No version could be resolved">
                There is no built image and no live release for this app yet. Pull an image
                outside swapdock or set up a source repository.
              </Callout>
            ) : null}

            <div className="rounded-xl border border-border bg-muted/20 p-3">
              <div className="flex flex-wrap items-center gap-x-2 gap-y-1.5 text-xs">
                <span className="label-eyebrow mr-1">What happens</span>
                {needsBuild ? (
                  <>
                    <span className="inline-flex items-center gap-1 text-muted-foreground">
                      <Hammer className="size-3" />
                      Build
                    </span>
                    <ArrowRight className="size-2.5 text-subtle-foreground/60" />
                  </>
                ) : null}
                {needsTransfer ? (
                  <>
                    <span className="inline-flex items-center gap-1 text-muted-foreground">
                      <Upload className="size-3" />
                      Send
                    </span>
                    <ArrowRight className="size-2.5 text-subtle-foreground/60" />
                  </>
                ) : null}
                <span className="inline-flex items-center gap-1 text-foreground">
                  <Rocket className="size-3" />
                  {app.kind === "static" ? "Publish" : "Deploy + health gate"}
                </span>
                {!needsBuild && targetImage ? (
                  <span className="ml-auto text-2xs text-subtle-foreground">Image already built</span>
                ) : null}
              </div>
              <p className="mt-1.5 text-pretty text-2xs leading-relaxed text-subtle-foreground">
                {blocker ??
                  (needsBuild
                    ? "The commit is checked out on the build host, the image is tagged, then the health gate opens."
                    : "No commit hash needed. The version above is resolved and deployed for you.")}
              </p>
            </div>

            {targetInvalid ? (
              <p role="alert" className="text-xs text-destructive-strong">
                A commit reference is 7–64 hexadecimal characters.
              </p>
            ) : null}
            {error && phase === "choose" ? (
              <Callout tone="danger" title="The last attempt did not start">
                {error}
              </Callout>
            ) : null}

            <details className="group rounded-lg border border-border bg-surface/30">
              <summary className="flex cursor-pointer list-none items-center gap-2 px-3 py-2 text-2xs text-subtle-foreground select-none">
                <span>Technical details</span>
                <ChevronDown className="size-3 transition-transform duration-150 group-open:rotate-180" />
              </summary>
              <div className="space-y-2 border-t border-border/60 px-3 py-2.5">
                <div className="flex items-center gap-2">
                  <span className="label-eyebrow w-16">Release</span>
                  <span className="min-w-0 flex-1 truncate font-mono text-2xs text-muted-foreground">
                    {targetRelease ?? "—"}
                  </span>
                  {targetRelease ? <CopyButton value={targetRelease} label="Copy release" /> : null}
                </div>
                {isSource ? (
                  <div className="flex items-center gap-2">
                    <span className="label-eyebrow w-16">Source</span>
                    <span className="min-w-0 flex-1 truncate font-mono text-2xs text-muted-foreground">
                      {latestSource ?? branch}
                    </span>
                  </div>
                ) : null}
                <div className="flex items-center gap-2">
                  <span className="label-eyebrow w-16">Image</span>
                  <span className="min-w-0 flex-1 truncate font-mono text-2xs text-muted-foreground">
                    {app.image_repo && targetRelease ? `${app.image_repo}:${targetRelease}` : "not applicable"}
                  </span>
                </div>
                {targetImage?.created ? (
                  <div className="flex items-center gap-2">
                    <span className="label-eyebrow w-16">Built</span>
                    <span className="text-2xs text-muted-foreground">{targetImage.created}</span>
                  </div>
                ) : null}
              </div>
            </details>
          </div>
        ) : (
          <div className="space-y-3">
            <div className="flex items-center gap-2.5">
              {(() => {
                const Icon = choiceIcon
                return (
                  <span className="flex size-7 items-center justify-center rounded-md border border-border bg-surface text-muted-foreground">
                    <Icon className="size-3.5" />
                  </span>
                )
              })()}
              <div className="min-w-0 flex-1">
                <div className="label-eyebrow">Version</div>
                <div className="truncate text-sm font-medium text-foreground">{chooseLabel}</div>
              </div>
              {phase === "running" && runPhase ? (
                <span className="inline-flex items-center gap-1.5 text-2xs text-info">
                  <StatusDot tone="info" pulse />
                  {runPhase === "polling"
                    ? "following the run"
                    : runPhase === "reconnecting"
                      ? "reconnecting"
                      : "live"}
                </span>
              ) : phase === "done" ? (
                <StatusPill tone="success" label="Live" />
              ) : phase === "failed" ? (
                <StatusPill tone="danger" label="Stopped" />
              ) : null}
            </div>

            <div className="overflow-hidden rounded-xl border border-border">
              {visibleStages.map((id) => (
                <PlanStage
                  key={id}
                  id={id}
                  info={stages[id]}
                  label={STAGE_LABELS[id]}
                  description={
                    id === "build"
                      ? `Building ${app.image_repo ?? "the image"} from ${branch}.`
                      : id === "transfer"
                        ? `Sending the image to ${transferTarget ?? "the deploy host"}.`
                        : app.kind === "static"
                          ? "Publishing the new atomic root."
                          : `Flipping nginx and running the ${app.strategy} health gate.`
                  }
                />
              ))}
            </div>

            {logs.length > 0 ? (
              <div
                aria-live="polite"
                className="max-h-40 overflow-y-auto rounded-lg border border-border bg-surface p-2.5 font-mono text-2xs leading-5"
              >
                {logs.slice(-40).map((line, index) => (
                  <div key={index} className="flex items-center gap-2">
                    <span className="w-12 shrink-0 text-subtle-foreground">{line.stage}</span>
                    <StatusDot tone={runTone(line.status)} className="size-1" />
                    <span className="min-w-0 flex-1 truncate text-muted-foreground">{line.step}</span>
                    {line.durationMs != null ? (
                      <span className="shrink-0 text-subtle-foreground">
                        {formatDuration(line.durationMs)}
                      </span>
                    ) : null}
                    {line.error ? (
                      <span className="max-w-[40%] truncate text-destructive-strong">{line.error}</span>
                    ) : null}
                  </div>
                ))}
              </div>
            ) : phase === "running" ? (
              <p className="flex items-center gap-2 text-2xs text-muted-foreground">
                <Loader2 className="size-3 animate-spin" />
                Waiting for the first step…
              </p>
            ) : null}

            {phase === "done" ? (
              <Callout tone="success" title="Release is live">
                The health gate passed and the previous version is retained for rollback.
              </Callout>
            ) : null}
            {phase === "failed" && error ? (
              <Callout tone="danger" title="The release stopped">
                {error}
              </Callout>
            ) : null}
          </div>
        )}

        <DialogFooter>
          {phase === "choose" ? (
            <>
              <Button variant="outline" onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
              <Button disabled={!canShip} onClick={() => void deployStarted()}>
                {needsBuild ? <Hammer /> : <Rocket />}
                {shipLabel}
                {!needsBuild ? <ArrowRight /> : null}
              </Button>
            </>
          ) : phase === "running" ? (
            <>
              <span className="mr-auto flex items-center gap-2 text-2xs text-muted-foreground">
                <Loader2 className="size-3 animate-spin" />
                Runs in the background — closing this dialog is safe.
              </span>
              <Button disabled>
                <Loader2 className="animate-spin" />
                In progress
              </Button>
            </>
          ) : phase === "done" ? (
            <>
              {lastRun ? (
                <Button
                  variant="outline"
                  render={<Link to="/runs/$runId" params={{ runId: lastRun }} />}
                >
                  View run
                  <ArrowRight />
                </Button>
              ) : null}
              <Button onClick={() => onOpenChange(false)}>Done</Button>
            </>
          ) : (
            <>
              <Button variant="outline" onClick={() => onOpenChange(false)}>
                Close
              </Button>
              <Button onClick={retry}>
                Try again
                <ArrowRight />
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
