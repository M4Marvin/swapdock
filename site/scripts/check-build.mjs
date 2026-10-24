import fs from 'node:fs'
import path from 'node:path'

const out = path.join(process.cwd(), 'dist/client')
const routes = [
  '/', '/why', '/path', '/how-it-works', '/features',
  '/features/zero-downtime', '/features/stateful-apps', '/features/recovery',
  '/features/operations', '/docs', '/security', '/faq', '/404',
]

let failed = 0
for (const route of routes) {
  const file = path.join(out, route === '/' ? 'index.html' : `${route.replace(/^\//, '')}/index.html`)
  if (!fs.existsSync(file)) {
    console.error(`missing: ${route} (${file})`)
    failed++
    continue
  }
  const html = fs.readFileSync(file, 'utf8')
  const checks = {
    title: /<title>[^<]{8,}<\/title>/.test(html),
    description: /<meta name="description" content="[^"]{20,}"/.test(html),
    h1: /<h1[^>]*>/.test(html),
    canonical: /rel="canonical" href="https?:\/\//.test(html),
    content: html.length > (route === '/404' ? 12000 : 20000),
  }
  const bad = Object.entries(checks).filter(([, ok]) => !ok).map(([k]) => k)
  if (bad.length) {
    console.error(`${route}: ${bad.join(', ')}`)
    failed++
  }
}
for (const extra of ['sitemap.xml', 'robots.txt', 'og.png', 'favicon.svg']) {
  if (!fs.existsSync(path.join(out, extra))) {
    console.error(`missing asset: ${extra}`)
    failed++
  }
}
if (failed) {
  console.error(`${failed} check(s) failed`)
  process.exit(1)
}
console.log(`check: ${routes.length} routes + assets OK`)
