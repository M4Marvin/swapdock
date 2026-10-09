import { createFileRoute, Link } from '@tanstack/react-router'
import { useEffect, useRef, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useForm } from '@tanstack/react-form'
import { toast } from 'sonner'
import { z } from 'zod'
import {
  api,
  type App,
  type AppLatest,
} from '../../api'
import { queryKeys } from '@/lib/query-keys'
import { Pipeline } from '@/components/Pipeline'
import { RegistryEditor } from '@/components/RegistryEditor'
import { StatusPill } from '@/components/StatusPill'
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
import {
  Field,
  FieldError,
  FieldLabel,
} from '@/components/ui/field'

export const Route = createFileRoute('/apps/$appName')({
  component: AppDetailRoute,
})

/**
 * Remounts `AppDetail` when the app name changes.
 *
 * Verify input, the committed verify window, and the last-run link are local
 * state; without a keyed remount they would leak from one app to the next.
 */
function AppDetailRoute() {
  const { appName } = Route.useParams()
  return <AppDetail key={appName} />
}

function formatMs(ms: number): string {
  const d = new Date(ms)
  return Number.isNaN(d.getTime()) ? String(ms) : d.toLocaleString()
}

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e)
}

/** A release is a commit SHA: optional prefix digits then 4-64 hex chars. */
const RELEASE_RE = /^\d*[0-9a-fA-F]{4,64}$/

const deploySchema = z.object({
  release: z
    .string()
    .refine(
      (v) => v === '' || RELEASE_RE.test(v),
      'must be a commit SHA (hex, 4-64 chars), or empty for the latest',
    ),
})

/** Shows an "update available" chip when the upstream ref moved ahead. */
function LatestChip({
  release,
  latest,
}: {
  release: string | null
  latest: AppLatest | undefined
}) {
  const remote = latest?.release
  if (!remote || (release != null && release === remote)) return null
  return (
    <Badge variant="secondary" title={latest?.source}>
      update available: {remote}
    </Badge>
  )
}

/**
 * Deploy one release. The release field is validated by the same zod schema
 * the pipeline uses; an empty value deploys whatever the app resolves to.
 */
