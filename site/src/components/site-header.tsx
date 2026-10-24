import { Link } from '@tanstack/react-router'
import { Menu } from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/components/ui/button'
import {
  Sheet,
  SheetContent,
  SheetHeader,
  SheetTitle,
} from '@/components/ui/sheet'
import { GithubIcon } from '@/components/github-icon'
import { Logo } from '@/components/logo'
import { ThemeToggle } from '@/components/theme-toggle'
import { NAV, REPO } from '@/lib/site'

export function SiteHeader() {
  const [open, setOpen] = useState(false)

  return (
    <header className="sticky top-0 z-50 border-b border-border/80 bg-background/80 backdrop-blur-xl">
      <div className="container-page flex h-16 items-center gap-6">
        <Link to="/" aria-label="swapdock home" className="shrink-0">
          <Logo />
        </Link>

        <nav className="hidden items-center gap-0.5 md:flex" aria-label="Primary">
          {NAV.map((item) => (
            <Link
              key={item.to}
              to={item.to}
              activeProps={{ className: 'text-foreground' }}
              className="rounded-md px-3 py-2 text-[13.5px] font-medium text-muted-foreground transition-colors hover:text-foreground"
            >
              {item.label}
            </Link>
          ))}
        </nav>

        <div className="ml-auto flex items-center gap-1">
          <a
            href={REPO}
            target="_blank"
            rel="noreferrer"
            aria-label="swapdock on GitHub"
            className="inline-flex size-9 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
          >
            <GithubIcon className="size-4" />
          </a>
          <ThemeToggle />
          <Button
            size="sm"
            className="ml-1 hidden h-9 rounded-lg px-4 text-[13px] sm:inline-flex"
            render={<Link to="/path/" />}
          >
            Get started
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="md:hidden"
            aria-label="Open menu"
            onClick={() => setOpen(true)}
          >
            <Menu className="size-5" />
          </Button>
        </div>
      </div>

      <Sheet open={open} onOpenChange={setOpen}>
        <SheetContent side="right" className="w-full max-w-sm bg-background">
          <SheetHeader className="px-6 pt-6">
            <SheetTitle className="sr-only">Menu</SheetTitle>
            <Logo />
          </SheetHeader>
          <nav className="flex flex-col gap-1 px-4" aria-label="Mobile">
            {NAV.map((item) => (
              <Link
                key={item.to}
                to={item.to}
                onClick={() => setOpen(false)}
                activeProps={{ className: 'text-foreground bg-muted' }}
                className="rounded-lg px-3 py-2.5 text-[15px] font-medium text-muted-foreground"
              >
                {item.label}
              </Link>
            ))}
          </nav>
          <div className="mt-auto flex flex-col gap-2 p-4">
            <Button
              className="h-11 rounded-lg"
              render={<Link to="/path/" onClick={() => setOpen(false)} />}
            >
              Get started
            </Button>
            <Button
              variant="outline"
              className="h-11 rounded-lg"
              render={<a href={REPO} target="_blank" rel="noreferrer" />}
            >
              <GithubIcon className="size-4" /> GitHub
            </Button>
          </div>
        </SheetContent>
      </Sheet>
    </header>
  )
}
