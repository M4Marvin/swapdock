import type { AppImage } from "@/api"

/** A new release is a commit name: hex, 7–64 characters. */
export const RELEASE_RE = /^[0-9a-fA-F]{7,64}$/

/**
 * Older estate tags may be 4–6 characters. They stay visible (display-safe)
 * even though the server now refuses to create new releases shorter than 7.
 */
export const RELEASE_TAG_RE = /^[0-9a-fA-F]{4,64}$/

/** Docker reports `YYYY-MM-DD HH:MM:SS +0000 UTC`; Date.parse needs the suffix trimmed. */
export function dockerTime(created: string | null | undefined): number | null {
  if (!created) return null
  const parsed = Date.parse(created.replace(" UTC", ""))
  return Number.isNaN(parsed) ? null : parsed
}

/** True when two commit names describe the same release, short or full. */
export function sameRelease(
  a: string | null | undefined,
  b: string | null | undefined
): boolean {
  if (!a || !b) return false
  return a === b || a.startsWith(b) || b.startsWith(a)
}

/** True when a value can be deployed as a newly-created release. */
export function isRelease(value: string): boolean {
  return RELEASE_RE.test(value)
}

/** True when a tag is release-shaped for display, including older 4–6 char tags. */
export function isReleaseTag(value: string): boolean {
  return RELEASE_TAG_RE.test(value)
}

function createdRank(image: AppImage, index: number): number {
  return dockerTime(image.created) ?? index
}

/**
 * Release images only: `latest` and `rollback-pre-*` are bookkeeping tags, not
 * deployable versions. Older 4–6 char tags remain visible. Newest first.
 */
export function releaseImages(images: AppImage[]): AppImage[] {
  return images
    .filter((image) => RELEASE_TAG_RE.test(image.tag))
    .sort((a, b) => createdRank(b, images.indexOf(b)) - createdRank(a, images.indexOf(a)))
}

export function newestReleaseImage(images: AppImage[]): AppImage | null {
  return releaseImages(images)[0] ?? null
}

/** The built image for a release, or undefined when it has never been built. */
export function imageForRelease(
  images: AppImage[],
  release: string | null | undefined
): AppImage | undefined {
  if (!release) return undefined
  return images.find(
    (image) =>
      image.tag === release ||
      image.tag.startsWith(release) ||
      release.startsWith(image.tag)
  )
}
