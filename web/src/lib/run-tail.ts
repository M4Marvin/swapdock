import {
  ApiError,
  api,
  classifyStatus,
  openRunEvents,
  type RunStream,
  type TraceEvent,
} from "@/api"

export type RunPhase = "streaming" | "reconnecting" | "polling"

export interface FollowRunHandlers {
  /** Every recorded step, as it lands. A start is followed by its finish. */
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
 * A quiet stream is normal: a Docker build can write nothing for minutes. The
 * server keeps it open while the run's process is alive, but a restart or a
 * proxy can still drop it. That is a transport event, not a failure: this
 * helper falls back to `/resume` polling and treats `interrupted` as final.
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
    /** The stream delivered `run_start`, so the id is real even at 404 resume. */
    let sawStart = false

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
          if (classifyStatus(info.state) === "interrupted") {
            finish("interrupted", info.error ?? undefined)
            return
          }
        } catch (caught) {
          // A run killed inside its first step has no completed step yet, so
          // `/resume` is a 404 even though the run exists. Do not call that a
          // lost stream once `run_start` has been delivered.
          const firstStep =
            caught instanceof ApiError &&
            caught.status === 404 &&
            sawStart
          if (!firstStep) {
            failures += 1
            if (failures >= 3) {
              finish(null, "the run stream was lost")
              return
            }
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
          if (event.event === "run_start") sawStart = true
          if (event.event === "step" || event.event === "step_start") {
            handlers.onStep?.(event)
          }
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
