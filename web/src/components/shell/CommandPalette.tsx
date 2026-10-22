import { useEffect, useMemo, useRef, useState } from "react"
import { Dialog } from "@base-ui/react/dialog"
import { useNavigate } from "@tanstack/react-router"
import { useQueryClient } from "@tanstack/react-query"
import {
  Activity,
  ArrowRight,
  RefreshCw,
  ScrollText,
  Search,
  Stethoscope,
} from "lucide-react"
import { toast } from "sonner"
import type { AppEntry } from "@/api"
import { cn } from "cn"
import { AppGlyph } from "@/components/AppGlyph"
import { StatusDot, type Tone } from "@/components/Status"
import { Kbd } from "@/components/kit"

interface Command {
  id: string
  group: "Go to" | "Applications" | "Actions"
  title: string
  subtitle?: string
  icon?: typeof Activity
  app?: AppEntry
  tone?: Tone
  keywords: string
  run: () => void
}

export function CommandPalette({
  open,
  onOpenChange,
  apps,
  onNavigate,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  apps: AppEntry[]
  /** Called before a navigation, so the mobile sheet can close. */
  onNavigate?: () => void
}) {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [query, setQuery] = useState("")
  const [active, setActive] = useState(0)
  const inputRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    setQuery("")
    setActive(0)
    // Autofocus after Base UI has mounted the popup.
    const frame = requestAnimationFrame(() => inputRef.current?.focus())
    return () => cancelAnimationFrame(frame)
  }, [open])

  const commands = useMemo<Command[]>(() => {
    const close = () => onOpenChange(false)
    const go = (to: "/" | "/runs" | "/doctor", title: string) => () => {
      close()
      onNavigate?.()
      void navigate({ to, search: {} })
      if (title) document.title = `${title} · swapdock`
    }

    const base: Command[] = [
      {
        id: "nav-overview",
        group: "Go to",
        title: "Fleet overview",
        subtitle: "Every app, every warning",
        icon: Activity,
        keywords: "home overview fleet apps dashboard",
        run: go("/", "Fleet overview"),
      },
      {
        id: "nav-runs",
        group: "Go to",
        title: "Runs",
        subtitle: "The append-only deploy ledger",
        icon: ScrollText,
        keywords: "runs history log timeline deploys audit",
        run: go("/runs", "Runs"),
      },
      {
        id: "nav-doctor",
        group: "Go to",
        title: "Doctor",
        subtitle: "Host, compose and health findings",
        icon: Stethoscope,
        keywords: "doctor checks diagnostics health compose ports",
        run: go("/doctor", "Doctor"),
      },
      {
        id: "action-refresh",
        group: "Actions",
        title: "Refresh all data",
        subtitle: "Re-read the registry, runs and probes",
        icon: RefreshCw,
        keywords: "refresh reload refetch sync data",
        run: () => {
          close()
          const id = toast.loading("Refreshing all data…")
          void queryClient.invalidateQueries().finally(() => {
            toast.success("All data refreshed", { id })
          })
        },
      },
    ]

    const appCommands: Command[] = apps.map(({ app, problems }) => ({
      id: `app-${app.name}`,
      group: "Applications",
      title: app.name,
      subtitle:
        app.hostnames[0] ??
        (app.kind === "static" ? app.root ?? "static app" : `${app.kind} · :${app.front_port}`),
      app: { app, problems },
      tone: problems.some((problem) => problem.severity === "error")
        ? "danger"
        : problems.length > 0
          ? "warning"
          : app.live_port != null
            ? "success"
            : "neutral",
      keywords: `${app.name} ${app.hostnames.join(" ")} ${app.kind} ${app.strategy} ${app.release ?? ""}`,
      run: () => {
        close()
        onNavigate?.()
        void navigate({
          to: "/apps/$appName",
          params: { appName: app.name },
          search: { tab: "overview" },
        })
      },
    }))

    return [...base.filter((command) => command.group === "Go to"), ...appCommands, ...base.filter((command) => command.group === "Actions")]
  }, [apps, navigate, onNavigate, onOpenChange, queryClient])

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase()
    if (!needle) return commands
    return commands
      .map((command) => {
        const haystack = `${command.title} ${command.subtitle ?? ""} ${command.keywords}`.toLowerCase()
        if (!haystack.includes(needle)) return null
        const rank = command.title.toLowerCase().startsWith(needle) ? 0 : 1
        return { command, rank }
      })
      .filter((entry): entry is { command: Command; rank: number } => entry != null)
      .sort((a, b) => a.rank - b.rank)
      .map((entry) => entry.command)
  }, [commands, query])

  const groups = useMemo(() => {
    const order: Command["group"][] = ["Go to", "Applications", "Actions"]
    return order
      .map((group) => ({ group, items: filtered.filter((command) => command.group === group) }))
      .filter((entry) => entry.items.length > 0)
  }, [filtered])

  const flat = useMemo(() => groups.flatMap((group) => group.items), [groups])
  const boundedActive = flat.length === 0 ? 0 : Math.min(active, flat.length - 1)

  useEffect(() => {
    setActive(0)
  }, [query])

  useEffect(() => {
    const node = listRef.current?.querySelector<HTMLElement>('[data-active="true"]')
    node?.scrollIntoView({ block: "nearest" })
  }, [boundedActive, open])

  const select = (command: Command | undefined) => {
    if (!command) return
    command.run()
  }

  const onKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "ArrowDown") {
      event.preventDefault()
      setActive((value) => (flat.length === 0 ? 0 : (value + 1) % flat.length))
    } else if (event.key === "ArrowUp") {
      event.preventDefault()
      setActive((value) => (flat.length === 0 ? 0 : (value - 1 + flat.length) % flat.length))
    } else if (event.key === "Home") {
      event.preventDefault()
      setActive(0)
    } else if (event.key === "End") {
      event.preventDefault()
      setActive(Math.max(0, flat.length - 1))
    } else if (event.key === "Enter") {
      event.preventDefault()
      select(flat[boundedActive])
    }
  }

  // The active index must follow the same order the list renders in.
  let runningIndex = -1

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Backdrop className="fixed inset-0 z-50 bg-black/45 backdrop-blur-[2px] duration-150 data-open:animate-in data-open:fade-in-0 data-closed:animate-out data-closed:fade-out-0" />
        <Dialog.Popup
          className="pointer-events-none fixed inset-0 z-50 flex items-start justify-center px-4 pt-[12vh] outline-none"
          aria-label="Command menu"
        >
          <Dialog.Title className="sr-only">Command menu</Dialog.Title>
          <Dialog.Description className="sr-only">
            Search applications, jump to a page, or refresh the estate.
          </Dialog.Description>

          <div
            className="animate-pop-in pointer-events-auto flex w-full max-w-[620px] origin-top flex-col overflow-hidden rounded-xl border border-border bg-popover text-popover-foreground shadow-[var(--shadow-pop)]"
          >
            <div className="flex h-12 items-center gap-2.5 border-b border-border px-3.5">
              <Search className="size-4 shrink-0 text-subtle-foreground" />
              <input
                ref={inputRef}
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                onKeyDown={onKeyDown}
                placeholder="Search apps, pages, or actions…"
                aria-label="Search commands"
                role="combobox"
                aria-autocomplete="list"
                aria-expanded="true"
                aria-controls="command-list"
                aria-activedescendant={flat[boundedActive] ? `command-${flat[boundedActive].id}` : undefined}
                autoComplete="off"
                spellCheck={false}
                className="h-full min-w-0 flex-1 bg-transparent text-sm text-foreground outline-none placeholder:text-subtle-foreground"
              />
              <Kbd>esc</Kbd>
            </div>

            <div
              id="command-list"
              ref={listRef}
              role="listbox"
              aria-label="Commands"
              className="max-h-[min(560px,62vh)] overflow-y-auto overscroll-contain p-1.5"
            >
              {groups.length === 0 ? (
                <div className="px-3 py-10 text-center">
                  <p className="text-sm text-foreground">No matches</p>
                  <p className="mt-1 text-xs text-muted-foreground">
                    Try an app name, “runs”, or “doctor”.
                  </p>
                </div>
              ) : (
                groups.map((group) => (
                  <div key={group.group} className="pb-1 last:pb-0">
                    <div className="flex items-center justify-between px-2.5 pt-2 pb-1">
                      <span className="label-eyebrow">{group.group}</span>
                      <span className="text-2xs text-subtle-foreground">{group.items.length}</span>
                    </div>
                    {group.items.map((command) => {
                      runningIndex += 1
                      const isActive = runningIndex === boundedActive
                      const Icon = command.icon
                      return (
                        <button
                          key={command.id}
                          id={`command-${command.id}`}
                          type="button"
                          role="option"
                          aria-selected={isActive}
                          data-active={isActive ? "true" : undefined}
                          onMouseMove={() => setActive(runningIndex)}
                          onClick={() => select(command)}
                          className={cn(
                            "group flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left transition-colors duration-100",
                            isActive ? "command-active text-foreground" : "text-muted-foreground"
                          )}
                        >
                          {command.app ? (
                            <AppGlyph name={command.app.app.name} size="sm" />
                          ) : Icon ? (
                            <span className="flex size-6 items-center justify-center rounded-md border border-border bg-surface">
                              <Icon className="size-3.5" />
                            </span>
                          ) : null}
                          <span className="min-w-0 flex-1">
                            <span className="flex items-center gap-2">
                              <span
                                className={cn(
                                  "truncate text-sm",
                                  isActive ? "text-foreground" : "text-foreground/90"
                                )}
                              >
                                {command.title}
                              </span>
                              {command.app ? (
                                <StatusDot tone={command.tone ?? "neutral"} />
                              ) : null}
                            </span>
                            {command.subtitle ? (
                              <span className="block truncate text-2xs text-subtle-foreground">
                                {command.subtitle}
                              </span>
                            ) : null}
                          </span>
                          <ArrowRight
                            className={cn(
                              "size-3.5 shrink-0 text-subtle-foreground transition-[opacity,transform] duration-150",
                              isActive ? "translate-x-0 opacity-100" : "-translate-x-0.5 opacity-0"
                            )}
                          />
                        </button>
                      )
                    })}
                  </div>
                ))
              )}
            </div>

            <div className="flex items-center justify-between border-t border-border px-3.5 py-2 text-2xs text-subtle-foreground">
              <div className="flex items-center gap-3">
                <span className="flex items-center gap-1">
                  <Kbd>↑</Kbd>
                  <Kbd>↓</Kbd>
                  navigate
                </span>
                <span className="flex items-center gap-1">
                  <Kbd>↵</Kbd>
                  open
                </span>
              </div>
              <span className="font-mono text-[10px] tracking-wide">swapdock</span>
            </div>
          </div>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  )
}
