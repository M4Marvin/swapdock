import { createFileRoute, Link } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import {
  api,
  type AppEntry,
  type AppStatus,
  type VerifyReport,
} from '../../api'
import { Pipeline } from '@/components/Pipeline'
import { RegistryEditor } from '@/components/RegistryEditor'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'

export const Route = createFileRoute('/apps/$appName')({
  component: AppDetail,
})

function formatMs(ms: number): string {
  const d = new Date(ms)
  return Number.isNaN(d.getTime()) ? String(ms) : d.toLocaleString()
}

function isUp(status: AppStatus): boolean {
  if (status.up === true) return true
  return (
    typeof status.status === 'number' &&
    status.status >= 200 &&
    status.status < 300
  )
}

function StatusPill({
  status,
  livePort,
}: {
  status: AppStatus | 'error' | null
  livePort: number | null
}) {
  // No live port means nothing was ever deployed: not the same as "down".
  if (livePort == null) {
    return (
      <Badge variant="outline" className="text-muted-foreground">
        not deployed
      </Badge>
    )
  }
  if (status === null || status === 'error') {
    return <Badge variant="outline">unknown</Badge>
  }
  const code = typeof status.status === 'number' ? ` ${status.status}` : ''
  return isUp(status) ? (
    <Badge
      variant="outline"
      className="border-emerald-500/40 bg-emerald-500/10 text-emerald-500"
    >
      up{code}
    </Badge>
  ) : (
    <Badge variant="destructive">down{code}</Badge>
  )
}

function AppDetail() {
  const { appName } = Route.useParams()
  const [entry, setEntry] = useState<AppEntry | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [status, setStatus] = useState<AppStatus | 'error' | null>(null)
  const [since, setSince] = useState('')
  const [verify, setVerify] = useState<VerifyReport | null>(null)
  const [verifyError, setVerifyError] = useState<string | null>(null)
  const [verifyBusy, setVerifyBusy] = useState(false)
  const [busy, setBusy] = useState(false)
  const [actionError, setActionError] = useState<string | null>(null)
  const [lastRun, setLastRun] = useState<string | null>(null)
  // Bumped after a successful deploy so the registry is re-read.
  const [nonce, setNonce] = useState(0)

  useEffect(() => {
    const controller = new AbortController()
    setError(null)
    setStatus(null)
    setVerify(null)
    setVerifyError(null)
    setActionError(null)
    api.app(appName, controller.signal).then(
      (e) => {
        if (controller.signal.aborted) return
        setEntry(e)
        // Only probe when there is a live port; otherwise the UI shows
        // "not deployed" without a request.
        if (e.app.live_port != null) {
          api.appStatus(appName, controller.signal).then(
            (s) => {
              if (!controller.signal.aborted) setStatus(s)
            },
            () => {
              if (!controller.signal.aborted) setStatus('error')
            },
          )
        }
      },
      (e) => {
        if (!controller.signal.aborted) setError(String(e))
      },
    )
    return () => controller.abort()
  }, [appName, nonce])

  const act = async (fn: () => Promise<{ run_id: string }>) => {
    setBusy(true)
    setActionError(null)
    try {
      const { run_id } = await fn()
      setLastRun(run_id)
    } catch (e) {
      setActionError(String(e))
    } finally {
      setBusy(false)
    }
  }

  const checkVerify = async () => {
    if (!since) return
    setVerifyBusy(true)
    setVerifyError(null)
    try {
      setVerify(await api.verify(appName, since))
    } catch (e) {
      setVerify(null)
      setVerifyError(String(e))
    } finally {
      setVerifyBusy(false)
    }
  }

  if (error && !entry) return <p className="text-destructive">{error}</p>
  if (!entry) return <p className="text-muted-foreground">Loading…</p>

  const { app, problems } = entry

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center gap-3">
        <h1 className="text-xl font-semibold">{app.name}</h1>
        <StatusPill status={status} livePort={app.live_port} />
        <Badge variant="secondary">{app.kind}</Badge>
      </div>

      <Pipeline app={app} onDeployed={() => setNonce((n) => n + 1)} />

      <RegistryEditor app={app} />

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
                <span className="font-mono">{p.code}</span> — {p.message}
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
            onClick={() => act(() => api.rollback(app.name))}
          >
            Rollback
          </Button>
          <Button
            variant="outline"
            disabled={busy}
            onClick={() => act(() => api.sync(app.name))}
          >
            Sync
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
            <p className="w-full text-sm text-destructive">{actionError}</p>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Verify access log</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex flex-wrap gap-2">
            <Input
              placeholder="since: RFC3339 timestamp or run id"
              value={since}
              onChange={(e) => setSince(e.target.value)}
              className="w-80"
            />
            <Button
              variant="outline"
              disabled={!since || verifyBusy}
              onClick={checkVerify}
            >
              {verifyBusy ? 'Checking…' : 'Check'}
            </Button>
          </div>

          {verifyError && <p className="text-destructive">{verifyError}</p>}

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
                <div>
                  <p className="mb-1 text-sm text-muted-foreground">
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
                <div>
                  <p className="mb-1 text-sm text-muted-foreground">
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
    </div>
  )
}
