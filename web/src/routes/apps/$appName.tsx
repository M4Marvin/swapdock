import { createFileRoute, Link } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import { api, type AppEntry } from '../../api'
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

function AppDetail() {
  const { appName } = Route.useParams()
  const [entry, setEntry] = useState<AppEntry | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [release, setRelease] = useState('')
  const [since, setSince] = useState('')
  const [verify, setVerify] = useState<Record<string, unknown> | null>(null)
  const [busy, setBusy] = useState(false)
  const [lastRun, setLastRun] = useState<string | null>(null)

  useEffect(() => {
    api.app(appName).then(setEntry, (e) => setError(String(e)))
  }, [appName])

  const act = async (fn: () => Promise<{ run_id: string }>) => {
    setBusy(true)
    setError(null)
    try {
      const { run_id } = await fn()
      setLastRun(run_id)
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }

  if (error && !entry) return <p className="text-destructive">{error}</p>
  if (!entry) return <p className="text-muted-foreground">Loading…</p>

  const { app, problems } = entry

  return (
    <div className="space-y-4">
      <h1 className="text-xl font-semibold">{app.name}</h1>

      <Card>
        <CardHeader>
          <CardTitle>Registry</CardTitle>
        </CardHeader>
        <CardContent>
          <dl className="grid grid-cols-2 gap-x-8 gap-y-1 text-sm">
            <dt className="text-muted-foreground">kind / strategy</dt>
            <dd>
              {app.kind} / {app.strategy}
            </dd>
            <dt className="text-muted-foreground">release</dt>
            <dd className="font-mono">{app.release ?? '—'}</dd>
            <dt className="text-muted-foreground">old release</dt>
            <dd className="font-mono">{app.old_release ?? '—'}</dd>
            <dt className="text-muted-foreground">live / old port</dt>
            <dd>
              {app.live_port ?? '—'} / {app.old_port ?? '—'}
            </dd>
            <dt className="text-muted-foreground">front port</dt>
            <dd>{app.front_port}</dd>
            <dt className="text-muted-foreground">hostnames</dt>
            <dd>{app.hostnames.join(', ') || '—'}</dd>
            <dt className="text-muted-foreground">image</dt>
            <dd className="font-mono">{app.image_repo ?? '—'}</dd>
            <dt className="text-muted-foreground">build host</dt>
            <dd>{app.build_host ?? 'local'}</dd>
          </dl>
        </CardContent>
      </Card>

      {problems.length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle>Problems</CardTitle>
          </CardHeader>
          <CardContent className="space-y-1 text-sm">
            {problems.map((p, i) => (
              <p key={i}>
                <Badge variant={p.severity === 'error' ? 'destructive' : 'outline'}>
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
          <CardTitle>Actions</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-wrap items-center gap-2">
          <Input
            placeholder="release (default: recorded)"
            value={release}
            onChange={(e) => setRelease(e.target.value)}
            className="w-64"
          />
          <Button
            disabled={busy}
            onClick={() => act(() => api.deploy(app.name, release || undefined))}
          >
            Deploy
          </Button>
          <Button
            variant="outline"
            disabled={busy}
            onClick={() => act(() => api.build(app.name, release || undefined))}
          >
            Build
          </Button>
          <Button variant="outline" disabled={busy} onClick={() => act(() => api.rollback(app.name))}>
            Rollback
          </Button>
          <Button variant="outline" disabled={busy} onClick={() => act(() => api.sync(app.name))}>
            Sync
          </Button>
          {lastRun && (
            <Link to="/runs/$runId" params={{ runId: lastRun }} className="underline">
              run {lastRun.slice(0, 12)}
            </Link>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Verify access log</CardTitle>
        </CardHeader>
        <CardContent className="space-y-2">
          <div className="flex gap-2">
            <Input
              placeholder="since: RFC3339 timestamp or run id"
              value={since}
              onChange={(e) => setSince(e.target.value)}
              className="w-80"
            />
            <Button
              variant="outline"
              disabled={!since}
              onClick={() => api.verify(app.name, since).then(setVerify)}
            >
              Check
            </Button>
          </div>
          {verify && <pre>{JSON.stringify(verify, null, 2)}</pre>}
        </CardContent>
      </Card>
    </div>
  )
}
