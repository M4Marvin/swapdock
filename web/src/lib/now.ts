import { useSyncExternalStore } from "react"

/**
 * One shared 30-second clock.
 *
 * Relative labels are coarse, so a single ticker is enough for every timestamp
 * on screen and avoids one interval per row.
 */
let now = Date.now()
let timer: ReturnType<typeof setInterval> | null = null
const listeners = new Set<() => void>()

function subscribe(listener: () => void) {
  listeners.add(listener)
  if (timer == null) {
    timer = setInterval(() => {
      now = Date.now()
      for (const notify of listeners) notify()
    }, 15_000)
  }
  return () => {
    listeners.delete(listener)
    if (listeners.size === 0 && timer != null) {
      clearInterval(timer)
      timer = null
    }
  }
}

const snapshot = () => now
const serverSnapshot = () => now

/** Current epoch milliseconds, refreshed every fifteen seconds. */
export function useNow(): number {
  return useSyncExternalStore(subscribe, snapshot, serverSnapshot)
}
