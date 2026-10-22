import { useQuery } from "@tanstack/react-query"
import { api, type App, type AppDrift, type AppDriftState } from "@/api"
import { queryKeys } from "@/lib/query-keys"
import { imageForRelease } from "@/lib/releases"
import { isAppUp, useAppHealth } from "@/components/Status"

/** A display-only state: a static root is served even without a recorded release. */
type CardState = AppDriftState | "static_served"

export interface VersionDriftView {
  state: CardState
  /** True when the API answered, false when this is the defensive fallback. */
  authoritative: boolean
  loading: boolean
  checking: boolean
  error: unknown
  liveRelease: string | null
  livePort: number | null
  liveUp: boolean | null
  latestRelease: string | null
  latestSource: string | null
  latestBranch: string
  latestSubject: string | null
  latestCommittedAt: string | null
  builtRelease: string | null
  imageReady: boolean | null
  commitsBehind: number | null
  message: string | null
  hasSource: boolean
  retry: () => void
}

function normalizeState(value: string): AppDriftState {
  switch (value) {
    case "up_to_date":
    case "behind_built":
    case "behind_unbuilt":
    case "not_deployed":
    case "live_unbuilt":
    case "no_source":
      return value
    default:
      return "unknown"
  }
}

function branchLabel(source: string | null | undefined, fallback: string): string {
  if (source === "HEAD") return "the checkout"
  if (source?.startsWith("origin/")) return source.slice("origin/".length)
  return fallback
}

/**
 * One read model for the app header badge and the version card.
 *
 * `GET /api/apps/{name}/drift` is the authority. When it is absent (an older
 * API returns 404) or fails, the existing `/latest`, image and status reads
 * are used to keep the card informative, but the state stays `unknown` so the
 * UI never claims "up to date" without a successful drift response.
 */
export function useVersionDrift(app: App): VersionDriftView {
  const hasSource = app.repo != null || app.build_repo != null

  const driftQuery = useQuery({
    queryKey: queryKeys.drift(app.name),
    queryFn: ({ signal }) => api.appDrift(app.name, signal),
    staleTime: 60_000,
    retry: 0,
  })

  // The fallback reads only start once drift is known to be unavailable, so a
  // healthy API never pays for three extra requests per app page.
  const fallback = driftQuery.isError
  const latestQuery = useQuery({
    queryKey: queryKeys.latest(app.name),
    queryFn: ({ signal }) => api.appLatest(app.name, signal),
    enabled: fallback && hasSource,
    staleTime: 60_000,
    retry: 0,
  })
  const imagesQuery = useQuery({
    queryKey: queryKeys.images(app.name),
    queryFn: ({ signal }) => api.appImages(app.name, signal),
    enabled: fallback && app.kind === "container" && app.image_repo != null,
    staleTime: 60_000,
    retry: 0,
  })
  const health = useAppHealth(app.name, app.live_port)

  const drift: AppDrift | undefined = driftQuery.data
  const fallbackImages = imagesQuery.data?.images
  const liveRelease = app.release ?? drift?.live?.release ?? null
  const latestRelease = drift?.latest?.release ?? latestQuery.data?.release ?? null
  const latestSource = drift?.latest?.source ?? latestQuery.data?.source ?? null
  const latestBranch = branchLabel(latestSource, app.branch ?? "main")
  const built = drift?.built
  const fallbackBuilt = imageForRelease(fallbackImages ?? [], latestRelease)
  const builtRelease = built ? (built.release ?? built.tag ?? null) : (fallbackBuilt?.tag ?? null)

  let state: CardState
  if (drift) {
    state = normalizeState(drift.state)
  } else if (fallback) {
    if (!hasSource) state = "no_source"
    else if (liveRelease == null) state = "not_deployed"
    else state = "unknown"
  } else {
    state = "unknown"
  }

  let imageReady: boolean | null = null
  if (drift) {
    if (built) imageReady = true
    else if (state === "behind_unbuilt" || state === "live_unbuilt") imageReady = false
  } else if (imagesQuery.isSuccess) {
    imageReady = latestRelease != null ? Boolean(fallbackBuilt) : null
  }

  const liveUp =
    health.data != null
      ? isAppUp(health.data)
      : drift?.live?.up != null
        ? drift.live.up
        : null

  const loading =
    driftQuery.isPending ||
    (fallback && (hasSource ? latestQuery.isPending : false)) ||
    (fallback && app.kind === "container" && app.image_repo != null && imagesQuery.isPending)

  const retry = () => {
    void driftQuery.refetch()
    if (fallback) {
      if (hasSource) void latestQuery.refetch()
      if (app.kind === "container" && app.image_repo != null) void imagesQuery.refetch()
    }
  }

  return {
    state,
    authoritative: drift != null,
    loading,
    checking: driftQuery.isFetching,
    error: driftQuery.error,
    liveRelease,
    livePort: app.live_port,
    liveUp,
    latestRelease,
    latestSource,
    latestBranch,
    latestSubject: drift?.latest?.subject ?? null,
    latestCommittedAt: drift?.latest?.committed_at ?? null,
    builtRelease,
    imageReady,
    commitsBehind: drift?.commits_behind ?? null,
    message: drift?.message ?? null,
    hasSource,
    retry,
  }
}
