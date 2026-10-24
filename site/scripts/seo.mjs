import fs from 'node:fs'
import path from 'node:path'

const SITE_URL = (process.env.VITE_SITE_URL || 'https://swapdock.dev').replace(/\/$/, '')
const out = path.join(process.cwd(), 'dist/client')

const routes = [
  ['/', '1.0'],
  ['/why', '0.9'],
  ['/path', '1.0'],
  ['/how-it-works', '0.9'],
  ['/features', '0.9'],
  ['/features/zero-downtime', '0.8'],
  ['/features/stateful-apps', '0.8'],
  ['/features/recovery', '0.8'],
  ['/features/operations', '0.8'],
  ['/docs', '0.8'],
  ['/security', '0.6'],
  ['/faq', '0.6'],
]

const today = new Date().toISOString().slice(0, 10)
const urls = routes
  .map(
    ([route, priority]) => `  <url>
    <loc>${SITE_URL}${route === '/' ? '' : `${route.replace(/\/$/, '')}/`}</loc>
    <lastmod>${today}</lastmod>
    <changefreq>weekly</changefreq>
    <priority>${priority}</priority>
  </url>`,
  )
  .join('\n')

const sitemap = `<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
${urls}
</urlset>
`

fs.writeFileSync(path.join(out, 'sitemap.xml'), sitemap)
fs.writeFileSync(
  path.join(out, 'robots.txt'),
  `User-agent: *\nAllow: /\n\nSitemap: ${SITE_URL}/sitemap.xml\n`,
)
console.log(`seo: sitemap with ${routes.length} routes for ${SITE_URL}`)
