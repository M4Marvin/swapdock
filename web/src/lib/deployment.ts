import type { App } from "@/api"

/**
 * Where a deployable version comes from.
 *
 * This is the fact the UI must respect: a commit SHA only exists for apps
 * built from a source checkout. Prebuilt images, compose-only apps and static
 * roots are first-class too.
 */
export type DeploymentKind = "static" | "source" | "image" | "compose"

export function deploymentKind(app: App): DeploymentKind {
  if (app.kind === "static") return "static"
  if (app.repo != null || app.git_remote != null) return "source"
  if (app.image_repo != null) return "image"
  return "compose"
}

/**
 * True when a normal deploy is not the next move yet.
 *
 * `replace` needs an existing live port: the backend refuses a first replace
 * deploy. An image/compose app has no source version to deploy either. In both
 * cases the running legacy container must be taken over first, which is what
 * adoption is for.
 */
export function needsAdoption(app: App): boolean {
  if (app.kind !== "container" || app.live_port != null) return false
  return app.strategy === "replace" || deploymentKind(app) !== "source"
}

/**
 * A hex release label for a snapshot of the currently running image.
 *
 * The deploy system only uses a release as an image tag. For an app that was
 * never built by swapdock there is no commit to record, so adoption preserves
 * the running image under a generated label instead. The operator never sees
 * or types it; it is a snapshot identity, not a claim that the hash is a commit.
 */
export function snapshotRelease(now = Date.now()): string {
  const seconds = Math.floor(now / 1000).toString(16)
  const salt = Math.floor(Math.random() * 0xffff)
    .toString(16)
    .padStart(4, "0")
  return `${seconds}${salt}`
}

/** The repository part of a Docker image reference, without tag or digest. */
export function imageRepository(ref: string | null | undefined): string | null {
  if (!ref) return null
  const withoutDigest = ref.split("@")[0]
  // An untagged image inspects as `sha256:<id>`; that is an image ID, not a repo.
  if (withoutDigest.startsWith("sha256:")) return null
  const colon = withoutDigest.lastIndexOf(":")
  const slash = withoutDigest.lastIndexOf("/")
  return colon > slash ? withoutDigest.slice(0, colon) : withoutDigest
}
