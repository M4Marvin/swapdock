import { SITE_URL } from './site'

type HeadInput = {
  title: string
  description: string
  path: string
  type?: 'website' | 'article'
}

export function pageHead({ title, description, path, type = 'website' }: HeadInput) {
  const suffix = path === '/' ? '' : `${path.replace(/\/$/, '')}/`
  const url = `${SITE_URL}${suffix}`
  return {
    meta: [
      { title },
      { name: 'description', content: description },
      { property: 'og:type', content: type },
      { property: 'og:title', content: title },
      { property: 'og:description', content: description },
      { property: 'og:url', content: url },
      { property: 'og:image', content: `${SITE_URL}/og.png` },
      { property: 'og:site_name', content: 'swapdock' },
      { name: 'twitter:card', content: 'summary_large_image' },
      { name: 'twitter:title', content: title },
      { name: 'twitter:description', content: description },
      { name: 'twitter:image', content: `${SITE_URL}/og.png` },
    ],
    links: [{ rel: 'canonical', href: url }],
  }
}

export function jsonLd(data: Record<string, unknown>) {
  return JSON.stringify(data).replace(/</g, '\\u003c')
}
