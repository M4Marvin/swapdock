import type { ComponentType, ReactNode } from "react"
import { cn } from "cn"
import {
  Check,
  CheckCircle2,
  Copy,
  Info,
  Loader2,
  Search,
  TriangleAlert,
  XCircle,
} from "lucide-react"
import { useCopy } from "@/lib/hooks"
import { useNow } from "@/lib/now"
import { formatExact, formatRelative } from "@/lib/format"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"

/* -------------------------------------------------------------------------- */
/* time                                                                       */
/* -------------------------------------------------------------------------- */

export function RelativeTime({
  value,
  className,
  fallback = "—",
}: {
  value: string | number | Date | null | undefined
  className?: string
  fallback?: ReactNode
}) {
  const now = useNow()
  if (value == null) return <span className={className}>{fallback}</span>
  const date = value instanceof Date ? value : new Date(value)
  if (Number.isNaN(date.getTime())) return <span className={className}>{fallback}</span>
  return (
    <time
      dateTime={date.toISOString()}
      title={formatExact(date)}
      className={cn("tabular whitespace-nowrap", className)}
    >
      {formatRelative(date, now)}
    </time>
  )
}

/* -------------------------------------------------------------------------- */
/* copy                                                                       */
/* -------------------------------------------------------------------------- */

export function CopyButton({
  value,
  label = "Copy",
  className,
  size = "icon-xs",
  variant = "ghost",
}: {
  value: string
  label?: string
  className?: string
  size?: "icon-xs" | "icon-sm" | "icon"
  variant?: "ghost" | "outline"
}) {
  const { copied, copy } = useCopy()
  return (
    <Button
      type="button"
      variant={variant}
      size={size}
      className={cn("text-subtle-foreground hover:text-foreground", className)}
      title={copied ? "Copied" : `${label} ${value}`}
      aria-label={copied ? "Copied" : label}
      onClick={(event) => {
        event.stopPropagation()
        event.preventDefault()
        void copy(value)
      }}
    >
      {copied ? <Check className="text-success" /> : <Copy />}
    </Button>
  )
}

/* -------------------------------------------------------------------------- */
/* keyboard                                                                   */
/* -------------------------------------------------------------------------- */

export function Kbd({ children, className }: { children: ReactNode; className?: string }) {
  return <kbd className={className}>{children}</kbd>
}

/* -------------------------------------------------------------------------- */
/* callout                                                                    */
/* -------------------------------------------------------------------------- */

type CalloutTone = "info" | "warning" | "danger" | "success" | "neutral"

const calloutTone: Record<CalloutTone, { wrap: string; icon: string }> = {
  info: { wrap: "border-info/20 bg-info/5", icon: "text-info" },
  warning: { wrap: "border-warning/20 bg-warning/5", icon: "text-warning" },
  danger: { wrap: "border-destructive/25 bg-destructive/5", icon: "text-destructive" },
  success: { wrap: "border-success/20 bg-success/5", icon: "text-success" },
  neutral: { wrap: "border-border bg-muted/30", icon: "text-muted-foreground" },
}

const calloutIcon: Record<CalloutTone, ComponentType<{ className?: string }>> = {
  info: Info,
  warning: TriangleAlert,
  danger: XCircle,
  success: CheckCircle2,
  neutral: Info,
}

export function Callout({
  tone = "info",
  title,
  children,
  action,
  icon,
  className,
}: {
  tone?: CalloutTone
  title?: ReactNode
  children?: ReactNode
  action?: ReactNode
  icon?: ComponentType<{ className?: string }>
  className?: string
}) {
  const Icon = icon ?? calloutIcon[tone]
  return (
    <div
      role={tone === "danger" ? "alert" : undefined}
      className={cn(
        "flex items-start gap-3 rounded-xl border px-3.5 py-3 text-sm",
        calloutTone[tone].wrap,
        className
      )}
    >
      <Icon className={cn("mt-0.5 size-4 shrink-0", calloutTone[tone].icon)} />
      <div className="min-w-0 flex-1 space-y-0.5">
        {title ? <div className="font-medium text-foreground">{title}</div> : null}
        {children ? (
          <div className="text-pretty text-xs leading-relaxed text-muted-foreground">
            {children}
          </div>
        ) : null}
      </div>
      {action ? <div className="shrink-0">{action}</div> : null}
    </div>
  )
}

