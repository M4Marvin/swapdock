import { QueryClient } from '@tanstack/react-query'

/**
 * The app-wide QueryClient.
 *
 * Responses stay fresh for ten seconds, then refetch on window focus — long
 * enough that switching tabs does not hammer the API, short enough that a
 * deploy made elsewhere shows up without a manual reload.
 */
export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 10_000,
      refetchOnWindowFocus: true,
    },
  },
})
