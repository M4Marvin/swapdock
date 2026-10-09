import { createFileRoute, Link } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { api, formatStatus } from '../../api'
import { queryKeys } from '@/lib/query-keys'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
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

/** Formats an RFC3339 timestamp for display, falling back to the raw value. */
function formatTime(iso: string): string {
  const d = new Date(iso)
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString()
}

function Runs() {
  const runsQuery = useQuery({
    queryKey: queryKeys.runs,
    queryFn: ({ signal }) => api.runs(50, signal),
  })
  const runs = runsQuery.data

  return (
    <div className="space-y-4">
      <h1 className="text-xl font-semibold">Runs</h1>
      {runsQuery.isError && (
        <div
          role="alert"
          className="flex flex-wrap items-center gap-3 rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive"
        >
          <span>Could not load runs: {String(runsQuery.error)}</span>
          <Button
            size="sm"
            variant="outline"
            onClick={() => void runsQuery.refetch()}
          >
            Retry
          </Button>
        </div>
      )}
      {runsQuery.isPending && (
        <div className="space-y-2">
          <Skeleton className="h-6 w-full" />
          <Skeleton className="h-6 w-full" />
          <Skeleton className="h-6 w-2/3" />
        </div>
      )}
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
                <TableCell>{formatTime(run.started)}</TableCell>
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
