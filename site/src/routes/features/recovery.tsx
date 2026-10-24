import { createFileRoute } from '@tanstack/react-router'
import { FeaturePage } from '@/components/marketing/feature-page'
import { FEATURE_DETAILS } from '@/content/features'
import { pageHead } from '@/lib/seo'

const detail = FEATURE_DETAILS['recovery']

export const Route = createFileRoute('/features/recovery')({
  head: () =>
    pageHead({
      title: 'Crash recovery — swapdock',
      description: 'A fsynced journal and reconcile that proves nginx, the image id and health before it advances the registry.',
      path: '/features/recovery',
    }),
  component: () => <FeaturePage detail={detail} />,
})
