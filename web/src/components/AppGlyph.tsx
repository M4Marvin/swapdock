import { cn } from "cn"
import { hueFor, initialsFor } from "@/lib/format"

const sizeClasses = {
  xs: "size-5 rounded-[5px] text-[9px]",
  sm: "size-6 rounded-md text-[10px]",
  md: "size-7 rounded-[7px] text-[10.5px]",
  lg: "size-10 rounded-[10px] text-sm",
  xl: "size-12 rounded-xl text-base",
} as const

/**
 * A stable, quiet identity tile for an app.
 *
 * The hue is derived from the name, so an app keeps its colour across the
 * fleet list, the header, and the command palette. It never carries meaning on
 * its own — status is always shown with a dot and a label.
 */
export function AppGlyph({
  name,
  size = "md",
  className,
}: {
  name: string
  size?: keyof typeof sizeClasses
  className?: string
}) {
  return (
    <span
      aria-hidden="true"
      style={{ "--glyph-h": hueFor(name) } as React.CSSProperties}
      className={cn(
        "app-glyph inline-flex shrink-0 items-center justify-center font-mono font-semibold tracking-[-0.03em] select-none",
        sizeClasses[size],
        className
      )}
    >
      {initialsFor(name)}
    </span>
  )
}
