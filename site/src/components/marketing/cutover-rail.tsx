import { ArrowRight, Check } from 'lucide-react'
import { cn } from '@/lib/utils'

const steps = [
  { label: 'start green', note: 'free port' },
  { label: 'health gate', note: 'container + HTTP' },
  { label: 'nginx flip', note: 'tested, atomic' },
  { label: 'probe + drain', note: '10 tries, 15 s' },
  { label: 'commit', note: 'registry last' },
]

export function CutoverRail({
  className,
  orientation = 'horizontal',
}: {
  className?: string
  orientation?: 'horizontal' | 'vertical'
}) {
  if (orientation === 'vertical') {
    return (
      <div className={cn('space-y-0', className)}>
        {steps.map((step, i) => (
          <div
            key={step.label}
            className="flex items-start gap-3 border-b border-border py-2.5 last:border-0"
          >
            <span className="mt-0.5 grid size-4 place-items-center rounded-full bg-live-soft text-live">
              <Check className="size-2.5" strokeWidth={3} />
            </span>
            <div>
              <p className="mono text-[11px] text-foreground">{step.label}</p>
              <p className="mt-0.5 text-[10.5px] text-subtle-foreground">{step.note}</p>
            </div>
            <span className="mono ml-auto text-[9.5px] text-subtle-foreground">0{i + 1}</span>
          </div>
        ))}
      </div>
    )
  }

  return (
    <div className={className}>
      <div className="flex flex-col gap-2 md:flex-row md:items-stretch">
        {steps.map((step, i) => (
          <div key={step.label} className="flex flex-1 items-center gap-2">
            <div className="flex-1 rounded-lg border border-border bg-card px-3 py-2.5">
              <div className="flex items-center gap-1.5">
                <Check className="size-3 shrink-0 text-live" strokeWidth={3} />
                <span className="mono text-[10.5px] text-foreground">{step.label}</span>
              </div>
              <p className="mt-1 text-[10px] text-subtle-foreground">{step.note}</p>
            </div>
            {i < steps.length - 1 ? (
              <ArrowRight className="hidden size-3.5 shrink-0 text-border-strong md:block" />
            ) : null}
          </div>
        ))}
      </div>
    </div>
  )
}
