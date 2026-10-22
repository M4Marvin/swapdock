import { useEffect, useState } from "react"
import { ScrollRestoration, useRouterState } from "@tanstack/react-router"
import { useQuery } from "@tanstack/react-query"
import { Dialog } from "@base-ui/react/dialog"
import { cfgState, type AppEntry } from "@/api"
import { queryKeys } from "@/lib/query-keys"
import { errorText } from "@/lib/format"
import { isCommandK } from "@/lib/hooks"
import { cn } from "cn"
import { Toaster } from "@/components/ui/sonner"
import { Button } from "@/components/ui/button"
import { Callout } from "@/components/kit"
import { SidebarContent } from "@/components/shell/Sidebar"
import { TopBar } from "@/components/shell/TopBar"
import { CommandPalette } from "@/components/shell/CommandPalette"

function ConnectionBanner({
  error,
  onRetry,
  retrying,
}: {
  error: unknown
  onRetry: () => void
  retrying: boolean
}) {
  return (
    <Callout
      tone="danger"
      title="Can’t reach the registry"
      className="mb-5"
      action={
        <Button variant="outline" size="sm" onClick={onRetry} disabled={retrying}>
          {retrying ? "Retrying…" : "Retry"}
        </Button>
      }
    >
      {errorText(error)}. The shell stays usable, but deploys and checks need the
      API before they can act.
    </Callout>
  )
}

/**
 * `/config.json` missing or malformed is not fatal — same-origin is the safe
 * default — but a split build/deploy install silently degrading to one box is
 * exactly how a deploy lands on the wrong host. Say so.
 */
function ConfigBanner({ issue }: { issue: "missing" | "malformed" }) {
  return (
    <Callout tone="warning" title="The runtime configuration is incomplete" className="mb-5">
      {issue === "missing"
        ? "config.json was not found next to this build. "
        : "config.json could not be read. "}
      Same-origin API defaults are in use, which is safe when the console and API
      share a host. For a split install, set <code className="font-mono">buildApi</code>,{" "}
      <code className="font-mono">deployApi</code> and{" "}
      <code className="font-mono">transferTarget</code> in{" "}
      <code className="font-mono">web/public/config.json</code> and rebuild.
    </Callout>
  )
}

/**
 * The application frame: fixed sidebar, a quiet sticky top bar, one content
 * column, the command palette, and the mobile navigation drawer.
 *
 * The shell owns the single global keyboard shortcut (⌘K). Everything else is
 * scoped to a page.
 */
export function AppShell({
  children,
  apps,
  appsLoading,
  appsError,
  appsUpdatedAt,
  apiStatus,
  onRetry,
  retrying,
}: {
  children: React.ReactNode
  apps: AppEntry[]
  appsLoading: boolean
  appsError: unknown
  appsUpdatedAt: number
  apiStatus: "online" | "offline" | "checking"
  onRetry: () => void
  retrying: boolean
}) {
  const pathname = useRouterState({ select: (state) => state.location.pathname })
  const [paletteOpen, setPaletteOpen] = useState(false)
  const [mobileOpen, setMobileOpen] = useState(false)
  const configQuery = useQuery({
    queryKey: queryKeys.config,
    queryFn: () => cfgState(),
    staleTime: Infinity,
    retry: 0,
  })

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (isCommandK(event)) {
        event.preventDefault()
        setMobileOpen(false)
        setPaletteOpen((open) => !open)
      }
    }
    window.addEventListener("keydown", onKeyDown)
    return () => window.removeEventListener("keydown", onKeyDown)
  }, [])

  useEffect(() => {
    setPaletteOpen(false)
    setMobileOpen(false)
  }, [pathname])

  const sidebarProps = {
    apps,
    appsLoading,
    appsError: appsError != null,
    appsUpdatedAt,
    apiStatus,
    pathname,
    onOpenPalette: () => {
      setMobileOpen(false)
      setPaletteOpen(true)
    },
  }

  return (
    <div className="min-h-dvh bg-background">
      <a
        href="#main-content"
        className="sr-only focus:not-sr-only focus:fixed focus:top-3 focus:left-3 focus:z-100 focus:rounded-lg focus:border focus:border-border focus:bg-popover focus:px-3 focus:py-2 focus:text-sm focus:text-foreground focus:shadow-[var(--shadow-pop)]"
      >
        Skip to content
      </a>

      <aside className="fixed inset-y-0 left-0 z-30 hidden w-[248px] border-r border-sidebar-border lg:block">
        <SidebarContent {...sidebarProps} />
      </aside>

      <div className="lg:pl-[248px]">
        <TopBar
          pathname={pathname}
          apps={apps}
          onOpenPalette={() => {
            setMobileOpen(false)
            setPaletteOpen(true)
          }}
          onOpenMobileNav={() => {
            setPaletteOpen(false)
            setMobileOpen(true)
          }}
        />
        <main
          id="main-content"
          className="page-glow min-h-[calc(100dvh-3.5rem)] border-l border-transparent"
        >
          <div className="mx-auto w-full max-w-[1460px] px-4 py-5 sm:px-6 sm:py-7 lg:px-8">
            {configQuery.data?.issue ? (
              <ConfigBanner issue={configQuery.data.issue} />
            ) : null}
            {appsError != null ? (
              <ConnectionBanner error={appsError} onRetry={onRetry} retrying={retrying} />
            ) : null}
            <div key={pathname} className="animate-page-in">
              {children}
            </div>
          </div>
        </main>
      </div>

      <CommandPalette
        open={paletteOpen}
        onOpenChange={setPaletteOpen}
        apps={apps}
        onNavigate={() => setMobileOpen(false)}
      />

      <Dialog.Root open={mobileOpen} onOpenChange={setMobileOpen}>
        <Dialog.Portal>
          <Dialog.Backdrop className="fixed inset-0 z-50 bg-black/50 backdrop-blur-[2px] duration-150 data-open:animate-in data-open:fade-in-0 data-closed:animate-out data-closed:fade-out-0 lg:hidden" />
          <Dialog.Popup
            className={cn(
              "fixed inset-y-0 left-0 z-50 w-[284px] max-w-[86vw] border-r border-sidebar-border bg-sidebar p-0 outline-none lg:hidden",
              "data-open:animate-in data-open:slide-in-from-left data-open:duration-200 data-open:ease-[var(--ease-out-strong)] data-closed:animate-out data-closed:slide-out-to-left data-closed:duration-150"
            )}
            aria-label="Navigation"
          >
            <Dialog.Title className="sr-only">Navigation</Dialog.Title>
            <Dialog.Description className="sr-only">
              Pages and applications in the swapdock estate.
            </Dialog.Description>
            <SidebarContent
              {...sidebarProps}
              onClose={() => setMobileOpen(false)}
              onNavigate={() => setMobileOpen(false)}
            />
          </Dialog.Popup>
        </Dialog.Portal>
      </Dialog.Root>

      <ScrollRestoration />
      <Toaster position="bottom-right" closeButton />
    </div>
  )
}
