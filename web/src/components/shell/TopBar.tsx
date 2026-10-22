import { useState } from "react"
import { Link } from "@tanstack/react-router"
import { useQueryClient } from "@tanstack/react-query"
import { ChevronRight, Menu, RefreshCw, Search } from "lucide-react"
import type { AppEntry } from "@/api"
import { shortId } from "@/lib/format"
import { cn } from "cn"
import { Kbd } from "@/components/kit"
import { useModKey } from "@/lib/hooks"
import { Button } from "@/components/ui/button"
import { ThemeMenu } from "@/components/shell/ThemeMenu"

function Breadcrumb({ pathname, apps }: { pathname: string; apps: AppEntry[] }) {
  const parts: React.ReactNode[] = []
  const appName = pathname.startsWith("/apps/") ? decodeURIComponent(pathname.split("/")[2] ?? "") : null
  const app = appName ? apps.find((entry) => entry.app.name === appName) : null
  const runId = pathname.startsWith("/runs/") ? pathname.split("/")[2] : null

  if (appName) {
    parts.push(
      <Link
        key="fleet"
        to="/"
        className="rounded-sm text-muted-foreground transition-colors hover:text-foreground"
      >
        Fleet
      </Link>,
      <ChevronRight key="sep" className="size-3 shrink-0 text-subtle-foreground/60" />,
      <span key="app" className="truncate font-medium text-foreground">
        {app?.app.name ?? appName}
      </span>
    )
  } else if (pathname === "/runs") {
    parts.push(
      <span key="runs" className="font-medium text-foreground">
        Runs
      </span>
    )
  } else if (runId) {
    parts.push(
      <Link
        key="runs"
        to="/runs"
        className="rounded-sm text-muted-foreground transition-colors hover:text-foreground"
      >
        Runs
      </Link>,
      <ChevronRight key="sep" className="size-3 shrink-0 text-subtle-foreground/60" />,
      <span key="run" className="font-mono text-xs font-medium text-foreground">
        {shortId(runId)}
      </span>
    )
  } else if (pathname === "/doctor") {
    parts.push(
      <span key="doctor" className="font-medium text-foreground">
        Doctor
      </span>
    )
  } else {
    parts.push(
      <span key="fleet" className="font-medium text-foreground">
        Fleet
      </span>
    )
  }

  return (
    <nav aria-label="Breadcrumb" className="flex min-w-0 items-center gap-1.5 text-sm">
      {parts}
    </nav>
  )
}

export function TopBar({
  pathname,
  apps,
  onOpenPalette,
  onOpenMobileNav,
}: {
  pathname: string
  apps: AppEntry[]
  onOpenPalette: () => void
  onOpenMobileNav: () => void
}) {
  const queryClient = useQueryClient()
  const modKey = useModKey()
  const [refreshing, setRefreshing] = useState(false)

  const refresh = async () => {
    if (refreshing) return
    setRefreshing(true)
    try {
      await queryClient.invalidateQueries()
    } finally {
      setRefreshing(false)
    }
  }

  return (
    <header className="sticky top-0 z-30 flex h-14 items-center gap-3 border-b border-border bg-[var(--header)] px-4 backdrop-blur-xl sm:px-6 lg:px-8">
      <Button
        type="button"
        variant="ghost"
        size="icon-sm"
        className="-ml-1 lg:hidden"
        onClick={onOpenMobileNav}
        aria-label="Open navigation"
      >
        <Menu />
      </Button>

      <Breadcrumb pathname={pathname} apps={apps} />

      <div className="ml-auto flex shrink-0 items-center gap-1.5">
        <button
          type="button"
          onClick={onOpenPalette}
          className={cn(
            "hidden h-8 w-56 items-center gap-2 rounded-lg border border-border bg-surface/70 px-2.5 text-left text-xs text-subtle-foreground transition-[border-color,background-color,color] duration-150 hover:border-border-strong hover:text-muted-foreground md:flex xl:w-64"
          )}
          aria-label="Open command menu"
        >
          <Search className="size-3.5 shrink-0" />
          <span className="truncate">Search or jump to…</span>
          <Kbd className="ml-auto border-transparent bg-transparent px-1 text-[10px]">
            {modKey} K
          </Kbd>
        </button>

        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          className="md:hidden"
          aria-label="Search"
          aria-keyshortcuts="Control+K Meta+K"
          onClick={onOpenPalette}
        >
          <Search />
        </Button>

        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          aria-label="Refresh all data"
          title="Refresh all data"
          disabled={refreshing}
          onClick={() => void refresh()}
        >
          <RefreshCw className={refreshing ? "animate-spin" : undefined} />
        </Button>

        <span aria-hidden="true" className="mx-0.5 h-4 w-px bg-border" />

        <ThemeMenu />
      </div>
    </header>
  )
}
