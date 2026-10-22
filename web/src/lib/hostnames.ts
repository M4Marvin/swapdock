import type { App } from "@/api"

/** Hosts that are only reachable over a local network or a tailnet. */
const LOCAL_SUFFIXES = [".local", ".internal", ".home", ".lan", ".ts.net", ".tailnet"]

/**
 * True for loopback, RFC1918, link-local and tailnet/CGNAT hostnames.
 *
 * Public DNS names and public IPs are served over TLS; private ones almost
 * never are. This only picks the link scheme, so a wrong guess degrades to a
 * redirect rather than a dead link.
 */
export function isLocalHost(hostname: string): boolean {
  const bare = hostname
    .trim()
    .toLowerCase()
    .replace(/^[a-z]+:\/\//, "")
    .replace(/\/.*$/, "")
    .replace(/^\[/, "")
    .replace(/\](:\d+)?$/, "")
    .replace(/:\d+$/, "")
  if (bare === "" || bare === "localhost" || bare === "::1") return true
  if (bare.endsWith(".localhost")) return true
  if (LOCAL_SUFFIXES.some((suffix) => bare.endsWith(suffix))) return true
  if (/^127\./.test(bare) || /^10\./.test(bare) || /^192\.168\./.test(bare)) return true
  if (/^169\.254\./.test(bare)) return true
  if (/^172\.(1[6-9]|2\d|3[01])\./.test(bare)) return true
  // 100.64.0.0/10 is CGNAT; Tailscale is the common operator use.
  if (/^100\.(6[4-9]|[7-9]\d|1[01]\d|12[0-7])\./.test(bare)) return true
  return false
}

/** Scheme-aware link for one hostname, preferring an explicit public URL. */
export function hostnameUrl(hostname: string, publicUrl?: string | null): string {
  if (publicUrl) {
    try {
      const url = new URL(publicUrl, window.location.origin)
      if (url.protocol === "http:" || url.protocol === "https:") {
        return `${url.protocol}//${url.host}`
      }
    } catch {
      // A relative or malformed public_url is not a site link; use the host.
    }
  }
  if (/^https?:\/\//i.test(hostname)) return hostname
  return `${isLocalHost(hostname) ? "http" : "https"}://${hostname}`
}

/** The best external link for an app, or null when it has no hostname. */
export function appSiteUrl(app: Pick<App, "hostnames" | "public_url">): string | null {
  const hostname = app.hostnames[0]
  if (!hostname) return null
  return hostnameUrl(hostname, app.public_url)
}
