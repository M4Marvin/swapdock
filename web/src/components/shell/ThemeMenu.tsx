import { useEffect, useRef, useState } from "react"
import { Check, Monitor, Moon, Sun } from "lucide-react"
import { cn } from "cn"
import { useTheme, type Theme } from "@/lib/theme"
import { Button } from "@/components/ui/button"

const options: { value: Theme; label: string; icon: typeof Sun }[] = [
  { value: "system", label: "System", icon: Monitor },
  { value: "light", label: "Light", icon: Sun },
  { value: "dark", label: "Dark", icon: Moon },
]

/** Theme picker anchored to the top bar. Closes on outside click and Escape. */
export function ThemeMenu() {
  const { theme, resolved, setTheme } = useTheme()
  const [open, setOpen] = useState(false)
  const rootRef = useRef<HTMLDivElement>(null)
  const firstRef = useRef<HTMLButtonElement>(null)
  const Current = theme === "system" ? Monitor : theme === "light" ? Sun : Moon

  useEffect(() => {
    if (!open) return
    const onPointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false)
    }
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setOpen(false)
        rootRef.current?.querySelector<HTMLButtonElement>("button[data-theme-trigger]")?.focus()
      }
    }
    document.addEventListener("pointerdown", onPointerDown, true)
    document.addEventListener("keydown", onKeyDown)
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true)
      document.removeEventListener("keydown", onKeyDown)
    }
  }, [open])

  useEffect(() => {
    if (open) firstRef.current?.focus()
  }, [open])

  return (
    <div ref={rootRef} className="relative">
      <Button
        type="button"
        data-theme-trigger=""
        variant="ghost"
        size="icon-sm"
        aria-label={`Theme: ${theme}. Change theme.`}
        aria-haspopup="menu"
        aria-expanded={open}
        title={`Theme: ${theme}`}
        onClick={() => setOpen((value) => !value)}
      >
        <Current />
      </Button>

      {open ? (
        <div
          role="menu"
          aria-label="Theme"
          className="animate-pop-in absolute top-full right-0 z-50 mt-1.5 w-40 origin-top-right rounded-lg border border-border bg-popover p-1 shadow-[var(--shadow-pop)]"
        >
          <div className="flex items-center justify-between px-2 pt-1 pb-1.5">
            <span className="label-eyebrow">Appearance</span>
            <span className="text-2xs text-subtle-foreground">{resolved}</span>
          </div>
          {options.map((option, index) => {
            const Icon = option.icon
            const selected = theme === option.value
            return (
              <button
                key={option.value}
                ref={index === 0 ? firstRef : undefined}
                type="button"
                role="menuitemradio"
                aria-checked={selected}
                onClick={() => {
                  setTheme(option.value)
                  setOpen(false)
                }}
                className={cn(
                  "flex h-7 w-full items-center gap-2 rounded-md px-2 text-left text-sm transition-colors duration-100",
                  selected
                    ? "bg-muted text-foreground"
                    : "text-muted-foreground hover:bg-muted/60 hover:text-foreground"
                )}
              >
                <Icon className="size-3.5" />
                <span className="flex-1">{option.label}</span>
                {selected ? <Check className="size-3.5 text-accent" /> : null}
              </button>
            )
          })}
        </div>
      ) : null}
    </div>
  )
}
