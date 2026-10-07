import { createFileRoute } from '@tanstack/react-router'
import { useEffect, useRef, useState } from 'react'
import { api, type TraceEvent } from '../../api'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'

export const Route = createFileRoute('/runs/$runId')({
  component: RunDetail,
})

function RunDetail() {
  const { runId } = Route.useParams()
  const [events, setEvents] = useState<TraceEvent[] | null>(null)
  const [ended, setEnded] = useState<string | null>(null)
  const timer = useRef<ReturnType<typeof setInterval> | null>(null)

  useEffect(() => {
    let alive = true
    const tick = async () => {
      try {
        const [evs, resume] = await Promise.all([api.run(runId), api.resume(runId)])
        if (!alive) return
        setEvents(evs)
        setEnded(resume.ended)
        if (resume.ended && timer.current) {
          clearInterval(timer.current)
          timer.current = null
        }
      } catch {
        // keep polling; the run may not have written yet
      }
    }
    tick()
    timer.current = setInterval(tick, 2000)
    return () => {
      alive = false
      if (timer.current) clearInterval(timer.current)
    }
  }, [runId])

  const steps = events?.filter((e) => e.event === 'step') ?? []

  return (
    <div className="space-y-4">
      <h1 className="text-xl font-semibold">
        Run <span className="font-mono">{runId.slice(0, 12)}</span>
      </h1>
      {ended && <p className="text-muted-foreground">ended: {ended}</p>}
      {!events && <p className="text-muted-foreground">Loading…</p>}
      {events && steps.length === 0 && (
        <p className="text-muted-foreground">No steps recorded yet.</p>
      )}
      {steps.length > 0 && (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>#</TableHead>
              <TableHead>Step</TableHead>
              <TableHead>Status</TableHead>
              <TableHead>Exit</TableHead>
              <TableHead>ms</TableHead>
              <TableHead>Error</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {steps.map((s, i) => (
              <TableRow key={i}>
                <TableCell>{s.seq}</TableCell>
                <TableCell>{s.step}</TableCell>
                <TableCell>{s.status}</TableCell>
                <TableCell>{s.exit_code ?? '—'}</TableCell>
                <TableCell>{s.duration_ms ?? '—'}</TableCell>
                <TableCell className="text-destructive">{s.error ?? ''}</TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </div>
  )
}
