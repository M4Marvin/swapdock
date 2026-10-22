/** Shared formatting helpers. Every label in the UI goes through one of these. */

/** Coarse, calm relative time: "just now", "4m ago", "2d ago". */
export function formatRelative(input: string | number | Date, now = Date.now()): string {
  const then = input instanceof Date ? input.getTime() : typeof input === "number" ? input : Date.parse(input)
  if (!Number.isFinite(then)) return "—"
  const delta = Math.round((then - now) / 1000)
  const abs = Math.abs(delta)

  if (abs < 10) return "just now"
  if (abs < 60) return delta > 0 ? `in ${abs}s` : `${abs}s ago`
  if (abs < 3600) {
    const m = Math.round(abs / 60)
    return delta > 0 ? `in ${m}m` : `${m}m ago`
  }
  if (abs < 86_400) {
    const h = Math.round(abs / 3600)
    return delta > 0 ? `in ${h}h` : `${h}h ago`
  }
  if (abs < 7 * 86_400) {
    const d = Math.round(abs / 86_400)
    return delta > 0 ? `in ${d}d` : `${d}d ago`
  }
  return new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" }).format(then)
}

/** Absolute timestamp for tooltips and run details. */
export function formatExact(input: string | number | Date): string {
  const date = input instanceof Date ? input : new Date(input)
  if (Number.isNaN(date.getTime())) return "—"
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
    second: "2-digit",
  }).format(date)
}

/** A build timestamp precise enough to tell two versions apart: "Oct 19, 5:24 PM". */
export function formatShortDateTime(input: string | number | Date): string {
  const date = input instanceof Date ? input : new Date(input)
  if (Number.isNaN(date.getTime())) return "—"
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  }).format(date)
}

/** Short date for day grouping: "Today", "Yesterday", or "Oct 12". */
export function formatDay(input: string | number | Date, now = new Date()): string {
  const date = input instanceof Date ? input : new Date(input)
  if (Number.isNaN(date.getTime())) return "Unknown"
  const startOf = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime()
  const days = Math.round((startOf(now) - startOf(date)) / 86_400_000)
  if (days === 0) return "Today"
  if (days === 1) return "Yesterday"
  return new Intl.DateTimeFormat(undefined, {
    weekday: "short",
    month: "short",
    day: "numeric",
  }).format(date)
}

/** Time of day with seconds, for streamed step rows. */
export function formatClock(input: string | number | Date): string {
  const date = input instanceof Date ? input : new Date(input)
  if (Number.isNaN(date.getTime())) return "—"
  return new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(date)
}

/** Step and run durations, rounded to something a human can scan. */
export function formatDuration(ms: number | null | undefined): string {
  if (ms == null || !Number.isFinite(ms)) return "—"
  if (ms < 1000) return `${Math.max(0, Math.round(ms))}ms`
  if (ms < 60_000) return `${(ms / 1000).toFixed(ms < 10_000 ? 1 : 0)}s`
  const minutes = Math.floor(ms / 60_000)
  const seconds = Math.round((ms % 60_000) / 1000)
  if (minutes < 60) return `${minutes}m ${seconds}s`
  const hours = Math.floor(minutes / 60)
  return `${hours}h ${minutes % 60}m`
}

/** "13 apps", "1 app". */
export function plural(count: number, singular: string, pluralForm = `${singular}s`): string {
  return `${count} ${count === 1 ? singular : pluralForm}`
}

/** First 8 characters of an id, with the full value kept for a title. */
export function shortId(id: string, length = 8): string {
  return id.length <= length ? id : id.slice(0, length)
}

export function formatCount(value: number): string {
  return new Intl.NumberFormat(undefined, { notation: value > 9999 ? "compact" : "standard" }).format(value)
}

/** Operator-facing text for a thrown value. */
export function errorText(error: unknown): string {
  if (error instanceof Error) return error.message
  if (typeof error === "string") return error
  return "unexpected error"
}

/** Deterministic hue for an app glyph, so the same app looks the same everywhere. */
export function hueFor(name: string): number {
  let hash = 0
  for (let i = 0; i < name.length; i += 1) {
    hash = (hash * 31 + name.charCodeAt(i)) | 0
  }
  return Math.abs(hash) % 360
}

/** Stable initials for an app glyph. */
export function initialsFor(name: string): string {
  const clean = name.replace(/[^a-zA-Z0-9]+/g, " ").trim()
  if (!clean) return "?"
  const parts = clean.split(" ")
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase()
  return (parts[0][0] + parts[1][0]).toUpperCase()
}

/** A release is a commit: show at most 12 characters. */
export function shortRelease(release: string | null | undefined): string {
  if (!release) return "—"
  return release.length > 12 ? release.slice(0, 12) : release
}

/** Type guard for new deployable release names (older 4–6 char tags may display). */
export function isRelease(value: string): boolean {
  return /^[0-9a-fA-F]{7,64}$/.test(value)
}
