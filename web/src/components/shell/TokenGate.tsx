import { useState } from "react"
import { KeyRound, LockKeyhole } from "lucide-react"
import { useQueryClient } from "@tanstack/react-query"
import { markUnauthorized, setSession, setToken, useAuthState } from "@/lib/auth"
import { Wordmark } from "@/components/Brand"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Callout } from "@/components/kit"

/**
 * The one screen rendered when this tab has no console session.
 *
 * The access token is exchanged for an HttpOnly cookie session. The token is
 * never stored after a successful login. Against an older API without
 * `/api/login`, the form falls back to the bearer-token flow.
 */
export function TokenGate() {
  const auth = useAuthState()
  const queryClient = useQueryClient()
  const [value, setValue] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const submit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    const clean = value.trim()
    if (clean === "") {
      setError("Enter the access token configured on the API host.")
      return
    }
    setError(null)
    setBusy(true)
    queryClient.clear()
    try {
      const res = await fetch("/api/login", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        credentials: "same-origin",
        body: JSON.stringify({ password: clean }),
      })
      if (res.ok) {
        const body = (await res.json()) as { csrf_token?: string }
        if (typeof body.csrf_token === "string" && body.csrf_token !== "") {
          setSession(body.csrf_token)
          return
        }
        setError("The API returned an incomplete login response.")
        return
      }
      if (res.status === 404 || res.status === 405) {
        // Older API: no cookie session endpoint; keep the bearer flow.
        setToken(clean)
        return
      }
      if (res.status === 401) {
        markUnauthorized()
        return
      }
      setError(`Login failed (HTTP ${res.status}).`)
    } catch {
      setError("The API could not be reached. Check the host and each proxy hop.")
    } finally {
      setBusy(false)
    }
  }

  return (
    <main className="page-glow grid min-h-dvh place-items-center px-6 py-10">
      <div className="w-full max-w-sm">
        <div className="mb-6 flex justify-center">
          <Wordmark />
        </div>

        <form
          onSubmit={submit}
          className="rounded-2xl border border-border bg-card p-6 shadow-[var(--shadow-pop)]"
        >
          <span className="flex size-9 items-center justify-center rounded-xl border border-border bg-elevated text-muted-foreground">
            <LockKeyhole className="size-4" />
          </span>
          <h1 className="mt-4 text-xl font-semibold tracking-[-0.02em] text-foreground">
            Access token
          </h1>
          <p className="mt-1.5 text-pretty text-sm leading-relaxed text-muted-foreground">
            Sign in with the token configured on the host. A successful sign-in
            creates an HttpOnly session cookie; the token is not kept in this tab.
          </p>

          {auth.rejected ? (
            <Callout tone="danger" title="That token was not accepted" className="mt-4">
              The API returned 401. Check <span className="font-mono">SWAPDOCK_API_TOKEN</span>{" "}
              on the host running <span className="font-mono">swapdock serve</span>, then try
              again.
            </Callout>
          ) : null}

          <div className="mt-4 space-y-1.5">
            <label
              htmlFor="api-access-token"
              className="text-xs font-medium text-muted-foreground"
            >
              Access token
            </label>
            <Input
              id="api-access-token"
              type="password"
              value={value}
              onChange={(event) => setValue(event.target.value)}
              placeholder="Paste the API token"
              autoComplete="current-password"
              autoFocus
              spellCheck={false}
              aria-invalid={error != null || auth.rejected}
              aria-describedby="api-access-token-hint"
              className="font-mono"
            />
            <p id="api-access-token-hint" className="text-2xs text-subtle-foreground">
              The token configured for <span className="font-mono">swapdock serve</span>. It is
              never stored in localStorage; a session cookie is used where the API supports it.
            </p>
            {error ? (
              <p role="alert" className="text-2xs text-destructive-strong">
                {error}
              </p>
            ) : null}
          </div>

          <Button type="submit" className="mt-4 w-full" disabled={busy}>
            <KeyRound />
            {busy ? "Signing in…" : "Sign in"}
          </Button>
        </form>
      </div>
    </main>
  )
}
