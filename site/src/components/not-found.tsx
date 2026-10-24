import { Link } from '@tanstack/react-router'
import { Button } from '@/components/ui/button'

export function NotFound() {
  return (
    <main className="container-page flex min-h-[70vh] flex-col items-center justify-center py-24 text-center">
      <p className="mono text-[11px] tracking-[0.2em] text-subtle-foreground">404</p>
      <h1 className="display-2 mt-4">This page didn’t deploy.</h1>
      <p className="mt-4 max-w-md text-[14px] leading-relaxed text-muted-foreground">
        The URL does not match a route in this build. The front door is fine; the path is not.
      </p>
      <div className="mt-7 flex flex-wrap justify-center gap-2">
        <Button render={<Link to="/" />}>Back home</Button>
        <Button variant="outline" render={<Link to="/path/" />}>
          Start the path
        </Button>
      </div>
    </main>
  )
}
