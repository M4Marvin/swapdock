import { api, openRunEvents, type RunStream, type TraceEvent } from "@/api"

export type RunPhase = "streaming" | "reconnecting" | "polling"

export interface FollowRunHandlers {
  /** Every recorded step, as it lands. */
  onStep?: (event: TraceEvent) => void
  /** Transport state, for a quiet "following" indicator. */
  onPhase?: (phase: RunPhase) => void
}

export interface FollowRunResult {
  /** Terminal run status, or null when the follow gave up or was cancelled. */
  status: string | null
  error?: string
}

/**
 * Follow one run until it ends.
 *
 * The events endpoint closes an idle stream when a long step (a Docker build)
 * writes nothing. That is a transport event, not a failure: this helper falls
 * back to `/resume` polling and keeps the same promise alive.
 *
 * Pass an AbortSignal to stop following (the run itself is not cancelled).
 */
export function followRun(
  runId: string,
  apiBase: "build" | "deploy",
  handlers: FollowRunHandlers = {},
  signal?: AbortSignal
): Promise<FollowRunResult> {
  return new Promise((resolve) => {
    let settled = false
    let polling = false
    let pollTimer: ReturnType<typeof setTimeout> | null = null
    let stream: RunStream | null = null

    const cleanup = () => {
      if (pollTimer) {
        clearTimeout(pollTimer)
        pollTimer = null
      }
      stream?.close()
      stream = null
      signal?.removeEventListener("abort", onAbort)
    }

    // The `run_end` record is the only place the failure reason is recorded;
    // keep it so the caller can say why, not just that it failed.
    let reason: string | undefined

    const finish = (status: string | null, error?: string) => {
      if (settled) return
      settled = true
      cleanup()
      resolve({ status, error })
    }

    const onAbort = () => finish(null, "cancelled")
    if (signal?.aborted) {
      finish(null, "cancelled")
      return
    }
    signal?.addEventListener("abort", onAbort)

    const poll = () => {
      if (settled || polling) return
      polling = true
      handlers.onPhase?.("polling")
      const started = Date.now()
      let failures = 0

      const tick = async () => {
        if (settled) return
        try {
          const info = await api.resume(runId, apiBase)
          failures = 0
          if (info.ended) {
            finish(info.ended, info.error ?? undefined)
            return
          }
        } catch {
          failures += 1
          if (failures >= 3) {
            finish(null, "the run stream was lost")
            return
          }
        }
        if (Date.now() - started > 45 * 60 * 1000) {
          finish(null, "the run did not finish within 45 minutes")
          return
        }
        pollTimer = setTimeout(() => void tick(), 3000)
      }
      void tick()
    }

    stream = openRunEvents(
      runId,
      {
        onEvent: (event) => {
          if (event.event === "step") handlers.onStep?.(event)
          if (event.event === "run_end" && event.error) reason = event.error
        },
        onStatus: (status) => {
          handlers.onPhase?.(status === "reconnecting" ? "reconnecting" : "streaming")
        },
        // A server error also closes the stream and calls onEnd; let the
        // polling fallback decide whether the run actually finished.
        onError: () => undefined,
        onEnd: (status) => {
          if (status != null) finish(status, reason)
          else poll()
        },
      },
      apiBase
    )
  })
}
