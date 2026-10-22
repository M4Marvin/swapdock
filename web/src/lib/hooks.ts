import { useCallback, useEffect, useRef, useState } from "react"

/** Set a page-specific document title: "Runs · swapdock". */
export function useDocumentTitle(title?: string) {
  useEffect(() => {
    document.title = title ? `${title} · swapdock` : "swapdock"
    return () => {
      document.title = "swapdock"
    }
  }, [title])
}

/**
 * Clipboard helper with a transient "copied" flag.
 *
 * No toast: copying a run id should feel like a quiet confirmation, not a
 * notification. The calling button swaps its icon for a check instead.
 */
export function useCopy(resetAfterMs = 1600) {
  const [copied, setCopied] = useState(false)
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    return () => {
      if (timer.current) clearTimeout(timer.current)
    }
  }, [])

  const copy = useCallback(
    async (value: string) => {
      try {
        await navigator.clipboard.writeText(value)
        setCopied(true)
        if (timer.current) clearTimeout(timer.current)
        timer.current = setTimeout(() => setCopied(false), resetAfterMs)
        return true
      } catch {
        return false
      }
    },
    [resetAfterMs]
  )

  return { copied, copy }
}

/** Keyboard shortcut matcher that works on macOS and everywhere else. */
export function isCommandK(event: KeyboardEvent): boolean {
  return (event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k"
}

/** The platform's command-modifier glyph, for keyboard hints. */
export function useModKey(): "⌘" | "Ctrl" {
  if (typeof navigator === "undefined") return "Ctrl"
  return /Mac|iPhone|iPad|iPod/i.test(navigator.userAgent) ? "⌘" : "Ctrl"
}
