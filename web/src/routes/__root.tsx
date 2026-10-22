import { createRootRoute, Link } from '@tanstack/react-router'
import { useAuthState } from '@/lib/auth'
import { TokenGate } from '@/components/shell/TokenGate'
import { AuthedShell } from '@/components/shell/AuthedShell'
import { Button } from '@/components/ui/button'

/** Centered fallback for unknown URLs and unexpected render errors. */
function CenteredMessage({
  eyebrow,
  title,
  description,
  children,
}: {
  eyebrow: string
  title: string
  description: React.ReactNode
  children?: React.ReactNode
}) {
  return (
    <main className="grid min-h-dvh place-items-center px-6">
      <div className="w-full max-w-sm text-center">
        <div className="label-eyebrow">{eyebrow}</div>
        <h1 className="mt-2 text-2xl font-semibold tracking-[-0.028em] text-foreground">{title}</h1>
        <p className="mt-2 text-pretty text-sm leading-relaxed text-muted-foreground">{description}</p>
        <div className="mt-5 flex items-center justify-center gap-2">{children}</div>
      </div>
    </main>
  )
}

function RootComponent() {
  const auth = useAuthState()
  if (auth.mode == null) return <TokenGate />
  return <AuthedShell />
}

export const Route = createRootRoute({
  component: RootComponent,
  notFoundComponent: () => (
    <CenteredMessage
      eyebrow="404"
      title="That page moved"
      description="The address does not match a view in this build. Head back to the fleet and pick an app."
    >
      <Button render={<Link to="/" />}>Back to the fleet</Button>
    </CenteredMessage>
  ),
  errorComponent: ({ error, reset }) => (
    <CenteredMessage
      eyebrow="Unexpected error"
      title="This view stopped rendering"
      description={error instanceof Error ? error.message : String(error)}
    >
      <Button variant="outline" onClick={() => reset()}>
        Try again
      </Button>
      <Button render={<Link to="/" />}>Back to the fleet</Button>
    </CenteredMessage>
  ),
})
