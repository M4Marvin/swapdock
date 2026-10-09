import { classifyStatus, formatStatus } from '../api'
import { Badge } from '@/components/ui/badge'

/**
 * Colored pill for a run or step status.
 *
 * The color comes from `classifyStatus` (a closed set), not a substring, so a
 * failed run is red and an ok run is green. A run that succeeded overall but
 * had a failed step is shown as ok plus a separate `non-ok` badge at the call
 * site, so the label never contradicts the color.
 */
export function RunStatusBadge({ status }: { status?: string | null }) {
  const label = status ? formatStatus(status) : '—'
  switch (classifyStatus(status)) {
    case 'ok':
      return <Badge variant="success">{label}</Badge>
    case 'failed':
      return <Badge variant="destructive">{label}</Badge>
    case 'dry':
      return <Badge variant="secondary">dry run</Badge>
    case 'interrupted':
      return <Badge variant="warning">{label}</Badge>
    default:
      return <Badge variant="outline">{label}</Badge>
  }
}
