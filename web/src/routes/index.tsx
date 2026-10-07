import { createFileRoute, Link } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import { api, type AppEntry } from '../api'
import { Badge } from '@/components/ui/badge'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'

export const Route = createFileRoute('/')({
  component: Index,
})

function Index() {
  const [apps, setApps] = useState<AppEntry[] | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [validate, setValidate] = useState<{
    status: number
    errors: number
    warnings: number
  } | null>(null)

  useEffect(() => {
    api.apps().then(setApps, (e) => setError(String(e)))
    api.validate().then(
      (r) =>
        setValidate({
          status: r.status,
          errors: r.body.errors,
          warnings: r.body.warnings,
        }),
      () => setValidate(null),
    )
  }, [])

  return (
    <div className="space-y-4">
      <h1 className="text-xl font-semibold">Apps</h1>

      {validate && (
        <div className="flex gap-2">
          <Badge variant={validate.errors > 0 ? 'destructive' : 'outline'}>
            {validate.errors} errors
          </Badge>
          <Badge variant="outline">{validate.warnings} warnings</Badge>
        </div>
      )}

      {error && <p className="text-destructive">{error}</p>}
      {!apps && !error && <p className="text-muted-foreground">Loading…</p>}

      {apps && (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Name</TableHead>
              <TableHead>Kind</TableHead>
              <TableHead>Strategy</TableHead>
              <TableHead>Release</TableHead>
              <TableHead>Live port</TableHead>
              <TableHead>Hostnames</TableHead>
              <TableHead>Issues</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {apps.map(({ app, problems }) => (
              <TableRow key={app.name}>
                <TableCell>
                  <Link
                    to="/apps/$appName"
                    params={{ appName: app.name }}
                    className="underline"
                  >
                    {app.name}
                  </Link>
                </TableCell>
                <TableCell>{app.kind}</TableCell>
                <TableCell>{app.strategy}</TableCell>
                <TableCell className="font-mono">{app.release ?? '—'}</TableCell>
                <TableCell>{app.live_port ?? '—'}</TableCell>
                <TableCell>{app.hostnames.join(', ') || '—'}</TableCell>
                <TableCell>
                  {problems.length === 0 ? (
                    <Badge variant="outline">0</Badge>
                  ) : (
                    <Badge variant={problems.some((p) => p.severity === 'error') ? 'destructive' : 'outline'}>
                      {problems.length}
                    </Badge>
                  )}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </div>
  )
}
