import type { ReactNode } from "react"
import { cn } from "cn"

/**
 * The one page header used everywhere: optional leading glyph, an eyebrow,
 * a title with inline badges, one sentence of context, and a right-aligned
 * action cluster.
 */
export function PageHeader({
  eyebrow,
  title,
  description,
  badges,
  actions,
  leading,
  className,
}: {
  eyebrow?: ReactNode
  title: ReactNode
  description?: ReactNode
  badges?: ReactNode
  actions?: ReactNode
  leading?: ReactNode
  className?: string
}) {
  return (
    <header className={cn("flex flex-col gap-4", className)}>
      <div className="flex flex-wrap items-start justify-between gap-x-6 gap-y-4">
        <div className="flex min-w-0 items-start gap-3.5">
          {leading}
          <div className="min-w-0 space-y-1">
            {eyebrow ? <div className="label-eyebrow">{eyebrow}</div> : null}
            <div className="flex flex-wrap items-center gap-2.5">
              <h1 className="text-2xl leading-none font-semibold tracking-[-0.028em] text-foreground">
                {title}
              </h1>
              {badges}
            </div>
            {description ? (
              <div className="max-w-2xl text-pretty text-sm leading-5 text-muted-foreground">
                {description}
              </div>
            ) : null}
          </div>
        </div>
        {actions ? (
          <div className="flex flex-wrap items-center gap-2 pt-0.5">{actions}</div>
        ) : null}
      </div>
    </header>
  )
}
