import { Outlet } from "@tanstack/react-router"
import { useQuery, useQueryClient } from "@tanstack/react-query"
import { api } from "@/api"
import { queryKeys } from "@/lib/query-keys"
import { AppShell } from "@/components/shell/AppShell"

/** The authenticated frame. Mounted only while the tab holds a token. */
export function AuthedShell() {
  const queryClient = useQueryClient()

  const appsQuery = useQuery({
    queryKey: queryKeys.apps,
    queryFn: ({ signal }) => api.apps(signal),
    refetchInterval: 60_000,
    placeholderData: (previous) => previous,
  })

  const healthQuery = useQuery({
    queryKey: queryKeys.health,
    queryFn: ({ signal }) => api.health(signal),
    refetchInterval: 30_000,
    retry: 1,
  })

  const apiStatus: 'online' | 'offline' | 'checking' = healthQuery.isSuccess
    ? 'online'
    : healthQuery.isError && appsQuery.isSuccess
      ? 'online'
      : healthQuery.isError && appsQuery.isError
        ? 'offline'
        : 'checking'

  const retry = () => {
    void queryClient.invalidateQueries({ queryKey: queryKeys.apps })
    void queryClient.invalidateQueries({ queryKey: queryKeys.health })
  }

  return (
    <AppShell
      apps={appsQuery.data ?? []}
      appsLoading={appsQuery.isPending && appsQuery.data == null}
      appsError={appsQuery.isError ? appsQuery.error : null}
      appsUpdatedAt={appsQuery.dataUpdatedAt}
      apiStatus={apiStatus}
      onRetry={retry}
      retrying={appsQuery.isFetching && appsQuery.isError}
    >
      <Outlet />
    </AppShell>
  )
}