function DeployDialog({
  app,
  open,
  onOpenChange,
}: {
  app: App
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const queryClient = useQueryClient()

  const deployMutation = useMutation({
    mutationFn: (release: string) =>
      api.deploy(app.name, release.trim() || undefined),
    onSuccess: ({ run_id }) => {
      toast.success(`deploy started: ${run_id}`)
      queryClient.invalidateQueries({ queryKey: queryKeys.app(app.name) })
      queryClient.invalidateQueries({ queryKey: queryKeys.apps })
      queryClient.invalidateQueries({ queryKey: queryKeys.runs })
      queryClient.invalidateQueries({ queryKey: queryKeys.latest(app.name) })
      queryClient.invalidateQueries({ queryKey: queryKeys.appStatus(app.name) })
    },
    onError: (error) => {
      toast.error('deploy failed', { description: errorText(error) })
    },
  })

  const form = useForm({
    defaultValues: { release: app.release ?? '' },
    validators: { onChange: deploySchema, onSubmit: deploySchema },
    onSubmit: async ({ value }) => {
      try {
        await deployMutation.mutateAsync(value.release)
        onOpenChange(false)
      } catch {
        // Surfaced from deployMutation.error next to the field.
      }
    },
  })

  // Start each open from the app's current release and clear stale errors.
  const wasOpen = useRef(false)
  useEffect(() => {
    if (open && !wasOpen.current) {
      form.reset({ release: app.release ?? '' })
      deployMutation.reset()
    }
    wasOpen.current = open
  }, [open, app.release, form, deployMutation])

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Deploy {app.name}</DialogTitle>
          <DialogDescription>
            Start a deploy run for a release. Leave the field empty to deploy
            the app's current upstream ref.
          </DialogDescription>
        </DialogHeader>
        <form
          id="deploy-form"
          onSubmit={(e) => {
            e.preventDefault()
            form.handleSubmit()
          }}
        >
          <form.Field name="release">
            {(field) => {
              const isInvalid =
                field.state.meta.isTouched && !field.state.meta.isValid
              return (
                <Field data-invalid={isInvalid}>
                  <FieldLabel htmlFor={field.name}>release</FieldLabel>
                  <Input
                    id={field.name}
                    name={field.name}
                    value={field.state.value}
                    onBlur={field.handleBlur}
                    onChange={(e) => field.handleChange(e.target.value)}
                    aria-invalid={isInvalid}
                    placeholder="commit sha (or empty)"
                    className="font-mono"
                    autoComplete="off"
                  />
                  {isInvalid && <FieldError errors={field.state.meta.errors} />}
                </Field>
              )
            }}
          </form.Field>
        </form>

        {deployMutation.isError && (
          <FieldError>{errorText(deployMutation.error)}</FieldError>
        )}

        <DialogFooter>
          <Button
            type="button"
            variant="outline"
            onClick={() => onOpenChange(false)}
          >
            Cancel
          </Button>
          <Button
            type="submit"
            form="deploy-form"
            disabled={deployMutation.isPending}
          >
            {deployMutation.isPending ? 'Deploying…' : 'Deploy'}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function AppDetail() {
  const { appName } = Route.useParams()
  const queryClient = useQueryClient()
  const [since, setSince] = useState('')
  const [deployOpen, setDeployOpen] = useState(false)
  const [confirmRollback, setConfirmRollback] = useState(false)
  const [lastRun, setLastRun] = useState<string | null>(null)
  // The window the verify query is keyed on; only committed by the Check
  // button so typing does not wipe the results already on screen.
  const [querySince, setQuerySince] = useState('')

  const appQuery = useQuery({
    queryKey: queryKeys.app(appName),
    queryFn: ({ signal }) => api.app(appName, signal),
  })

  const latestQuery = useQuery({
    queryKey: queryKeys.latest(appName),
    queryFn: ({ signal }) => api.appLatest(appName, signal),
    enabled: appQuery.data?.app.live_port != null,
  })

  // Verify is operator-triggered; the query stays idle until a since value is
  // committed.
  const verifyQuery = useQuery({
    queryKey: queryKeys.verify(appName, querySince),
    queryFn: ({ signal }) => api.verify(appName, querySince, signal),
    enabled: querySince !== '',
    retry: false,
  })

  const checkVerify = () => {
    if (querySince === since) void verifyQuery.refetch()
    else setQuerySince(since)
  }

  const rollbackMutation = useMutation({
    mutationFn: () => api.rollback(appName),
    onSuccess: ({ run_id }) => {
      setLastRun(run_id)
      queryClient.invalidateQueries({ queryKey: queryKeys.app(appName) })
      queryClient.invalidateQueries({ queryKey: queryKeys.apps })
      queryClient.invalidateQueries({ queryKey: queryKeys.runs })
      queryClient.invalidateQueries({ queryKey: queryKeys.latest(appName) })
      toast.success(`rollback started: ${run_id}`)
    },
    onError: (error) => {
      toast.error('rollback failed', { description: errorText(error) })
    },
  })

  const syncMutation = useMutation({
    mutationFn: () => api.sync(appName),
    onSuccess: ({ run_id }) => {
      setLastRun(run_id)
      queryClient.invalidateQueries({ queryKey: queryKeys.app(appName) })
      queryClient.invalidateQueries({ queryKey: queryKeys.apps })
      queryClient.invalidateQueries({ queryKey: queryKeys.runs })
      queryClient.invalidateQueries({ queryKey: queryKeys.latest(appName) })
      toast.success(`sync started: ${run_id}`)
    },
    onError: (error) => {
      toast.error('sync failed', { description: errorText(error) })
    },
  })

  const busy = rollbackMutation.isPending || syncMutation.isPending
  const actionError = rollbackMutation.error ?? syncMutation.error
  const verify = verifyQuery.data

  if (appQuery.isError && !appQuery.data) {
    return (
      <div
        role="alert"
        className="flex flex-wrap items-center gap-3 rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive"
      >
        <span>Could not load {appName}: {errorText(appQuery.error)}</span>
        <Button
          size="sm"
          variant="outline"
          onClick={() => void appQuery.refetch()}
        >
          Retry
        </Button>
        <Link to="/" className="text-sm underline-offset-4 hover:underline">
          Back to apps
        </Link>
      </div>
    )
  }
  if (!appQuery.data) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-8 w-64" />
        <Skeleton className="h-40 w-full" />
        <Skeleton className="h-64 w-full" />
      </div>
    )
  }

  const { app, problems } = appQuery.data

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="text-xl font-semibold">{app.name}</h1>
        <StatusPill name={app.name} livePort={app.live_port} showCode />
        <Badge variant="secondary">{app.kind}</Badge>
        <LatestChip release={app.release} latest={latestQuery.data} />
        <span className="flex-1" />
        <Button onClick={() => setDeployOpen(true)}>Deploy…</Button>
      </div>

      <Pipeline
        app={app}
        onDeployed={() => {
          queryClient.invalidateQueries({ queryKey: queryKeys.app(appName) })
          queryClient.invalidateQueries({ queryKey: queryKeys.apps })
          queryClient.invalidateQueries({
            queryKey: queryKeys.appStatus(appName),
          })
        }}
      />

      <RegistryEditor key={app.name} app={app} />

      {problems.length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle>Problems</CardTitle>
          </CardHeader>
          <CardContent className="space-y-1 text-sm">
            {problems.map((p, i) => (
              <p key={i}>
                <Badge
                  variant={p.severity === 'error' ? 'destructive' : 'outline'}
                >
                  {p.severity}
                </Badge>{' '}
                <span className="font-mono text-xs">{p.code}</span> — {p.message}
              </p>
            ))}
          </CardContent>
        </Card>
      )}

      <Card>
        <CardHeader>
          <CardTitle>Rollback / sync</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-wrap items-start gap-2">
          <Button
            variant="outline"
            disabled={busy}
            onClick={() => setConfirmRollback(true)}
          >
            {rollbackMutation.isPending ? 'Rolling back…' : 'Rollback'}
          </Button>
          <Button
            variant="outline"
            disabled={busy}
            onClick={() => syncMutation.mutate()}
          >
            {syncMutation.isPending ? 'Syncing…' : 'Sync'}
          </Button>
          {lastRun && (
            <Link
              to="/runs/$runId"
              params={{ runId: lastRun }}
              className="text-sm underline-offset-4 hover:underline"
            >
              run <span className="font-mono">{lastRun.slice(0, 12)}</span>
            </Link>
          )}
          {actionError && (
            <p role="alert" className="w-full text-sm text-destructive">
              {errorText(actionError)}
            </p>
          )}
        </CardContent>
      </Card>

      <Dialog open={confirmRollback} onOpenChange={setConfirmRollback}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Roll back {app.name}?</DialogTitle>
            <DialogDescription>
              This redeploys the app's previous release on production. It
              cannot be undone from here; run another deploy to move forward
              again.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => setConfirmRollback(false)}
            >
              Cancel
            </Button>
            <Button
              type="button"
              onClick={() => {
                setConfirmRollback(false)
                rollbackMutation.mutate()
              }}
            >
              Roll back
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Card>
        <CardHeader>
          <CardTitle>Verify access log</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex flex-wrap gap-2">
            <Label htmlFor="verify-since" className="sr-only">
              Since
            </Label>
            <Input
              id="verify-since"
              placeholder="since: RFC3339 timestamp or run id"
              value={since}
              onChange={(e) => setSince(e.target.value)}
              className="w-80"
            />
            <Button
              variant="outline"
              disabled={!since || verifyQuery.isFetching}
              onClick={checkVerify}
            >
              {verifyQuery.isFetching ? 'Checking…' : 'Check'}
            </Button>
          </div>

          {verifyQuery.isError && (
            <p role="alert" className="text-destructive">
              {errorText(verifyQuery.error)}
            </p>
          )}

          {verify && (
            <div className="space-y-4">
              <div className="flex flex-wrap gap-2">
                <Badge variant="outline">{verify.requests} requests</Badge>
                <Badge
                  variant={verify.errors_5xx > 0 ? 'destructive' : 'outline'}
                >
                  {verify.errors_5xx} errors (5xx)
                </Badge>
                <Badge
                  variant={verify.malformed > 0 ? 'secondary' : 'outline'}
                >
                  {verify.malformed} malformed
                </Badge>
                <Badge variant="outline">{verify.lines} lines</Badge>
              </div>

              {Object.keys(verify.by_status).length > 0 && (
                <div className="space-y-1">
                  <p className="text-xs font-medium tracking-wide uppercase text-muted-foreground">
                    By status
                  </p>
                  <div className="flex flex-wrap gap-2">
                    {Object.entries(verify.by_status)
                      .sort(([a], [b]) => Number(a) - Number(b))
                      .map(([code, count]) => (
                        <Badge
                          key={code}
                          variant={code.startsWith('5') ? 'destructive' : 'outline'}
                        >
                          {code}: {count}
                        </Badge>
                      ))}
                  </div>
                </div>
              )}

              {Object.keys(verify.upstreams).length > 0 && (
                <div className="space-y-1">
                  <p className="text-xs font-medium tracking-wide uppercase text-muted-foreground">
                    Upstreams
                  </p>
                  <ul className="space-y-1 text-sm">
                    {Object.entries(verify.upstreams).map(([up, info]) => (
                      <li key={up} className="flex flex-wrap items-center gap-2">
                        <span className="font-mono">{up}</span>
                        <Badge variant="outline">{info.requests} req</Badge>
                        {info.errors_5xx > 0 && (
                          <Badge variant="destructive">
                            {info.errors_5xx} 5xx
                          </Badge>
                        )}
                        {Number.isFinite(info.first_ms) &&
                          Number.isFinite(info.last_ms) && (
                            <span className="text-xs text-muted-foreground">
                              first {formatMs(info.first_ms)} · last{' '}
                              {formatMs(info.last_ms)}
                            </span>
                          )}
                      </li>
                    ))}
                  </ul>
                </div>
              )}

              {verify.flip && (
                <p className="text-sm">
                  <span className="text-muted-foreground">flip at </span>
                  {formatMs(verify.flip.at_ms)}
                  <span className="text-muted-foreground"> from </span>
                  <span className="font-mono">{verify.flip.from}</span>
                  <span className="text-muted-foreground"> → </span>
                  <span className="font-mono">{verify.flip.to}</span>
                </p>
              )}

              <details className="text-xs text-muted-foreground">
                <summary className="cursor-pointer select-none">
                  Raw JSON
                </summary>
                <pre className="mt-2 overflow-x-auto rounded-lg border bg-muted/40 p-3 font-mono">
                  {JSON.stringify(verify, null, 2)}
                </pre>
              </details>
            </div>
          )}
        </CardContent>
      </Card>

      <DeployDialog
        app={app}
        open={deployOpen}
        onOpenChange={setDeployOpen}
      />
    </div>
  )
}
