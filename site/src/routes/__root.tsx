import {
  HeadContent,
  Link,
  Scripts,
  createRootRoute,
} from '@tanstack/react-router'
import { useEffect, type ReactNode } from 'react'
import { SiteHeader } from '@/components/site-header'
import { SiteFooter } from '@/components/site-footer'
import { Button } from '@/components/ui/button'
import { NotFound } from '@/components/not-found'
import { jsonLd } from '@/lib/seo'
import appCss from '../styles.css?url'

const software = {
  '@context': 'https://schema.org',
  '@type': 'SoftwareApplication',
  name: 'swapdock',
  applicationCategory: 'DeveloperApplication',
  operatingSystem: 'Linux',
  description:
    'Blue/green deploys, health-gated cutovers and one-command rollbacks for a single Docker host behind nginx.',
  offers: { '@type': 'Offer', price: '0', priceCurrency: 'USD' },
  license: 'https://github.com/M4Marvin/swapdock/blob/main/LICENSE-MIT',
  url: 'https://swapdock.dev',
}

export const Route = createRootRoute({
  head: () => ({
    meta: [
      { charSet: 'utf-8' },
      { name: 'viewport', content: 'width=device-width, initial-scale=1' },
      { title: 'swapdock — blue/green deploys for one box' },
      {
        name: 'description',
        content:
          'Blue/green deploys, health-gated cutovers and one-command rollbacks for a single Linux host. Built on Docker Compose and nginx. Open source, MIT.',
      },
      { name: 'theme-color', content: '#08090a' },
      { name: 'color-scheme', content: 'dark light' },
    ],
    links: [
      { rel: 'stylesheet', href: appCss },
      { rel: 'icon', type: 'image/svg+xml', href: '/favicon.svg' },
      { rel: 'preload', href: '/fonts/geist-mono-latin-wght-normal.woff2', as: 'font', type: 'font/woff2', crossorigin: 'anonymous' },
    ],
  }),
  shellComponent: RootDocument,
  notFoundComponent: NotFound,
  errorComponent: ({ error, reset }) => (
    <main className="container-page flex min-h-[60vh] flex-col items-center justify-center py-24 text-center">
      <p className="label-eyebrow">Unexpected error</p>
      <h1 className="display-3 mt-4">This view stopped rendering.</h1>
      <p className="mt-3 max-w-md text-sm text-muted-foreground">
        {error instanceof Error ? error.message : String(error)}
      </p>
      <div className="mt-6 flex gap-2">
        <Button onClick={() => reset()}>Try again</Button>
        <Button variant="outline" render={<Link to="/" />}>
          Back home
        </Button>
      </div>
    </main>
  ),
})

function RootDocument({ children }: { children: ReactNode }) {
  useEffect(() => {
    document.documentElement.classList.add('js')
  }, [])

  const themeScript = `(function(){try{var t=localStorage.getItem('swapdock-site-theme');if(t==='light'){document.documentElement.classList.remove('dark')}else{document.documentElement.classList.add('dark')}var m=document.querySelector('meta[name="theme-color"]');if(m)m.setAttribute('content',t==='light'?'#f7f7f4':'#08090a')}catch(e){document.documentElement.classList.add('dark')}})()`

  return (
    <html lang="en" className="dark" suppressHydrationWarning>
      <head>
        <script dangerouslySetInnerHTML={{ __html: themeScript }} />
        <HeadContent />
        <script
          type="application/ld+json"
          dangerouslySetInnerHTML={{ __html: jsonLd(software) }}
        />
      </head>
      <body className="grain min-h-dvh bg-background text-foreground antialiased">
        <a
          href="#main"
          className="sr-only z-[100] rounded-md bg-foreground px-4 py-2 text-background focus:not-sr-only focus:fixed focus:left-4 focus:top-4"
        >
          Skip to content
        </a>
        <SiteHeader />
        <main id="main">{children}</main>
        <SiteFooter />
        <Scripts />
      </body>
    </html>
  )
}
