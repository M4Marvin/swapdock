import { createFileRoute } from '@tanstack/react-router'
import { FeaturePage } from '@/components/marketing/feature-page'
import { FEATURE_DETAILS } from '@/content/features'
import { pageHead } from '@/lib/seo'

const detail = FEATURE_DETAILS['stateful-apps']

export const Route = createFileRoute('/features/stateful-apps')({
  head: () =>
    pageHead({
      title: 'SQLite-safe stateful deploys — swapdock',
      description: 'Replace deploys enforced by the registry, a pre-deploy backup hook, and a validator that refuses unsafe swaps.',
      path: '/features/stateful-apps',
    }),
  component: () => <FeaturePage detail={detail} />,
})
