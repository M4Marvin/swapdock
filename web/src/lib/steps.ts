import type { TraceEvent } from "@/api"

/**
 * Merge a `step_start` into the list, or replace it with the matching `step`
 * that carries the same seq. Both records are the same step; the caller should
 * see one row that goes from running to finished.
 */
export function mergeStep(steps: TraceEvent[], event: TraceEvent): TraceEvent[] {
  if (event.event !== "step_start" && event.event !== "step") return steps
  const seq = event.seq
  const next =
    seq == null
      ? [...steps, event]
      : [...steps.filter((step) => step.seq !== seq), event]
  return next.sort((a, b) => (a.seq ?? 0) - (b.seq ?? 0))
}

/** The latest record for every step, sorted by seq. */
export function mergeSteps(events: TraceEvent[]): TraceEvent[] {
  return events.reduce(mergeStep, [] as TraceEvent[])
}
