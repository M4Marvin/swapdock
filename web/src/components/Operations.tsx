import { useMemo, useState } from "react"
import { Link } from "@tanstack/react-router"
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"
import {
  ArrowRight,
  Box,
  Code2,
  Globe,
  HardDrive,
  PackagePlus,
  RefreshCw,
  ScanSearch,
  Stethoscope,
  Trash,
} from "lucide-react"
import {
  api,
  type App,
  type InspectResult,
  type Problem,
  type ProbeResult,
} from "@/api"
import { queryKeys } from "@/lib/query-keys"
import { errorText, plural, shortId } from "@/lib/format"
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
import { Callout, CopyButton, MetaList, SectionHeading } from "@/components/kit"
import { StatusPill } from "@/components/Status"
import { AdoptDialog } from "@/components/app/AdoptDialog"

/** Hosts the probe API is expected to accept for this app. */
function probeHosts(app: App): string[] {
  const hosts = new Set(app.hostnames.map((host) => host.toLowerCase()))
  if (app.public_url) {
    try {
      hosts.add(new URL(app.public_url).hostname.toLowerCase())
    } catch {
      // A relative or malformed public_url is ignored; the server still decides.
    }
  }
  return [...hosts]
}

/** Client-side hint only: the allowlist may be wider than the registry. */
function probeTargetCheck(app: App, value: string) {
  const raw = value.trim()
  if (raw === "") return { invalid: false, warning: null as string | null }
  if (!/^https?:\/\//i.test(raw)) {
    return { invalid: true, warning: "Enter a full http:// or https:// URL." }
  }
  let host: string
  try {
    host = new URL(raw).hostname.toLowerCase()
  } catch {
    return { invalid: true, warning: "That URL could not be parsed." }
  }
  const allowed = probeHosts(app)
  if (allowed.length === 0) {
    return {
      invalid: false,
      warning:
        "No hostnames or public_url are configured for this app; the API will refuse a target outside its allowlist.",
    }
  }
  if (!allowed.includes(host)) {
    return {
      invalid: false,
      warning: `The API is restricted to ${allowed.join(", ")} (or its own SWAPDOCK_PROBE_ALLOWLIST). This host may be refused.`,
    }
  }
  return { invalid: false, warning: null as string | null }
}

function ActionCard({
  icon: Icon,
  title,
  description,
  badge,
  action,
  footer,
  className,
}: {
  icon: typeof Stethoscope
  title: string
  description: string
  badge?: React.ReactNode
  action: React.ReactNode
  footer?: React.ReactNode
  className?: string
}) {
  return (
    <section
      className={cn(
        "flex flex-col rounded-xl border border-border bg-card p-4 shadow-[var(--shadow-card)]",
        className
      )}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 items-start gap-3">
          <span className="flex size-8 shrink-0 items-center justify-center rounded-lg border border-border bg-surface text-muted-foreground">
            <Icon className="size-4" />
          </span>
          <div className="min-w-0">
            <h3 className="text-sm font-medium tracking-[-0.01em] text-foreground">{title}</h3>
            <p className="mt-0.5 text-pretty text-xs leading-relaxed text-muted-foreground">
              {description}
            </p>
          </div>
        </div>
        {badge}
      </div>
      <div className="mt-auto flex flex-wrap items-center gap-2 pt-4">{action}</div>
      {footer ? <div className="mt-2 text-2xs text-subtle-foreground">{footer}</div> : null}
    </section>
  )
}

function SeverityMark({ severity }: { severity: Problem["severity"] }) {
  return (
    <span
      className={cn(
        "mt-1.5 size-1.5 shrink-0 rounded-full",
        severity === "error" ? "bg-destructive" : "bg-warning"
      )}
    />
  )
}

function ProblemList({ problems }: { problems: Problem[] }) {
  if (problems.length === 0) {
    return (
      <div className="flex items-center gap-2 px-4 py-6 text-sm">
        <StatusPill tone="success" label="No findings" />
        <span className="text-muted-foreground">Every check passed on this app.</span>
      </div>
    )
  }
  return (
    <div>
      {problems.map((problem, index) => (
        <div
          key={`${problem.code}-${index}`}
          className="flex items-start gap-2.5 border-b border-border/60 px-4 py-2.5 last:border-0"
        >
          <SeverityMark severity={problem.severity} />
          <div className="min-w-0 flex-1">
            <div className="flex flex-wrap items-center gap-1.5">
              <span className="font-mono text-[10px] text-subtle-foreground">{problem.code}</span>
              <span className="text-2xs text-subtle-foreground">· {problem.severity}</span>
            </div>
            <p className="mt-0.5 text-pretty text-xs leading-relaxed text-muted-foreground">
              {problem.message}
            </p>
          </div>
        </div>
      ))}
    </div>
  )
}

