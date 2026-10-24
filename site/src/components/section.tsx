import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'

export function Section({
  id,
  children,
  className,
  bordered = false,
}: {
  id?: string
  children: ReactNode
  className?: string
  bordered?: boolean
}) {
  return (
    <section
      id={id}
      className={cn(
        'relative scroll-mt-20 py-20 md:py-28',
        bordered && 'border-t border-border',
        className,
      )}
    >
      {children}
    </section>
  )
}

export function Container({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn('container-page', className)}>{children}</div>
}

export function Eyebrow({ children, className }: { children: ReactNode; className?: string }) {
  return <p className={cn('label-eyebrow mb-4 flex items-center gap-2', className)}>{children}</p>
}

export function SectionHead({
  eyebrow,
  title,
  lead,
  align = 'left',
  className,
}: {
  eyebrow?: ReactNode
  title: ReactNode
  lead?: ReactNode
  align?: 'left' | 'center'
  className?: string
}) {
  return (
    <div
      className={cn(
        'mb-12 max-w-2xl md:mb-16',
        align === 'center' && 'mx-auto text-center',
        className,
      )}
    >
      {eyebrow ? <Eyebrow className={align === 'center' ? 'justify-center' : undefined}>{eyebrow}</Eyebrow> : null}
      <h2 className="display-2 text-foreground">{title}</h2>
      {lead ? <p className="mt-4 text-base leading-relaxed text-muted-foreground md:text-lg">{lead}</p> : null}
    </div>
  )
}

export function PageHero({
  eyebrow,
  title,
  lead,
  children,
  className,
}: {
  eyebrow: ReactNode
  title: ReactNode
  lead: ReactNode
  children?: ReactNode
  className?: string
}) {
  return (
    <section className={cn('relative overflow-hidden border-b border-border pb-16 pt-14 md:pb-20 md:pt-20', className)}>
      <div className="grid-backdrop pointer-events-none absolute inset-0 opacity-50" />
      <div className="glow -top-32 left-1/4 h-72 w-[480px] bg-accent/12" />
      <Container className="relative">
        <div className="max-w-3xl">
          <Eyebrow>{eyebrow}</Eyebrow>
          <h1 className="display-1">{title}</h1>
          <p className="mt-6 max-w-2xl text-[16.5px] leading-relaxed text-muted-foreground">{lead}</p>
          {children ? <div className="mt-8">{children}</div> : null}
        </div>
      </Container>
    </section>
  )
}

export function Prose({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn('prose-swapdock', className)}>{children}</div>
}
