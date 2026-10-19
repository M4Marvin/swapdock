import { Link } from '@tanstack/react-router'
import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import {
  api,
  type App,
  type InspectResult,
  type Problem,
  type ProbeResult,
} from '../api'
import { queryKeys } from '@/lib/query-keys'
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
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  Field,
  FieldDescription,
  FieldError,
  FieldLabel,
} from '@/components/ui/field'

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e)
}

const RELEASE_RE = /^[0-9a-fA-F]{4,64}$/

function ProblemList({ problems }: { problems: Problem[] }) {
  if (problems.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        No problems found on the host.
      </p>
    )
  }
  return (
    <div className="space-y-1 text-sm">
      {problems.map((p, i) => (
        <p key={`${p.code}-${p.app ?? ''}-${i}`}>
          <Badge variant={p.severity === 'error' ? 'destructive' : 'outline'}>
            {p.severity}
          </Badge>{' '}
          <span className="font-mono text-xs">{p.code}</span>
          {p.app ? (
            <span className="text-muted-foreground"> ({p.app})</span>
          ) : null}
          {' — '}
          {p.message}
        </p>
      ))}
    </div>
  )
}

function InspectView({ data }: { data: InspectResult | undefined }) {
  if (!data) return <Skeleton className="h-40 w-full" />
  return (
    <div className="space-y-4">
      <div className="flex flex-wrap gap-2">
        <Badge variant={data.live.up ? 'success' : 'destructive'}>
          {data.live.up ? `up ${data.live.code ?? ''}` : 'down'}
        </Badge>
        <Badge variant="outline">
          release {data.live.release ?? '—'}
        </Badge>
        <Badge variant="outline">
          previous {data.live.old_release ?? '—'}
        </Badge>
        <Badge variant="outline">
          port {data.live.live_port ?? '—'}
        </Badge>
      </div>
      {data.container ? (
        <div className="space-y-2 text-sm">
          <p>
            <span className="text-muted-foreground">container </span>
            <span className="font-mono">{data.container.name}</span>
            {data.container.image ? (
              <>
                <span className="text-muted-foreground"> image </span>
                <span className="font-mono">{data.container.image}</span>
              </>
            ) : null}
            {data.container.health ? (
              <>
                <span className="text-muted-foreground"> health </span>
                <span className="font-mono">{data.container.health}</span>
              </>
            ) : null}
          </p>
          {data.container.mounts && data.container.mounts.length > 0 && (
            <div className="space-y-1">
              <p className="text-xs font-medium tracking-wide uppercase text-muted-foreground">
                mounts
              </p>
              <ul className="space-y-1">
                {data.container.mounts.map((m, i) => (
                  <li key={i} className="font-mono text-xs">
                    {m.Source ?? '?'} → {m.Destination ?? '?'}
                    {m.RW === false ? ' (ro)' : ''}
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>
      ) : (
        <p className="text-sm text-muted-foreground">
          No container publishes this app's port.
        </p>
      )}
      <details className="text-xs">
        <summary className="cursor-pointer select-none text-muted-foreground">
          Raw JSON
        </summary>
        <pre className="mt-2 max-h-80 overflow-auto rounded-lg border bg-muted/40 p-3 font-mono">
          {JSON.stringify(data, null, 2)}
        </pre>
      </details>
    </div>
  )
}

/**
 * Operator actions that are not part of the deploy pipeline: diagnostics,
 * snapshots, public probes, image pruning and first-run adoption.
 */
export function Operations({ app }: { app: App }) {
  const queryClient = useQueryClient()
  const [lastRun, setLastRun] = useState<string | null>(null)
  const [probeOpen, setProbeOpen] = useState(false)
  const [inspectOpen, setInspectOpen] = useState(false)
  const [pruneOpen, setPruneOpen] = useState(false)
  const [adoptOpen, setAdoptOpen] = useState(false)
  const [probeUrl, setProbeUrl] = useState(app.public_url ?? '')
  const [probeResult, setProbeResult] = useState<ProbeResult | null>(null)
  const [keep, setKeep] = useState('3')
  const [adoptRelease, setAdoptRelease] = useState(app.release ?? '')
  const [healthcheck, setHealthcheck] = useState('')
  const [containerPort, setContainerPort] = useState('')

  const invalidate = () => {
    queryClient.invalidateQueries({ queryKey: queryKeys.app(app.name) })
    queryClient.invalidateQueries({ queryKey: queryKeys.apps })
    queryClient.invalidateQueries({ queryKey: queryKeys.runs })
    queryClient.invalidateQueries({ queryKey: queryKeys.appStatus(app.name) })
    queryClient.invalidateQueries({ queryKey: queryKeys.appDoctor(app.name) })
  }

  const doctorMutation = useMutation({
    mutationFn: () => api.doctor(app.name),
    onError: (e) =>
      toast.error('doctor failed', { description: errorText(e) }),
  })

  const backupMutation = useMutation({
    mutationFn: () => api.backup(app.name),
    onSuccess: ({ run_id }) => {
      setLastRun(run_id)
      invalidate()
      toast.success(`backup started: ${run_id}`)
    },
    onError: (e) =>
      toast.error('backup failed', { description: errorText(e) }),
  })

  const probeMutation = useMutation({
    mutationFn: () =>
      api.probe(app.name, probeUrl.trim() === '' ? undefined : probeUrl.trim()),
    onSuccess: (result) => {
      setProbeResult(result)
      const ok = result.status >= 200 && result.status < 400
      toast[ok ? 'success' : 'error'](`probe returned ${result.status}`)
    },
    onError: (e) =>
      toast.error('probe failed', { description: errorText(e) }),
  })

  const pruneMutation = useMutation({
    mutationFn: () => api.prune(app.name, Math.max(0, Number(keep) || 0)),
    onSuccess: ({ run_id }) => {
      setLastRun(run_id)
      setPruneOpen(false)
      invalidate()
      toast.success(`prune started: ${run_id}`)
    },
    onError: (e) =>
      toast.error('prune failed', { description: errorText(e) }),
  })

  const adoptMutation = useMutation({
    mutationFn: () => {
      const port = containerPort.trim() === '' ? undefined : Number(containerPort)
      return api.adopt(app.name, {
        release: adoptRelease.trim(),
        healthcheck: healthcheck.trim() === '' ? undefined : healthcheck.trim(),
        container_port:
          port != null && Number.isInteger(port) ? port : undefined,
      })
    },
    onSuccess: ({ run_id }) => {
      setLastRun(run_id)
      setAdoptOpen(false)
      invalidate()
      toast.success(`adopt started: ${run_id}`)
    },
    onError: (e) =>
      toast.error('adopt failed', { description: errorText(e) }),
  })

  const inspectQuery = useQuery({
    queryKey: queryKeys.inspect(app.name),
    queryFn: ({ signal }) => api.inspect(app.name, signal),
    enabled: inspectOpen,
  })

  const adoptReleaseValid =
    adoptRelease.trim() !== '' && RELEASE_RE.test(adoptRelease.trim())

  return (
    <Card>
      <CardHeader>
        <CardTitle>Operations</CardTitle>
        <p className="text-sm text-muted-foreground">
          Host checks, snapshots, public probes, image pruning, adoption and a
          full container/mount view. Deploys and rollbacks stay in the pipeline
          above.
        </p>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex flex-wrap items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            disabled={doctorMutation.isPending}
            onClick={() => doctorMutation.mutate()}
          >
            {doctorMutation.isPending ? 'Checking…' : 'Doctor'}
          </Button>
          <Button
            variant="outline"
            size="sm"
            disabled={backupMutation.isPending || !app.backup}
            title={
              app.backup
                ? 'Run the configured [backup] command'
                : 'Add a [backup] block in the registry editor'
            }
            onClick={() => backupMutation.mutate()}
          >
            {backupMutation.isPending ? 'Backing up…' : 'Backup'}
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              setProbeResult(null)
              setProbeOpen(true)
            }}
          >
            Probe
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => setPruneOpen(true)}
          >
            Prune
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => setInspectOpen(true)}
          >
            Inspect
          </Button>
          {app.live_port == null && (
            <Button size="sm" onClick={() => setAdoptOpen(true)}>
              Adopt…
            </Button>
          )}
          {lastRun && (
            <Link
              to="/runs/$runId"
              params={{ runId: lastRun }}
              className="text-sm underline-offset-4 hover:underline"
            >
              run <span className="font-mono">{lastRun.slice(0, 12)}</span>
            </Link>
          )}
        </div>

        {doctorMutation.data && (
          <div className="space-y-2 rounded-lg border bg-muted/30 p-3">
            <div className="flex items-center gap-2">
              <p className="text-sm font-medium">Doctor result</p>
              <Badge
                variant={
                  doctorMutation.data.problems.some(
                    (p) => p.severity === 'error',
                  )
                    ? 'destructive'
                    : 'outline'
                }
              >
                {doctorMutation.data.problems.filter(
                  (p) => p.severity === 'error',
                ).length}{' '}
                errors
              </Badge>
              <span className="flex-1" />
              <Button
                variant="ghost"
                size="sm"
                onClick={() => doctorMutation.reset()}
              >
                Dismiss
              </Button>
            </div>
            <ProblemList problems={doctorMutation.data.problems} />
          </div>
        )}

        {doctorMutation.isError && (
          <p role="alert" className="text-sm text-destructive">
            {errorText(doctorMutation.error)}
          </p>
        )}
      </CardContent>

      <Dialog open={probeOpen} onOpenChange={setProbeOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Probe {app.name}</DialogTitle>
            <DialogDescription>
              HTTP status from a public URL. Defaults to the registry's
              public_url.
            </DialogDescription>
          </DialogHeader>
          <Field>
            <FieldLabel htmlFor="probe-url">url</FieldLabel>
            <Input
              id="probe-url"
              value={probeUrl}
              onChange={(e) => setProbeUrl(e.target.value)}
              placeholder="https://example.com/up"
              className="font-mono"
              autoComplete="off"
            />
            <FieldDescription>
              Leave empty to use public_url; both http and https are allowed.
            </FieldDescription>
          </Field>
          {probeResult && (
            <p className="text-sm">
              <Badge
                variant={
                  probeResult.status >= 200 && probeResult.status < 400
                    ? 'success'
                    : 'destructive'
                }
              >
                HTTP {probeResult.status}
              </Badge>{' '}
              <span className="font-mono text-xs">{probeResult.url}</span>
            </p>
          )}
          <DialogFooter>
            <Button variant="outline" onClick={() => setProbeOpen(false)}>
              Close
            </Button>
            <Button
              onClick={() => probeMutation.mutate()}
              disabled={probeMutation.isPending}
            >
              {probeMutation.isPending ? 'Probing…' : 'Probe'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={inspectOpen} onOpenChange={setInspectOpen}>
        <DialogContent className="max-w-3xl">
          <DialogHeader>
            <DialogTitle>Inspect {app.name}</DialogTitle>
            <DialogDescription>
              Registry entry, live verdict and the container's actual mounts.
            </DialogDescription>
          </DialogHeader>
          {inspectQuery.isError ? (
            <p role="alert" className="text-sm text-destructive">
              {errorText(inspectQuery.error)}
            </p>
          ) : (
            <InspectView data={inspectQuery.data} />
          )}
          <DialogFooter>
            <Button variant="outline" onClick={() => setInspectOpen(false)}>
              Close
            </Button>
            <Button
              variant="outline"
              onClick={() => void inspectQuery.refetch()}
              disabled={inspectQuery.isFetching}
            >
              {inspectQuery.isFetching ? 'Refreshing…' : 'Refresh'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={pruneOpen} onOpenChange={setPruneOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Prune images for {app.name}</DialogTitle>
            <DialogDescription>
              Removes old local release tags. The current release, previous
              release, <span className="font-mono">latest</span>, unknown tags
              and the newest rollback tags are kept. This cannot be undone.
            </DialogDescription>
          </DialogHeader>
          <Field>
            <FieldLabel htmlFor="prune-keep">rollback tags to keep</FieldLabel>
            <Input
              id="prune-keep"
              type="number"
              min={0}
              value={keep}
              onChange={(e) => setKeep(e.target.value)}
            />
          </Field>
          <DialogFooter>
            <Button variant="outline" onClick={() => setPruneOpen(false)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={() => pruneMutation.mutate()}
              disabled={pruneMutation.isPending}
            >
              {pruneMutation.isPending ? 'Pruning…' : 'Prune'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={adoptOpen} onOpenChange={setAdoptOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Adopt {app.name}</DialogTitle>
            <DialogDescription>
              Takes over the container already publishing front port{' '}
              <span className="font-mono">{app.front_port}</span>. It tags that
              image as the release, stops the legacy container, writes a
              persistent compose override and puts nginx in front. Use the
              release the container is currently running.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-3">
            <Field>
              <FieldLabel htmlFor="adopt-release">release</FieldLabel>
              <Input
                id="adopt-release"
                value={adoptRelease}
                onChange={(e) => setAdoptRelease(e.target.value)}
                placeholder="current commit sha"
                className="font-mono"
                autoComplete="off"
              />
              <FieldDescription>
                Required: the commit already running in the legacy container.
              </FieldDescription>
              {adoptRelease.trim() !== '' && !adoptReleaseValid && (
                <FieldError>
                  must be a commit SHA (hex, 4-64 chars)
                </FieldError>
              )}
            </Field>
            <Field>
              <FieldLabel htmlFor="adopt-healthcheck">
                healthcheck (optional)
              </FieldLabel>
              <Input
                id="adopt-healthcheck"
                value={healthcheck}
                onChange={(e) => setHealthcheck(e.target.value)}
                placeholder="node -e &quot;fetch('http://127.0.0.1:3000/')…&quot;"
                className="font-mono"
                autoComplete="off"
              />
              <FieldDescription>
                Only needed when the compose service has no healthcheck.
              </FieldDescription>
            </Field>
            <Field>
              <FieldLabel htmlFor="adopt-port">
                container port (optional)
              </FieldLabel>
              <Input
                id="adopt-port"
                type="number"
                min={1}
                max={65535}
                value={containerPort}
                onChange={(e) => setContainerPort(e.target.value)}
                placeholder="inferred from the legacy container"
              />
            </Field>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setAdoptOpen(false)}>
              Cancel
            </Button>
            <Button
              onClick={() => adoptMutation.mutate()}
              disabled={adoptMutation.isPending || !adoptReleaseValid}
            >
              {adoptMutation.isPending ? 'Adopting…' : 'Adopt'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </Card>
  )
}