function InspectBody({ data }: { data: InspectResult | undefined }) {
  if (!data) {
    return (
      <div className="space-y-3">
        <Skeleton className="h-6 w-48" />
        <Skeleton className="h-24 w-full" />
      </div>
    )
  }
  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center gap-2">
        <StatusPill
          tone={data.live.up ? "success" : "danger"}
          label={data.live.up ? `Live ${data.live.code ?? ""}` : "Not answering"}
        />
        <Badge variant="outline" className="font-mono">
          release {data.live.release?.slice(0, 10) ?? "—"}
        </Badge>
        <Badge variant="outline" className="font-mono">
          previous {data.live.old_release?.slice(0, 10) ?? "—"}
        </Badge>
        <Badge variant="outline" className="font-mono">
          port {data.live.live_port ?? "—"}
        </Badge>
      </div>

      {data.container ? (
        <div className="space-y-3">
          <div className="rounded-lg border border-border bg-muted/20 p-3">
            <MetaList
              columns={2}
              items={[
                { label: "Container", value: data.container.name, mono: true },
                { label: "Image", value: data.container.image ?? "—", mono: true },
                { label: "Health", value: data.container.health ?? "—", mono: true },
                { label: "Mounts", value: String(data.container.mounts?.length ?? 0) },
              ]}
            />
          </div>

          {data.container.mounts && data.container.mounts.length > 0 ? (
            <div className="overflow-hidden rounded-lg border border-border">
              <div className="border-b border-border/60 bg-muted/20 px-3 py-1.5">
                <span className="label-eyebrow">Mounts</span>
              </div>
              {data.container.mounts.map((mount, index) => (
                <div
                  key={index}
                  className="flex items-center gap-2 border-b border-border/60 px-3 py-2 font-mono text-[10px] last:border-0"
                >
                  <Box className="size-3 shrink-0 text-subtle-foreground" />
                  <span className="min-w-0 flex-1 truncate text-muted-foreground">
                    {mount.Source ?? "?"} → {mount.Destination ?? "?"}
                  </span>
                  <span className="shrink-0 text-subtle-foreground">{mount.RW === false ? "ro" : "rw"}</span>
                </div>
              ))}
            </div>
          ) : null}
        </div>
      ) : (
        <p className="text-xs text-muted-foreground">
          No container publishes this app's port. A static app serves from its root.
        </p>
      )}

      <details>
        <summary className="cursor-pointer list-none text-2xs text-subtle-foreground hover:text-muted-foreground">
          Raw inspect JSON
        </summary>
        <pre className="code-surface mt-2 max-h-64 overflow-auto rounded-lg border border-border p-3 font-mono text-[10px]">
          {JSON.stringify(data, null, 2)}
        </pre>
      </details>
    </div>
  )
}

/**
 * One-off operator actions: diagnostics, snapshots, probes, pruning and
 * first-run adoption. Deploys and rollbacks stay in the header and pipeline.
 */
