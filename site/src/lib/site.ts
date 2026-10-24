export const SITE_URL = (import.meta.env.VITE_SITE_URL as string | undefined) ?? 'https://swapdock.dev'

export const REPO = 'https://github.com/M4Marvin/swapdock'
export const DOCS = `${REPO}/tree/main/docs`
export const LAUNCH = `${REPO}/blob/main/LAUNCH.md`

export const NAV = [
  { to: '/why/', label: 'Why' },
  { to: '/path/', label: 'Path' },
  { to: '/how-it-works/', label: 'How it works' },
  { to: '/features/', label: 'Features' },
  { to: '/docs/', label: 'Docs' },
] as const

export const FOOTER = [
  {
    title: 'Product',
    links: [
      { to: '/why/', label: 'Why swapdock' },
      { to: '/path/', label: 'The path' },
      { to: '/how-it-works/', label: 'How it works' },
      { to: '/features/', label: 'Features' },
    ],
  },
  {
    title: 'Resources',
    links: [
      { to: '/docs/', label: 'Docs & quickstart' },
      { to: '/security/', label: 'Security' },
      { to: '/faq/', label: 'FAQ' },
      { href: REPO, label: 'GitHub' },
    ],
  },
  {
    title: 'Project',
    links: [
      { href: `${REPO}/blob/main/CHANGELOG.md`, label: 'Changelog' },
      { href: LAUNCH, label: 'Launch gates' },
      { href: `${REPO}/blob/main/LICENSE-MIT`, label: 'MIT license' },
      { href: `${REPO}/blob/main/SECURITY.md`, label: 'Report a vulnerability' },
    ],
  },
] as const
