import { createFileRoute, Link } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import { api, formatStatus, type RunSummary } from '../../api'
import { Badge } from '@/components/ui/badge'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'

export const Route = createFileRoute('/runs/')({
  component: Runs,
})

function Runs() {
  const [runs, setRuns] = useState<RunSummary[] | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    const controller = new AbortController()
    api.runs(50, controller.signal).then(
      (r) => {
        if (!controller.signal.aborted) setRuns(r)
      },
      (e) => {
        if (!controller.signal.aborted) setError(String(e))
      },
    )
    return () => controller.abort()
  }, [])

  return (
    <div className="space-y-4">
      <h1 className="text-xl font-semibold">Runs</h1>
      {error && <p className="text-destructive">{error}</p>}
      {!runs && !error && <p className="text-muted-foreground">Loading…</p>}
      {runs && runs.length === 0 && (
        <p className="text-muted-foreground">No runs recorded yet.</p>
      )}
      {runs && runs.length > 0 && (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Run</TableHead>
              <TableHead>App</TableHead>
              <TableHead>Started</TableHead>
              <TableHead>Status</TableHead>
              <TableHead>Steps</TableHead>
              <TableHead>Non-ok</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {runs.map((run) => (
              <TableRow key={run.run_id}>
                <TableCell className="font-mono">
                  <Link to="/runs/$runId" params={{ runId: run.run_id }} className="underline">
                    {run.run_id.slice(0, 12)}
                  </Link>
                </TableCell>
                <TableCell>
                  {run.app ?? <span className="text-muted-foreground">—</span>}
                </TableCell>
                <TableCell>{run.started}</TableCell>
                <TableCell>
                  <Badge variant={run.non_ok > 0 ? 'destructive' : 'outline'}>
                    {formatStatus(run.status)}
                  </Badge>
                </TableCell>
                <TableCell>{run.steps}</TableCell>
                <TableCell>{run.non_ok || '—'}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </div>
  )
}
