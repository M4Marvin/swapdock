import { createRootRoute, Link, Outlet } from '@tanstack/react-router'
import { Toaster } from '@/components/ui/sonner'

export const Route = createRootRoute({
  component: () => (
    <div className="min-h-screen bg-background text-foreground">
      <header className="flex items-center gap-6 border-b px-6 py-3">
        <strong className="text-lg">swapdock</strong>
        <nav className="flex gap-4 text-sm">
          <Link to="/" className="[&.active]:font-semibold [&.active]:underline">
            Apps
          </Link>
          <Link to="/runs" className="[&.active]:font-semibold [&.active]:underline">
            Runs
          </Link>
        </nav>
      </header>
      <main className="p-6">
        <Outlet />
      </main>
      <Toaster richColors position="bottom-right" />
    </div>
  ),
})
