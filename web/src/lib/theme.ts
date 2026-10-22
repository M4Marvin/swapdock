import { useCallback, useEffect, useState } from "react"

export type Theme = "light" | "dark" | "system"
export type ResolvedTheme = "light" | "dark"

const STORAGE_KEY = "swapdock-theme"
const EVENT = "swapdock:theme"

function readTheme(): Theme {
  try {
    const saved = localStorage.getItem(STORAGE_KEY)
    if (saved === "light" || saved === "dark" || saved === "system") return saved
  } catch {
    // Private mode / disabled storage: follow the system.
  }
  return "system"
}

function systemPrefersDark(): boolean {
  return typeof window !== "undefined" && window.matchMedia("(prefers-color-scheme: dark)").matches
}

function applyTheme(theme: Theme, prefersDark: boolean) {
  const dark = theme === "dark" || (theme === "system" && prefersDark)
  document.documentElement.classList.toggle("dark", dark)
  document.documentElement.style.colorScheme = dark ? "dark" : "light"
  const meta = document.querySelector('meta[name="theme-color"]')
  if (meta) meta.setAttribute("content", dark ? "#0a0a0b" : "#f8f8f9")
}

/**
 * Theme preference with a safe default: follow the OS until the operator
 * chooses. The boot script in index.html applies this before first paint, so
 * this hook only keeps state and the toggle in sync.
 */
export function useTheme() {
  const [theme, setThemeState] = useState<Theme>(readTheme)
  const [prefersDark, setPrefersDark] = useState(systemPrefersDark)

  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)")
    const onChange = () => setPrefersDark(media.matches)
    media.addEventListener("change", onChange)

    const onThemeEvent = () => setThemeState(readTheme())
    window.addEventListener(EVENT, onThemeEvent)
    window.addEventListener("storage", onThemeEvent)

    return () => {
      media.removeEventListener("change", onChange)
      window.removeEventListener(EVENT, onThemeEvent)
      window.removeEventListener("storage", onThemeEvent)
    }
  }, [])

  useEffect(() => {
    applyTheme(theme, prefersDark)
  }, [theme, prefersDark])

  const setTheme = useCallback((next: Theme) => {
    setThemeState(next)
    try {
      localStorage.setItem(STORAGE_KEY, next)
    } catch {
      // The visual change still applies for this session.
    }
    window.dispatchEvent(new Event(EVENT))
  }, [])

  const resolved: ResolvedTheme =
    theme === "system" ? (prefersDark ? "dark" : "light") : theme

  /** System → light → dark → system, so one button can reach every mode. */
  const cycleTheme = useCallback(() => {
    setTheme(theme === "system" ? "light" : theme === "light" ? "dark" : "system")
  }, [theme, setTheme])

  return { theme, resolved, setTheme, cycleTheme }
}
