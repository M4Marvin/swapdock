import { createFileRoute } from '@tanstack/react-router'
import { useEffect, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import {
  ApiError,
  api,
  formatStatus,
  openRunEvents,
  type RunStream,
  type TraceEvent,
} from '../../api'
import { queryKeys } from '@/lib/query-keys'
import { Badge } from '@/components/ui/badge'
import { Skeleton } from '@/components/ui/skeleton'
import {
  Card,
  CardAction,
  CardContent,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
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

/** How long the client waits without an event before closing the stream. */
const IDLE_TIMEOUT_MS = 60_000

function StepStatusBadge({ status }: { status?: string }) {
  const key = status?.toLowerCase()
  if (key === 'ok') {
    return (
      <Badge
        variant="outline"
        className="border-emerald-500/40 bg-emerald-500/10 text-emerald-500"
      >
        ok
      </Badge>
    )
  }
  if (key === 'error' || key === 'timeout') {
    return <Badge variant="destructive">{formatStatus(status ?? '')}</Badge>
  }
  return <Badge variant="secondary">{status ? formatStatus(status) : '—'}</Badge>
}

function EndedBadge({ status }: { status: string }) {
  const s = status.toLowerCase()
  if (s.includes('ok') || s.includes('succeed')) {
    return (
      <Badge
        variant="outline"
        className="border-emerald-500/40 bg-emerald-500/10 text-emerald-500"
      >
        {formatStatus(status)}
      </Badge>
    )
  }
  if (s.includes('fail') || s.includes('error')) {
    return <Badge variant="destructive">{formatStatus(status)}</Badge>
  }
  return <Badge variant="secondary">{formatStatus(status)}</Badge>
}

type Phase = 'loading' | 'streaming' | 'reconnecting' | 'ended' | 'error'

function RunDetail() {
  const { runId } = Route.useParams()
  const [events, setEvents] = useState<TraceEvent[]>([])
  const [ended, setEnded] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [phase, setPhase] = useState<Phase>('loading')

  // Ask whether the run exists (and is over) before opening a stream, so an
  // unknown or already-finished run does not hold a connection open. The
  // stream itself stays a custom EventSource; this is only the pre-check.
  const resumeQuery = useQuery({
    queryKey: queryKeys.run(runId),
    queryFn: ({ signal }) => api.resume(runId, signal),
    retry: 0,
    staleTime: Infinity,
    refetchOnWindowFocus: false,
  })

  useEffect(() => {
    setEvents([])
    setEnded(null)
    setError(null)
    setPhase('loading')
  }, [runId])

  useEffect(() => {
    if (resumeQuery.status === 'pending') return

    const controller = new AbortController()
    let stream: RunStream | null = null
    let idle: ReturnType<typeof setTimeout> | null = null

    const clearIdle = () => {
      if (idle) {
        clearTimeout(idle)
        idle = null
      }
    }
    const finish = (status: string | null) => {
      clearIdle()
      setEnded(status ?? 'stream closed')
      setPhase('ended')
    }
    // Backstop: a dangling run whose writer stopped still ends the stream.
    const armIdle = () => {
      clearIdle()
      idle = setTimeout(() => {
        stream?.close()
        finish('no events for 60s — stream closed')
      }, IDLE_TIMEOUT_MS)
    }

    const startStream = () => {
      setPhase('streaming')
      stream = openRunEvents(runId, {
        onEvent: (ev) => {
          setEvents((cur) => [...cur, ev])
          armIdle()
        },
        onError: (message) => setError(message),
        onStatus: (s) =>
          setPhase(s === 'reconnecting' ? 'reconnecting' : 'streaming'),
        onEnd: (status) => finish(status),
      })
      armIdle()
    }

    // An already-finished run is shown from the recorded trace, no stream.
    const loadHistory = async (status: string) => {
      try {
        const past = await api.run(runId, controller.signal)
        if (controller.signal.aborted) return
        setEvents(past)
        finish(status)
      } catch (e) {
        if (controller.signal.aborted) return
        setError(String(e))
        setPhase('error')
      }
    }

    // `/resume` only knows about runs that recorded a step; fall back to the
    // full trace so a zero-step run is not mistaken for a missing one.
    const fallbackToTrace = async () => {
      try {
        const past = await api.run(runId, controller.signal)
        if (controller.signal.aborted) return
        const end = past.find((ev) => ev.event === 'run_end')
        if (end) {
          setEvents(past)
          finish(end.status ?? 'stream closed')
        } else {
          startStream()
        }
      } catch {
        if (controller.signal.aborted) return
        setPhase('error')
        setError('run not found')
      }
    }

    if (resumeQuery.status === 'success') {
      if (resumeQuery.data.ended != null) {
        void loadHistory(resumeQuery.data.ended)
      } else {
        startStream()
      }
    } else {
      const e = resumeQuery.error
      if (e instanceof ApiError && e.status === 404) {
        void fallbackToTrace()
      } else {
        setPhase('error')
        setError(String(e))
      }
    }

    return () => {
      controller.abort()
      clearIdle()
      stream?.close()
    }
    // Only re-run when the run changes or the pre-check resolves; a background
    // refetch of the same key must not tear down a live stream.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runId, resumeQuery.status])

  const start = events.find((e) => e.event === 'run_start')
  const end = events.find((e) => e.event === 'run_end')
  const steps = events
    .filter((e) => e.event === 'step')
    .sort((a, b) => (a.seq ?? 0) - (b.seq ?? 0))

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-baseline gap-3">
        <h1 className="text-xl font-semibold">
          Run <span className="font-mono">{runId.slice(0, 12)}</span>
        </h1>
        {start?.app && <Badge variant="secondary">{start.app}</Badge>}
        {start?.mode && <Badge variant="outline">{start.mode}</Badge>}
      </div>

      {error ? (
        <div className="flex items-center gap-2 rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </div>
      ) : ended ? (
        <div className="flex items-center gap-2 rounded-lg border bg-card px-3 py-2 text-sm">
          <span className="text-muted-foreground">ended:</span>
          <EndedBadge status={ended} />
        </div>
      ) : phase === 'reconnecting' ? (
        <p className="flex items-center gap-2 text-sm text-muted-foreground">
          <span className="size-2 animate-pulse rounded-full bg-amber-500" />
          reconnecting…
        </p>
      ) : phase === 'streaming' ? (
        <p className="flex items-center gap-2 text-sm text-muted-foreground">
          <span className="size-2 animate-pulse rounded-full bg-emerald-500" />
          streaming…
        </p>
      ) : phase === 'loading' ? (
        <p className="text-sm text-muted-foreground">Loading…</p>
      ) : null}

      <Card className="py-0">
        <CardHeader className="border-b py-4">
          <CardTitle>Steps</CardTitle>
          <CardAction>
            <span className="text-sm text-muted-foreground">
              {steps.length} step{steps.length === 1 ? '' : 's'}
              {typeof end?.steps === 'number' ? ` · ${end.steps} reported` : ''}
            </span>
          </CardAction>
        </CardHeader>
        <CardContent className="px-0">
          {steps.length === 0 ? (
            phase === 'loading' || phase === 'streaming' || phase === 'reconnecting' ? (
              <div className="space-y-2 p-6">
                <Skeleton className="h-6 w-full" />
                <Skeleton className="h-6 w-full" />
              </div>
            ) : (
              <p className="p-6 text-sm text-muted-foreground">
                No steps recorded.
              </p>
            )
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="pl-6">#</TableHead>
                  <TableHead>Step</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead>Exit</TableHead>
                  <TableHead>ms</TableHead>
                  <TableHead className="pr-6">Error</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {steps.map((s, i) => (
                  <TableRow key={`${s.seq ?? i}-${s.step ?? ''}`}>
                    <TableCell className="pl-6 text-muted-foreground">
                      {s.seq ?? i + 1}
                    </TableCell>
                    <TableCell className="font-medium">{s.step}</TableCell>
                    <TableCell>
                      <StepStatusBadge status={s.status} />
                    </TableCell>
                    <TableCell>{s.exit_code ?? '—'}</TableCell>
                    <TableCell>{s.duration_ms ?? '—'}</TableCell>
                    <TableCell className="pr-6 text-destructive">
                      {s.error ?? ''}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>
    </div>
  )
}
