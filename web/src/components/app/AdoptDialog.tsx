import { useEffect, useMemo, useState } from "react"
import { Link } from "@tanstack/react-router"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"
import {
  ArrowRight,
  Check,
  Container,
  PackagePlus,
  RefreshCw,
  TriangleAlert,
} from "lucide-react"
import { api, type App } from "@/api"
import { queryKeys } from "@/lib/query-keys"
import { errorText, formatRelative, isRelease, shortId } from "@/lib/format"
import { imageRepository, snapshotRelease } from "@/lib/deployment"
import { followRun } from "@/lib/run-tail"
import { cn } from "cn"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Callout, MetaList } from "@/components/kit"

/**
 * The one-time cutover for an app whose container already publishes its front
 * port directly.
 *
 * A normal replace deploy needs a live port that swapdock owns; until then the
 * only correct action is adoption. There is no commit to know here: the running
 * image is preserved under a generated snapshot release. A specific commit is
 * available as an advanced choice for source-built apps, but it is not required.
 */
export function AdoptDialog({
  app,
  open,
  onOpenChange,
  onStarted,
}: {
  app: App
  open: boolean
  onOpenChange: (open: boolean) => void
  onStarted?: (runId: string) => void
}) {
  const queryClient = useQueryClient()
  const [snapshot, setSnapshot] = useState(() => snapshotRelease())
  const [useCommit, setUseCommit] = useState(false)
  const [commit, setCommit] = useState("")
  const [healthcheck, setHealthcheck] = useState("")
  const [containerPort, setContainerPort] = useState("")
  const [startedRun, setStartedRun] = useState<string | null>(null)

  const inspectQuery = useQuery({
    queryKey: queryKeys.inspect(app.name),
    queryFn: ({ signal }) => api.inspect(app.name, signal),
    enabled: open,
    retry: 0,
  })

  const doctorQuery = useQuery({
    queryKey: queryKeys.appDoctor(app.name),
    queryFn: ({ signal }) => api.doctor(app.name, signal),
    enabled: open,
    retry: 0,
    staleTime: 120_000,
    refetchOnWindowFocus: false,
  })

  useEffect(() => {
    if (!open) return
    setSnapshot(snapshotRelease())
    setUseCommit(false)
    setCommit("")
    setHealthcheck("")
    setContainerPort("")
    setStartedRun(null)
  }, [open])

  const suggestedProbe = useMemo(() => {
    try {
      const health = new URL(app.health_url ?? "http://127.0.0.1/")
      return `curl -fsS http://127.0.0.1${health.port ? `:${health.port}` : ""}${health.pathname}${health.search}`
    } catch {
      return "curl -fsS http://127.0.0.1/"
    }
  }, [app.health_url])

  const detectedImage = inspectQuery.data?.container?.image ?? null
  const detectedRepo = imageRepository(detectedImage)
  const noContainer = inspectQuery.isSuccess && inspectQuery.data?.container == null
  const imageRepoMissing = app.image_repo == null
  const noHealthcheck =
    doctorQuery.data?.problems.some((problem) => problem.code === "no-healthcheck") ?? false
  const commitValid = isRelease(commit.trim())
  const release = useCommit ? commit.trim() : snapshot
  const canAdopt =
    !imageRepoMissing &&
    !noContainer &&
    (!useCommit || commitValid) &&
    (!noHealthcheck || healthcheck.trim() !== "")

  const blockedReason = imageRepoMissing
    ? "Set an image repository first; adoption needs a local name for the running image."
    : noContainer
      ? `No container currently publishes :${app.front_port}. There is nothing to take over yet.`
      : noHealthcheck && healthcheck.trim() === ""
        ? "The compose service has no healthcheck. Enter one below, or add one to the compose file."
        : useCommit && !commitValid
          ? "A specific commit must be 7–64 hexadecimal characters."
          : null

  const saveDetectedRepo = useMutation({
    mutationFn: (repo: string) =>
      api.updateApp(app.name, { ...app, image_repo: repo }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.app(app.name) })
      void queryClient.invalidateQueries({ queryKey: queryKeys.apps })
      void queryClient.invalidateQueries({ queryKey: queryKeys.validate })
      void queryClient.invalidateQueries({ queryKey: queryKeys.appDoctor(app.name) })
      toast.success("Image repository set", {
        description: detectedRepo ?? undefined,
      })
    },
    onError: (error) => {
      toast.error("Could not save the image repository", {
        description: errorText(error),
      })
    },
  })

  const adopt = useMutation({
    mutationFn: () =>
      api.adopt(app.name, {
        release,
        healthcheck: healthcheck.trim() === "" ? undefined : healthcheck.trim(),
        container_port:
          containerPort.trim() === ""
            ? undefined
            : Number.isInteger(Number(containerPort))
              ? Number(containerPort)
              : undefined,
      }),
    onSuccess: ({ run_id }) => {
      setStartedRun(run_id)
      const refresh = () => {
        void queryClient.invalidateQueries({ queryKey: queryKeys.app(app.name) })
        void queryClient.invalidateQueries({ queryKey: queryKeys.apps })
        void queryClient.invalidateQueries({ queryKey: queryKeys.runs })
        void queryClient.invalidateQueries({ queryKey: queryKeys.appStatus(app.name) })
        void queryClient.invalidateQueries({ queryKey: queryKeys.inspect(app.name) })
        void queryClient.invalidateQueries({ queryKey: queryKeys.appDoctor(app.name) })
      }
      refresh()
      // The page should flip from Unmanaged to managed as soon as the cutover
      // commits, not on the next 60s apps poll.
      void followRun(run_id, "deploy").then(refresh)
      onStarted?.(run_id)
      toast.success("Adoption started", {
        description: `${app.name} · run ${shortId(run_id)}`,
      })
    },
    onError: (error) => {
      toast.error("Adoption did not start", { description: errorText(error) })
    },
  })

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <PackagePlus className="size-4 text-accent" />
            {startedRun ? `Taking over ${app.name}` : `Take over ${app.name}`}
          </DialogTitle>
          <DialogDescription>
            {startedRun
              ? "swapdock is moving nginx in front of the running container. Follow the run for the cutover steps."
              : `nginx does not own this app yet. swapdock will preserve the running image, stop the legacy container, and take over port :${app.front_port}. One time only — you do not need a commit hash.`}
          </DialogDescription>
        </DialogHeader>

        {startedRun ? (
          <div className="space-y-3">
            <Callout tone="success" title="Adoption is running">
              The app is managed after the run commits the registry and probes the front
              door. The page will update when it finishes.
            </Callout>
            <div className="rounded-xl border border-border bg-muted/25 p-3">
              <MetaList
                columns={2}
                items={[
                  { label: "App", value: app.name },
                  { label: "Run", value: shortId(startedRun), mono: true },
                  { label: "Front port", value: `:${app.front_port}`, mono: true },
                  {
                    label: "Version",
                    value: useCommit ? "specific commit" : "running image snapshot",
                  },
                ]}
              />
            </div>
          </div>
        ) : (
          <div className="min-w-0 space-y-3">
            <div className="min-w-0 overflow-hidden rounded-xl border border-border bg-muted/20 p-3">
              <div className="flex items-center justify-between gap-3">
                <span className="label-eyebrow">Legacy container</span>
                {inspectQuery.isFetching ? (
                  <span className="inline-flex items-center gap-1 text-2xs text-subtle-foreground">
                    <RefreshCw className="size-3 animate-spin" />
                    inspecting
                  </span>
                ) : null}
              </div>
              {inspectQuery.isError ? (
                <p className="mt-1 text-xs text-muted-foreground">
                  Could not inspect the host. Adoption will still locate the container by
                  its front port.
                </p>
              ) : noContainer ? (
                <p className="mt-1 text-xs text-muted-foreground">
                  No container is publishing :{app.front_port} right now.
                </p>
              ) : (
                <div className="mt-1.5 min-w-0 space-y-1.5">
                  <div className="flex min-w-0 items-center gap-2">
                    <Container className="size-3.5 shrink-0 text-subtle-foreground" />
                    <span className="min-w-0 flex-1 truncate font-mono text-xs text-foreground">
                      {inspectQuery.data?.container?.name ?? "detecting…"}
                    </span>
                  </div>
                  <div className="flex min-w-0 items-center gap-2">
                    <span className="shrink-0 text-2xs text-subtle-foreground">image</span>
                    <span className="min-w-0 flex-1 truncate font-mono text-2xs text-muted-foreground">
                      {detectedImage ?? "unknown"}
                    </span>
                  </div>
                </div>
              )}
            </div>

            <div className="rounded-xl border border-border bg-surface/40 p-3">
              <div className="flex items-center gap-2">
                <span className="label-eyebrow">Version to preserve</span>
                <span className="text-2xs text-subtle-foreground">generated for you</span>
              </div>
              <div className={cn("mt-2 flex items-center gap-2", useCommit && "opacity-55")}>
                <span
                  className={cn(
                    "flex size-5 items-center justify-center rounded-full border",
                    useCommit
                      ? "border-border-strong bg-surface"
                      : "border-success/30 bg-success/10 text-success"
                  )}
                >
                  {useCommit ? null : <Check className="size-3" />}
                </span>
                <div className="min-w-0 flex-1">
                  <div className="text-sm text-foreground">Snapshot of the running image</div>
                  <div className="text-2xs text-muted-foreground">
                    Generated {formatRelative(Date.now())}; no commit needed.
                  </div>
                </div>
              </div>

              <button
                type="button"
                onClick={() => setUseCommit((value) => !value)}
                className="mt-2.5 text-left text-2xs text-subtle-foreground transition-colors hover:text-muted-foreground"
              >
                {useCommit ? "Use the generated snapshot instead" : "I know the commit — use it instead"}
              </button>

              {useCommit ? (
                <div className="animate-fade-in mt-2 space-y-1.5">
                  <label htmlFor="adopt-commit" className="text-2xs text-muted-foreground">
                    Commit
                  </label>
                  <Input
                    id="adopt-commit"
                    value={commit}
                    onChange={(event) => setCommit(event.target.value)}
                    placeholder="a 7–64 character commit"
                    className="font-mono"
                    autoComplete="off"
                    spellCheck={false}
                    aria-invalid={commit.trim() !== "" && !commitValid ? true : undefined}
                  />
                  <p className="text-2xs text-subtle-foreground">
                    The recorded release is this commit instead of the snapshot label.
                  </p>
                </div>
              ) : null}
            </div>

            {noHealthcheck ? (
              <Callout tone="warning" title="This service has no healthcheck">
                The compose service does not define one, so adoption needs a command that
                exits 0 when the app is healthy. Enter it below, or add a healthcheck to
                the compose file and reopen this dialog.
              </Callout>
            ) : null}

            <div className="grid min-w-0 gap-3 sm:grid-cols-2">
              <div className="min-w-0 space-y-1.5">
                <div className="flex items-center justify-between gap-2">
                  <label
                    htmlFor="adopt-healthcheck"
                    className="text-2xs font-medium text-muted-foreground"
                  >
                    Healthcheck{" "}
                    <span className="font-normal">
                      {noHealthcheck ? "(required)" : "(optional)"}
                    </span>
                  </label>
                  {noHealthcheck ? (
                    <button
                      type="button"
                      onClick={() => setHealthcheck(suggestedProbe)}
                      className="text-2xs text-subtle-foreground underline-offset-4 transition-colors hover:text-muted-foreground hover:underline"
                    >
                      Use HTTP probe
                    </button>
                  ) : null}
                </div>
                <Input
                  id="adopt-healthcheck"
                  value={healthcheck}
                  onChange={(event) => setHealthcheck(event.target.value)}
                  placeholder={
                    noHealthcheck
                      ? suggestedProbe
                      : "only if compose has none"
                  }
                  className="font-mono"
                  autoComplete="off"
                  spellCheck={false}
                  aria-invalid={noHealthcheck && healthcheck.trim() === "" ? true : undefined}
                />
              </div>
              <div className="min-w-0 space-y-1.5">
                <label htmlFor="adopt-port" className="text-2xs font-medium text-muted-foreground">
                  Container port <span className="font-normal">(optional)</span>
                </label>
                <Input
                  id="adopt-port"
                  type="number"
                  min={1}
                  max={65535}
                  value={containerPort}
                  onChange={(event) => setContainerPort(event.target.value)}
                  placeholder="inferred"
                  className="w-full"
                />
              </div>
            </div>

            {imageRepoMissing ? (
              <Callout
                tone="warning"
                title="An image repository is required"
                icon={TriangleAlert}
                action={
                  <div className="flex items-center gap-1.5">
                    {detectedRepo ? (
                      <Button
                        variant="outline"
                        size="xs"
                        disabled={saveDetectedRepo.isPending}
                        onClick={() => saveDetectedRepo.mutate(detectedRepo)}
                      >
                        {saveDetectedRepo.isPending ? "Saving…" : "Use detected"}
                      </Button>
                    ) : null}
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
                      onClick={() => onOpenChange(false)}
                    >
                      Configure
                      <ArrowRight />
                    </Button>
                  </div>
                }
              >
                Adoption preserves the running image under this app's image repo.
                {detectedRepo ? (
                  <>
                    {" "}
                    Use the detected repository{" "}
                    <span className="font-mono">{detectedRepo}</span>, or set another one in
                    Configuration.
                  </>
                ) : null}
              </Callout>
            ) : null}

            <Callout tone="warning" title="This is a one-time cutover">
              The legacy container stops, nginx takes the front port, and the adopted
              release starts on a managed backend port. The previous image is retained for
              rollback.
            </Callout>

            {adopt.isError ? (
              <Callout tone="danger" title="Adoption was refused">
                {errorText(adopt.error)}
              </Callout>
            ) : null}
          </div>
        )}

        <DialogFooter>
          {startedRun ? (
            <>
              <Button
                variant="outline"
                render={<Link to="/runs/$runId" params={{ runId: startedRun }} />}
              >
                View run
                <ArrowRight />
              </Button>
              <Button onClick={() => onOpenChange(false)}>Done</Button>
            </>
          ) : (
            <>
              <Button variant="outline" onClick={() => onOpenChange(false)}>
                Cancel
              </Button>
              <Button
                disabled={!canAdopt || adopt.isPending}
                title={blockedReason ?? undefined}
                onClick={() => adopt.mutate()}
              >
                <PackagePlus />
                {adopt.isPending ? "Starting…" : "Take over app"}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
