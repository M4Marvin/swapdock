import { createFileRoute } from '@tanstack/react-router'
import { NotFound } from '@/components/not-found'
import { pageHead } from '@/lib/seo'

export const Route = createFileRoute('/404')({
  head: () =>
    pageHead({
      title: '404 — swapdock',
      description: 'That page does not exist. Head back to the front door.',
      path: '/404',
    }),
  component: NotFound,
})
