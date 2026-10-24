import { Link } from '@tanstack/react-router'
import { ArrowRight } from 'lucide-react'
import { STAGES } from '@/content/path'
import { cn } from '@/lib/utils'

const HOME_STAGES = ['install', 'first-app', 'first-cutover', 'recovery']

export function PathRail({ className }: { className?: string }) {
  const stages = HOME_STAGES.map((id) => STAGES.find((s) => s.id === id)).filter(Boolean)

  return (
    <div className={cn('grid gap-3 md:grid-cols-4', className)}>
      {stages.map((stage, i) =>
        stage ? (
          <div key={stage.id} className="relative border-t border-border pt-5">
            <span className="absolute -top-1 left-0 size-2 rounded-full bg-accent" />
            <div className="flex items-center justify-between">
              <span className="mono text-[10px] text-subtle-foreground">{stage.n}</span>
              <span className="text-[10px] text-subtle-foreground">{stage.time}</span>
            </div>
            <h3 className="mt-2 text-[14px] font-semibold">{stage.title}</h3>
            <p className="mt-1.5 text-[12px] leading-relaxed text-muted-foreground">{stage.get}</p>
            {i < stages.length - 1 ? null : (
              <Link
                to="/path/"
                className="mt-4 inline-flex items-center gap-1 text-[11.5px] font-medium text-accent"
              >
                Start the path <ArrowRight className="size-3" />
              </Link>
            )}
          </div>
        ) : null,
      )}
    </div>
  )
}