/* -------------------------------------------------------------------------- */
/* empty state                                                                */
/* -------------------------------------------------------------------------- */

export function EmptyState({
  icon: Icon,
  title,
  description,
  action,
  children,
  className,
}: {
  icon: ComponentType<{ className?: string }>
  title: ReactNode
  description?: ReactNode
  action?: ReactNode
  children?: ReactNode
  className?: string
}) {
  return (
    <div
      className={cn(
        "relative overflow-hidden rounded-xl border border-border bg-card px-6 py-12 text-center shadow-[var(--shadow-card)]",
        className
      )}
    >
      <div
        aria-hidden="true"
        className="grid-dots pointer-events-none absolute inset-0 opacity-50 [mask-image:radial-gradient(60%_60%_at_50%_0%,black,transparent)]"
      />
      <div className="relative mx-auto flex max-w-md flex-col items-center gap-3">
        <span className="flex size-10 items-center justify-center rounded-xl border border-border bg-elevated text-muted-foreground shadow-[var(--shadow-card)]">
          <Icon className="size-4.5" />
        </span>
        <div className="space-y-1">
          <h2 className="text-base font-medium tracking-[-0.015em] text-foreground">{title}</h2>
          {description ? (
            <p className="text-pretty text-xs leading-relaxed text-muted-foreground">
              {description}
            </p>
          ) : null}
        </div>
        {action ? <div className="mt-1 flex flex-wrap items-center justify-center gap-2">{action}</div> : null}
        {children}
      </div>
    </div>
  )
}

/* -------------------------------------------------------------------------- */
/* headings                                                                   */
/* -------------------------------------------------------------------------- */

export function SectionHeading({
  title,
  description,
  action,
  className,
}: {
  title: ReactNode
  description?: ReactNode
  action?: ReactNode
  className?: string
}) {
  return (
    <div className={cn("flex items-start justify-between gap-4", className)}>
      <div className="min-w-0 space-y-0.5">
        <h2 className="text-sm font-medium tracking-[-0.01em] text-foreground">{title}</h2>
        {description ? (
          <p className="text-pretty text-xs leading-relaxed text-muted-foreground">
            {description}
          </p>
        ) : null}
      </div>
      {action ? <div className="flex shrink-0 items-center gap-2">{action}</div> : null}
    </div>
  )
}

/* -------------------------------------------------------------------------- */
/* metadata                                                                   */
/* -------------------------------------------------------------------------- */

export interface MetaItem {
  label: ReactNode
  value: ReactNode
  mono?: boolean
  title?: string
  href?: string
}

export function MetaList({
  items,
  columns = 2,
  className,
}: {
  items: MetaItem[]
  columns?: 1 | 2 | 3 | 4
  className?: string
}) {
  const columnClass =
    columns === 1
      ? "grid-cols-1"
      : columns === 2
        ? "grid-cols-2"
        : columns === 3
          ? "grid-cols-2 sm:grid-cols-3"
          : "grid-cols-2 sm:grid-cols-4"
  return (
    <dl className={cn("grid gap-x-5 gap-y-3.5", columnClass, className)}>
      {items.map((item, index) => (
        <div key={`${String(item.label)}-${index}`} className="min-w-0 space-y-1">
          <dt className="label-eyebrow">{item.label}</dt>
          <dd
            title={item.title}
            className={cn(
              "truncate text-sm text-foreground",
              item.mono && "font-mono text-xs tracking-[-0.01em]"
            )}
          >
            {item.href ? (
              <a
                href={item.href}
                target="_blank"
                rel="noreferrer"
                className="underline decoration-border underline-offset-4 transition-colors hover:text-accent hover:decoration-accent/40"
              >
                {item.value}
              </a>
            ) : (
              item.value
            )}
          </dd>
        </div>
      ))}
    </dl>
  )
}

/* -------------------------------------------------------------------------- */
/* controls                                                                   */
/* -------------------------------------------------------------------------- */