export function Operations({ app }: { app: App }) {
  const queryClient = useQueryClient()
  const [lastRun, setLastRun] = useState<string | null>(null)
  const [probeOpen, setProbeOpen] = useState(false)
  const [inspectOpen, setInspectOpen] = useState(false)
  const [pruneOpen, setPruneOpen] = useState(false)
  const [adoptOpen, setAdoptOpen] = useState(false)
  const [nginxOpen, setNginxOpen] = useState(false)
  const [probeUrl, setProbeUrl] = useState(app.public_url ?? "")
  const [probeResult, setProbeResult] = useState<ProbeResult | null>(null)
  const probeCheck = useMemo(() => probeTargetCheck(app, probeUrl), [app, probeUrl])
  const [keep, setKeep] = useState("3")

  const invalidate = () => {
    void queryClient.invalidateQueries({ queryKey: queryKeys.app(app.name) })
    void queryClient.invalidateQueries({ queryKey: queryKeys.apps })
    void queryClient.invalidateQueries({ queryKey: queryKeys.runs })
    void queryClient.invalidateQueries({ queryKey: queryKeys.appStatus(app.name) })
    void queryClient.invalidateQueries({ queryKey: queryKeys.appDoctor(app.name) })
    void queryClient.invalidateQueries({ queryKey: queryKeys.latest(app.name) })
  }

  const doctorMutation = useMutation({
    mutationFn: () => api.doctor(app.name),
    onError: (error) => toast.error("Doctor did not finish", { description: errorText(error) }),
  })

  const backupMutation = useMutation({
    mutationFn: () => api.backup(app.name),
    onSuccess: ({ run_id }) => {
      setLastRun(run_id)
      invalidate()
      toast.success("Backup started", { description: `run ${shortId(run_id)}` })
    },
    onError: (error) => toast.error("Backup did not start", { description: errorText(error) }),
  })

  const probeMutation = useMutation({
    mutationFn: () => api.probe(app.name, probeUrl.trim() === "" ? undefined : probeUrl.trim()),
    onSuccess: (result) => {
      setProbeResult(result)
      const ok = result.status >= 200 && result.status < 400
      toast[ok ? "success" : "error"](`Probe returned HTTP ${result.status}`, {
        description: result.url,
      })
    },
    onError: (error) => toast.error("Probe failed", { description: errorText(error) }),
  })

  const pruneMutation = useMutation({
    mutationFn: () => api.prune(app.name, Math.max(0, Number(keep) || 0)),
    onSuccess: ({ run_id }) => {
      setLastRun(run_id)
      setPruneOpen(false)
      invalidate()
      toast.success("Prune started", { description: `run ${shortId(run_id)}` })
    },
    onError: (error) => toast.error("Prune did not start", { description: errorText(error) }),
  })

  const inspectQuery = useQuery({
    queryKey: queryKeys.inspect(app.name),
    queryFn: ({ signal }) => api.inspect(app.name, signal),
    enabled: inspectOpen,
  })

  const nginxQuery = useQuery({
    queryKey: queryKeys.render(app.name),
    queryFn: () => api.render(app.name),
    enabled: nginxOpen,
    retry: 0,
  })

  const doctorProblems = doctorMutation.data?.problems
  const needsAdoption = app.live_port == null

  return (
    <div className="space-y-4">
      <SectionHeading
        title="Operator actions"
        description="Read-only checks, snapshots, probes and maintenance. Deploys and rollbacks stay in Deployment."
      />

      {lastRun ? (
        <Callout
          tone="info"
          title="Operation started"
          action={
            <Button
              variant="outline"
              size="xs"
              render={<Link to="/runs/$runId" params={{ runId: lastRun }} />}
            >
              View run
              <ArrowRight />
            </Button>
          }
        >
          <span className="font-mono">{shortId(lastRun)}</span> is streaming to the run log.
        </Callout>
      ) : null}

      <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
        {needsAdoption ? (
          <ActionCard
            icon={PackagePlus}
            title="Adopt a legacy container"
            description="This app is not behind nginx yet. Take over the container already publishing the front port; no commit hash is needed."
            className="border-accent/30 md:col-span-2 xl:col-span-3"
            badge={<Badge variant="accent">recommended</Badge>}
            action={
              <Button size="sm" onClick={() => setAdoptOpen(true)}>
                <PackagePlus />
                Take over app
              </Button>
            }
            footer="One-time cutover: nginx takes the front port, the container moves to a managed backend port."
          />
        ) : null}

        <ActionCard
          icon={Stethoscope}
          title="Doctor"
          description="Check compose paths, bind mounts, ports, images and healthchecks for this app."
          badge={
            doctorProblems ? (
              <Badge variant={doctorProblems.some((p) => p.severity === "error") ? "destructive" : "warning"}>
                {plural(doctorProblems.length, "finding")}
              </Badge>
            ) : undefined
          }
          action={
            <Button
              variant="outline"
              size="sm"
              disabled={doctorMutation.isPending}
              onClick={() => doctorMutation.mutate()}
            >
              <Stethoscope className={cn(doctorMutation.isPending && "animate-pulse")} />
              {doctorMutation.isPending ? "Checking…" : "Run check"}
            </Button>
          }
          footer="Read-only. Nothing here changes the host."
        />

        <ActionCard
          icon={ScanSearch}
          title="Inspect"
          description="See the live verdict, container image and the mounts Docker really has, not what the registry hopes."
          action={
            <Button variant="outline" size="sm" onClick={() => setInspectOpen(true)}>
              <ScanSearch />
              Open inspector
            </Button>
          }
        />

        <ActionCard
          icon={Globe}
          title="Probe public URL"
          description="Make one HTTP request to the app's public URL and report the status. The API only probes this app's hostnames or public_url (plus its server allowlist)."
          action={
            <Button
              variant="outline"
              size="sm"
              onClick={() => {
                setProbeResult(null)
                setProbeOpen(true)
              }}
            >
              <Globe />
              Probe URL
            </Button>
          }
          footer={app.public_url ? <span className="font-mono">{app.public_url}</span> : "No public_url configured"}
        />

        <ActionCard
          icon={HardDrive}
          title="Backup"
          description="Run the app's configured [backup] command once, as a deploy run with a trace."
          badge={app.backup ? <Badge variant="outline">configured</Badge> : <Badge variant="warning">not configured</Badge>}
          action={
            <Button
              variant="outline"
              size="sm"
              disabled={!app.backup || backupMutation.isPending}
              title={app.backup ? "Run the configured backup command" : "Add a [backup] block in Configuration"}
              onClick={() => backupMutation.mutate()}
            >
              <HardDrive />
              {backupMutation.isPending ? "Backing up…" : "Back up now"}
            </Button>
          }
          footer={
            app.backup
              ? `${app.backup.command[0] ?? "command"}${app.backup.dir ? ` → ${app.backup.dir}` : ""}`
              : "Configure the command in the Configuration tab."
          }
        />

        <ActionCard
          icon={Trash}
          title="Prune images"
          description="Remove old local release tags while keeping the live release, the rollback release and recent tags."
          action={
            <Button variant="outline" size="sm" onClick={() => setPruneOpen(true)}>
              <Trash />
              Prune images
            </Button>
          }
          footer="Destructive: pruned tags cannot be deployed again without a rebuild."
        />

        <ActionCard
          icon={Code2}
          title="Generated nginx"
          description="Read the exact server block swapdock derives for this app. Copy it for review; the registry stays the source of truth."
          action={
            <Button variant="outline" size="sm" onClick={() => setNginxOpen(true)}>
              <Code2 />
              Preview config
            </Button>
          }
          footer="Read-only preview. apply stays part of a deploy."
        />
      </div>

      {doctorMutation.isError ? (
        <Callout tone="danger" title="Doctor could not finish">
          {errorText(doctorMutation.error)}
        </Callout>
      ) : null}

      {doctorProblems ? (
        <section className="overflow-hidden rounded-xl border border-border bg-card shadow-[var(--shadow-card)]">
          <div className="flex items-start justify-between gap-3 border-b border-border/60 px-4 py-3.5">
            <div className="flex items-start gap-2.5">
              <Stethoscope className="mt-0.5 size-4 text-muted-foreground" />
              <div>
                <h2 className="text-sm font-medium">Doctor result</h2>
                <p className="text-2xs text-muted-foreground">
                  {doctorProblems.length === 0
                    ? "No host findings for this app."
                    : `${plural(doctorProblems.length, "finding")} on this app's host surface.`}
                </p>
              </div>
            </div>
            <Button variant="ghost" size="xs" onClick={() => doctorMutation.reset()}>
              Dismiss
            </Button>
          </div>
          <ProblemList problems={doctorProblems} />
        </section>
      ) : null}

      {/* Probe ----------------------------------------------------------------- */}
      <Dialog open={probeOpen} onOpenChange={setProbeOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Probe {app.name}</DialogTitle>
            <DialogDescription>
              One HTTP request from the deploy host. Targets are restricted to this app’s
              hostnames or <span className="font-mono">public_url</span>; the API can also allow
              extra hosts through <span className="font-mono">SWAPDOCK_PROBE_ALLOWLIST</span>.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-1.5">
            <label htmlFor="probe-url" className="text-xs font-medium text-muted-foreground">
              URL
            </label>
            <Input
              id="probe-url"
              value={probeUrl}
              onChange={(event) => setProbeUrl(event.target.value)}
              placeholder="https://example.com/up"
              className="font-mono"
              autoComplete="off"
              spellCheck={false}
            />
            <p className="text-2xs text-muted-foreground">
              Leave empty to use the registry's public_url. Allowed hosts:{" "}
              {probeHosts(app).length > 0 ? (
                <span className="font-mono">{probeHosts(app).join(", ")}</span>
              ) : (
                "none configured"
              )}
              .
            </p>
            {probeCheck.invalid ? (
              <p role="alert" className="text-2xs text-destructive-strong">
                {probeCheck.warning}
              </p>
            ) : probeCheck.warning ? (
              <p className="text-2xs text-warning-strong">{probeCheck.warning}</p>
            ) : null}
          </div>
          {probeMutation.isError ? (
            <Callout tone="danger" title="The probe was refused">
              {errorText(probeMutation.error)}
            </Callout>
          ) : null}
          {probeResult ? (
            <div className="flex items-center gap-2 rounded-lg border border-border bg-muted/25 px-3 py-2">
              <StatusPill
                tone={
                  probeResult.status >= 200 && probeResult.status < 400
                    ? "success"
                    : probeResult.status === 0
                      ? "warning"
                      : "danger"
                }
                label={probeResult.status === 0 ? "No answer" : `HTTP ${probeResult.status}`}
              />
              <span className="truncate font-mono text-xs text-muted-foreground">
                {probeResult.url}
              </span>
            </div>
          ) : null}
          <DialogFooter>
            <Button variant="outline" onClick={() => setProbeOpen(false)}>
              Close
            </Button>
            <Button
              disabled={probeMutation.isPending || probeCheck.invalid}
              onClick={() => probeMutation.mutate()}
            >
              {probeMutation.isPending ? "Probing…" : "Probe"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Inspect --------------------------------------------------------------- */}
      <Dialog open={inspectOpen} onOpenChange={setInspectOpen}>
        <DialogContent className="sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle>Inspect {app.name}</DialogTitle>
            <DialogDescription>
              Registry entry, live verdict and the container's real mounts.
            </DialogDescription>
          </DialogHeader>
          {inspectQuery.isError ? (
            <Callout tone="danger" title="Inspect failed">
              {errorText(inspectQuery.error)}
            </Callout>
          ) : (
            <InspectBody data={inspectQuery.data} />
          )}
          <DialogFooter>
            <Button variant="outline" onClick={() => setInspectOpen(false)}>
              Close
            </Button>
            <Button
              variant="outline"
              disabled={inspectQuery.isFetching}
              onClick={() => void inspectQuery.refetch()}
            >
              <RefreshCw className={inspectQuery.isFetching ? "animate-spin" : undefined} />
              Refresh
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Prune ----------------------------------------------------------------- */}
      <Dialog open={pruneOpen} onOpenChange={setPruneOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Prune images for {app.name}</DialogTitle>
            <DialogDescription>
              Removes old local release tags. The current release, previous release,{" "}
              <span className="font-mono">latest</span>, unknown tags and the newest rollback
              tags are kept.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-2">
            <label htmlFor="prune-keep" className="text-xs font-medium text-muted-foreground">
              Rollback tags to keep
            </label>
            <Input
              id="prune-keep"
              type="number"
              min={0}
              value={keep}
              onChange={(event) => setKeep(event.target.value)}
              className="w-32"
            />
            <Callout tone="warning" title="This cannot be undone">
              A pruned image can only be deployed again after it is rebuilt.
            </Callout>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setPruneOpen(false)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              disabled={pruneMutation.isPending}
              onClick={() => pruneMutation.mutate()}
            >
              <Trash />
              {pruneMutation.isPending ? "Pruning…" : "Prune"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Generated nginx ------------------------------------------------------- */}
      <Dialog open={nginxOpen} onOpenChange={setNginxOpen}>
        <DialogContent className="sm:max-w-3xl">
          <DialogHeader>
            <DialogTitle>Generated nginx config for {app.name}</DialogTitle>
            <DialogDescription>
              Derived from the registry and never hand-edited. The live file is only replaced
              by a deploy after <span className="font-mono">nginx -t</span> passes.
            </DialogDescription>
          </DialogHeader>
          {nginxQuery.isPending ? (
            <div className="space-y-2">
              <Skeleton className="h-4 w-full" />
              <Skeleton className="h-4 w-5/6" />
              <Skeleton className="h-4 w-2/3" />
            </div>
          ) : nginxQuery.isError ? (
            <Callout tone="danger" title="The config could not be rendered">
              {errorText(nginxQuery.error)}
            </Callout>
          ) : (
            <pre className="code-surface max-h-[52vh] overflow-auto rounded-lg border border-border p-3 font-mono text-[11px] leading-relaxed">
              {nginxQuery.data}
            </pre>
          )}
          <DialogFooter>
            {nginxQuery.data ? (
              <CopyButton
                value={nginxQuery.data}
                label="Copy config"
                size="icon-sm"
                variant="outline"
              />
            ) : null}
            <Button variant="outline" onClick={() => setNginxOpen(false)}>
              Close
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Adoption is the one-time on-ramp for a legacy container. */}
      <AdoptDialog
        app={app}
        open={adoptOpen}
        onOpenChange={setAdoptOpen}
        onStarted={setLastRun}
      />
    </div>
  )
}
