import { Link } from '@tanstack/react-router'
import { ArrowLeftRight, ArrowUpRight } from 'lucide-react'
import type { Feature } from '@/content/features'
import { ICONS } from '@/components/marketing/icon-map'
import { cn } from '@/lib/utils'

export function FeatureCard({ feature, className }: { feature: Feature; className?: string }) {
  const Icon = ICONS[feature.icon] ?? ArrowLeftRight

  const body = (
    <>
      <span className="mb-5 grid size-9 place-items-center rounded-lg border border-border bg-accent-soft text-accent">
        <Icon className="size-4" />
      </span>
      <h3 className="text-[15px] font-semibold tracking-[-0.01em]">{feature.title}</h3>
      <p className="mt-2 text-[13px] leading-relaxed text-muted-foreground">{feature.blurb}</p>
      {feature.href ? (
        <span className="mt-auto inline-flex items-center gap-1 pt-5 text-[12px] font-medium text-accent">
          How it works <ArrowUpRight className="size-3.5" />
        </span>
      ) : null}
    </>
  )

  const classes = cn(
    'group flex h-full flex-col rounded-xl border border-border bg-card p-5 transition-colors duration-200',
    feature.href && 'hover:border-border-strong hover:bg-elevated',
    className,
  )

  if (feature.href) {
    return (
      <Link to={feature.href} className={classes}>
        {body}
      </Link>
    )
  }
  return <div className={classes}>{body}</div>
}

