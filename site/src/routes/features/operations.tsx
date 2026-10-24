import { createFileRoute } from '@tanstack/react-router'
import { FeaturePage } from '@/components/marketing/feature-page'
import { FEATURE_DETAILS } from '@/content/features'
import { pageHead } from '@/lib/seo'

const detail = FEATURE_DETAILS['operations']

export const Route = createFileRoute('/features/operations')({
  head: () =>
    pageHead({
      title: 'Day-two operations — swapdock',
      description: 'verify, monitor, prune and doctor: the day-two tools around a swapdock estate.',
      path: '/features/operations',
    }),
  component: () => <FeaturePage detail={detail} />,
})
