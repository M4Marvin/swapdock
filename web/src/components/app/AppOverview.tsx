import { useMemo } from "react"
import { useQuery } from "@tanstack/react-query"
import { api, type App, type Problem } from "@/api"
import { queryKeys } from "@/lib/query-keys"
import { DeploymentCard } from "@/components/app/DeploymentCard"
import { VersionDriftCard } from "@/components/app/VersionDrift"
import { TrafficCard } from "@/components/app/TrafficCard"
import { SourceCard } from "@/components/app/SourceCard"
import { RecentRunsCard } from "@/components/app/RecentRunsCard"
import { ProblemsCard } from "@/components/app/ProblemsCard"
import { AdoptionRequired } from "@/components/app/DeploymentModes"
import { needsAdoption } from "@/lib/deployment"

/**
 * The app's "what is true right now" view: the version, the blue/green
 * topology, the nginx verdict, the source and image state, and the last runs.
 */
export function AppOverview({
  app,
  problems,
  onAdopt,
  onDeploy,
  onBuild,
  onOpenPipeline,
}: {
  app: App
  problems: Problem[]
  onAdopt: () => void
  onDeploy: (release: string) => void
  onBuild: (release: string) => void
  onOpenPipeline: () => void
}) {
  if (needsAdoption(app)) {
    return (
      <div className="space-y-4">
        <VersionDriftCard
          app={app}
          onDeploy={onDeploy}
          onBuild={onBuild}
          onOpenPipeline={onOpenPipeline}
        />
        <AdoptionRequired app={app} onAdopt={onAdopt} />
      </div>
    )
  }
  return (
    <ManagedOverview
      app={app}
      problems={problems}
      onDeploy={onDeploy}
      onBuild={onBuild}
      onOpenPipeline={onOpenPipeline}
    />
  )
}

/** Hooks live in their own component so the adoption view never changes hook order. */
function ManagedOverview({
  app,
  problems,
  onDeploy,
  onBuild,
  onOpenPipeline,
}: {
  app: App
  problems: Problem[]
  onDeploy: (release: string) => void
  onBuild: (release: string) => void
  onOpenPipeline: () => void
}) {
  const runsQuery = useQuery({
    queryKey: queryKeys.runs,
    queryFn: ({ signal }) => api.runs(50, signal),
    refetchInterval: 30_000,
    placeholderData: (previous) => previous,
  })

  const appRuns = useMemo(
    () => (runsQuery.data ?? []).filter((run) => run.app === app.name),
    [runsQuery.data, app.name]
  )

  return (
    <div className="space-y-4">
      <VersionDriftCard
        app={app}
        onDeploy={onDeploy}
        onBuild={onBuild}
        onOpenPipeline={onOpenPipeline}
      />
      <DeploymentCard app={app} />
      <ProblemsCard app={app} problems={problems} />
      <div className="grid gap-4 xl:grid-cols-3">
        <TrafficCard app={app} latestRun={appRuns[0]} />
        <SourceCard app={app} />
        <RecentRunsCard
          appName={app.name}
          runs={appRuns.slice(0, 6)}
          loading={runsQuery.isPending}
        />
      </div>
    </div>
  )
}