export function SearchInput({
  value,
  onChange,
  placeholder = "Search",
  ariaLabel,
  className,
  right,
  inputClassName,
}: {
  value: string
  onChange: (value: string) => void
  placeholder?: string
  /** Accessible name. A placeholder alone is not a label. */
  ariaLabel?: string
  className?: string
  right?: ReactNode
  inputClassName?: string
}) {
  return (
    <div className={cn("relative", className)}>
      <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-subtle-foreground" />
      <Input
        type="search"
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
        aria-label={ariaLabel ?? placeholder}
        className={cn("h-8 bg-surface pl-8 text-sm [&::-webkit-search-cancel-button]:hidden", inputClassName)}
      />
      {right ? (
        <div className="absolute top-1/2 right-2 -translate-y-1/2">{right}</div>
      ) : null}
    </div>
  )
}

export interface FilterOption<T extends string> {
  value: T
  label: string
  count?: number
}

export function FilterPills<T extends string>({
  options,
  value,
  onChange,
  className,
  ariaLabel,
}: {
  options: FilterOption<T>[]
  value: T
  onChange: (value: T) => void
  className?: string
  ariaLabel?: string
}) {
  return (
    <div
      role="group"
      aria-label={ariaLabel}
      className={cn("inline-flex items-center gap-0.5 rounded-lg border border-border bg-surface p-0.5", className)}
    >
      {options.map((option) => {
        const active = option.value === value
        return (
          <button
            key={option.value}
            type="button"
            aria-pressed={active}
            onClick={() => onChange(option.value)}
            className={cn(
              "inline-flex h-6 items-center gap-1.5 rounded-md px-2 text-2xs font-medium transition-[color,background-color,box-shadow] duration-150",
              active
                ? "bg-foreground text-background shadow-sm"
                : "text-muted-foreground hover:text-foreground"
            )}
          >
            {option.label}
            {option.count != null ? (
              <span
                className={cn(
                  "tabular text-2xs",
                  active ? "text-background/60" : "text-subtle-foreground"
                )}
              >
                {option.count}
              </span>
            ) : null}
          </button>
        )
      })}
    </div>
  )
}

/* -------------------------------------------------------------------------- */
/* stats                                                                      */
/* -------------------------------------------------------------------------- */

export function StatTile({
  label,
  value,
  hint,
  icon: Icon,
  tone = "neutral",
  className,
  children,
}: {
  label: ReactNode
  value: ReactNode
  hint?: ReactNode
  icon?: ComponentType<{ className?: string }>
  tone?: "neutral" | "success" | "warning" | "danger" | "accent"
  className?: string
  children?: ReactNode
}) {
  const iconTone =
    tone === "success"
      ? "text-success"
      : tone === "warning"
        ? "text-warning"
        : tone === "danger"
          ? "text-destructive"
          : tone === "accent"
            ? "text-accent"
            : "text-subtle-foreground"
  return (
    <div
      className={cn(
        "relative flex flex-col justify-between gap-4 overflow-hidden rounded-xl border border-border bg-card p-4 shadow-[var(--shadow-card)]",
        className
      )}
    >
      <div className="flex items-start justify-between gap-3">
        <span className="label-eyebrow">{label}</span>
        {Icon ? <Icon className={cn("size-3.5", iconTone)} /> : null}
      </div>
      <div className="min-w-0">
        <div className="flex items-baseline gap-1.5">
          <span className="tabular shrink-0 text-2xl font-semibold tracking-[-0.03em] text-foreground">{value}</span>
          {hint ? <span className="min-w-0 text-2xs leading-tight text-muted-foreground">{hint}</span> : null}
        </div>
        {children}
      </div>
    </div>
  )
}

/* -------------------------------------------------------------------------- */
/* misc                                                                       */
/* -------------------------------------------------------------------------- */

export function Spinner({ className }: { className?: string }) {
  return <Loader2 className={cn("size-3.5 animate-spin", className)} aria-hidden="true" />
}

export function CodeSnippet({
  code,
  className,
  copyLabel = "Copy command",
}: {
  code: string
  className?: string
  copyLabel?: string
}) {
  return (
    <div
      className={cn(
        "code-surface group/snippet relative flex items-start gap-3 rounded-lg border border-border px-3 py-2",
        className
      )}
    >
      <pre className="min-w-0 flex-1 overflow-x-auto font-mono text-xs leading-5 text-foreground">
        {code}
      </pre>
      <CopyButton value={code} label={copyLabel} className="shrink-0" />
    </div>
  )
}
