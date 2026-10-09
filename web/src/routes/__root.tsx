import { createRootRoute, Link, Outlet } from '@tanstack/react-router'
import { Toaster } from '@/components/ui/sonner'

export const Route = createRootRoute({
  component: () => (
    <div className="min-h-screen bg-background text-foreground">
      <header className="flex items-center gap-6 border-b px-6 py-3">
        <strong className="text-lg">swapdock</strong>
        <nav className="flex items-center gap-1 text-sm">
          <Link
            to="/"
            className="rounded-md px-2.5 py-1.5 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground [&.active]:bg-muted [&.active]:font-medium [&.active]:text-foreground"
          >
            Apps
          </Link>
          <Link
            to="/runs"
            className="rounded-md px-2.5 py-1.5 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground [&.active]:bg-muted [&.active]:font-medium [&.active]:text-foreground"
          >
            Runs
          </Link>
        </nav>
      </header>
      <main className="mx-auto w-full max-w-[1600px] p-6">
        <Outlet />
      </main>
      <Toaster position="bottom-right" />
    </div>
  ),
})
