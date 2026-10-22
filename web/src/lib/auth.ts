import { useSyncExternalStore } from "react"

/**
 * Console authentication.
 *
 * Preferred mode is an HttpOnly cookie session: the shared access token is
 * exchanged once at `/api/login`, the token is not kept in JavaScript, and
 * state-changing requests carry a per-session CSRF header. The bearer-token
 * mode remains as a fallback for an older API that has no `/api/login`, and
 * for API clients that never use the console.
 */
const TOKEN_KEY = "swapdock-api-token"
const MODE_KEY = "swapdock-auth-mode"
const CSRF_KEY = "swapdock-csrf-token"

export type AuthMode = "token" | "session"

export interface AuthState {
  mode: AuthMode | null
  token: string | null
  csrf: string | null
  /** True after an API call returned 401, so the gate can explain the failure. */
  rejected: boolean
}

function read(key: string): string | null {
  try {
    return sessionStorage.getItem(key)
  } catch {
    return null
  }
}

function write(key: string, value: string | null) {
  try {
    if (value == null) sessionStorage.removeItem(key)
    else sessionStorage.setItem(key, value)
  } catch {
    // A blocked storage API still leaves the in-memory state usable for this tab.
  }
}

function initial(): AuthState {
  if (typeof window === "undefined") {
    return { mode: null, token: null, csrf: null, rejected: false }
  }
  const mode = read(MODE_KEY)
  if (mode === "session") {
    const csrf = read(CSRF_KEY)
    if (csrf) return { mode: "session", token: null, csrf, rejected: false }
  }
  if (mode === "token") {
    const token = read(TOKEN_KEY)
    if (token) return { mode: "token", token, csrf: null, rejected: false }
  }
  return { mode: null, token: null, csrf: null, rejected: false }
}

let state: AuthState = initial()

const listeners = new Set<() => void>()

function emit() {
  for (const listener of listeners) listener()
}

export function subscribeAuth(listener: () => void): () => void {
  listeners.add(listener)
  return () => listeners.delete(listener)
}

export function getAuthState(): AuthState {
  return state
}

/** React binding for the gate and the sign-out control. */
export function useAuthState(): AuthState {
  return useSyncExternalStore(subscribeAuth, getAuthState, getAuthState)
}

export function getToken(): string | null {
  return state.token
}

/** Store the fallback bearer token for an older API. */
export function setToken(token: string) {
  const clean = token.trim()
  state = {
    mode: clean === "" ? null : "token",
    token: clean === "" ? null : clean,
    csrf: null,
    rejected: false,
  }
  write(TOKEN_KEY, clean === "" ? null : clean)
  write(CSRF_KEY, null)
  write(MODE_KEY, clean === "" ? null : "token")
  emit()
}

/** Store the CSRF token after a successful cookie-session login. */
export function setSession(csrf: string) {
  state = { mode: "session", token: null, csrf, rejected: false }
  write(TOKEN_KEY, null)
  write(CSRF_KEY, csrf)
  write(MODE_KEY, "session")
  emit()
}

/** Sign out, or record that the API rejected the credential that was sent. */
export function clearAuth(rejected = false) {
  state = { mode: null, token: null, csrf: null, rejected }
  write(TOKEN_KEY, null)
  write(CSRF_KEY, null)
  write(MODE_KEY, null)
  emit()
}

/** Kept for callers that only know about the old token flow. */
export const clearToken = clearAuth

/** Called by the API layer on any 401 before throwing. */
export function markUnauthorized() {
  clearAuth(true)
}

/** Headers for one API request, preserving any content type already set. */
export function authHeaders(init?: HeadersInit): Headers {
  const headers = new Headers(init)
  if (state.mode === "token" && state.token) {
    headers.set("Authorization", `Bearer ${state.token}`)
  }
  if (state.mode === "session" && state.csrf) {
    headers.set("X-CSRF-Token", state.csrf)
  }
  return headers
}

/** EventSource cannot send headers; the server accepts `?token=` for SSE only. */
export function withTokenParam(url: string): string {
  if (state.mode !== "token" || !state.token) return url
  const separator = url.includes("?") ? "&" : "?"
  return `${url}${separator}token=${encodeURIComponent(state.token)}`
}

/** End the server-side session (when there is one), then clear this tab. */
export async function logout(): Promise<void> {
  if (state.mode === "session") {
    try {
      await fetch("/api/logout", {
        method: "POST",
        headers: authHeaders(),
        credentials: "same-origin",
      })
    } catch {
      // Clearing the local tab is still the right end state.
    }
  }
  clearAuth()
}
