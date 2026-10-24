import { createFileRoute } from '@tanstack/react-router'
import { FeaturePage } from '@/components/marketing/feature-page'
import { FEATURE_DETAILS } from '@/content/features'
import { pageHead } from '@/lib/seo'

const detail = FEATURE_DETAILS['zero-downtime']

export const Route = createFileRoute('/features/zero-downtime')({
  head: () =>
    pageHead({
      title: 'Zero-downtime cutovers — swapdock',
      description: 'How swapdock starts a new release on the free port, gates health, and flips nginx without a maintenance window.',
      path: '/features/zero-downtime',
    }),
  component: () => <FeaturePage detail={detail} />,
})
