import { useQuery } from '@tanstack/react-query'
import { api, type AppStatus } from '../api'
import { queryKeys } from '@/lib/query-keys'
import { Badge } from '@/components/ui/badge'
import { Skeleton } from '@/components/ui/skeleton'

/** The API computes `up`, but fall back to the 2xx code when it is absent. */
function isUp(status: AppStatus): boolean {
  if (status.up === true) return true
  return (
    typeof status.status === 'number' &&
    status.status >= 200 &&
    status.status < 300
  )
}

/**
 * Live health verdict for one app's live port.
 *
 * The probe polls every fifteen seconds while the pill is mounted. Apps with
 * no live port render "not deployed" without a request, and a failed probe
 * renders "unknown" rather than pretending the app is down.
 */
export function StatusPill({
  name,
  livePort,
  showCode = false,
}: {
  name: string
  livePort: number | null
  /** Include the HTTP status code in the label (app detail view). */
  showCode?: boolean
}) {
  const { data, isError, isLoading } = useQuery({
    queryKey: queryKeys.appStatus(name),
    queryFn: ({ signal }) => api.appStatus(name, signal),
    enabled: livePort != null,
    refetchInterval: 15_000,
  })

  // No live port means nothing was ever deployed: not the same as "down".
  if (livePort == null) {
    return (
      <Badge variant="outline" className="text-muted-foreground">
        not deployed
      </Badge>
    )
  }
  if (isLoading) return <Skeleton className="h-5 w-12 rounded-4xl" />
  if (isError || data === undefined) {
    return (
      <Badge variant="warning" title="health probe failed">
        unknown
      </Badge>
    )
  }
  const code =
    showCode && typeof data.status === 'number' ? ` ${data.status}` : ''
  return isUp(data) ? (
    <Badge variant="success">up{code}</Badge>
  ) : (
    <Badge variant="destructive">down{code}</Badge>
  )
}
