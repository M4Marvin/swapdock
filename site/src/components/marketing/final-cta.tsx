import { Link } from '@tanstack/react-router'
import { ArrowRight } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Container, Section } from '@/components/section'

type Path = '/' | '/path/' | '/docs/' | '/how-it-works/' | '/features/' | '/faq/'

export function FinalCta({
  title = 'Make the next deploy boring.',
  lead = 'One TOML. One command. A rollback that works at 3am.',
  primaryTo = '/path/',
  primaryLabel = 'Start the path',
  secondaryTo = '/docs/',
  secondaryLabel = 'Read the quickstart',
}: {
  title?: string
  lead?: string
  primaryTo?: Path
  primaryLabel?: string
  secondaryTo?: Path
  secondaryLabel?: string
}) {
  return (
    <Section className="pb-24 pt-8 md:pb-32">
      <Container>
        <div className="relative overflow-hidden rounded-2xl border border-border bg-card px-6 py-16 text-center md:px-12 md:py-20">
          <div className="glow -top-24 left-1/2 h-56 w-[420px] -translate-x-1/2 bg-accent/15" />
          <div className="grid-backdrop pointer-events-none absolute inset-0 opacity-40" />
          <div className="relative">
            <p className="label-eyebrow justify-center">Start where you are</p>
            <h2 className="display-2 mx-auto mt-4 max-w-[16ch]">{title}</h2>
            <p className="mx-auto mt-4 max-w-lg text-[15px] leading-relaxed text-muted-foreground">{lead}</p>
            <div className="mt-8 flex flex-wrap items-center justify-center gap-3">
              <Button className="h-11 rounded-lg px-6 text-[13.5px]" render={<Link to={primaryTo} />}>
                {primaryLabel} <ArrowRight className="size-4" />
              </Button>
              <Button
                variant="outline"
                className="h-11 rounded-lg px-6 text-[13.5px]"
                render={<Link to={secondaryTo} />}
              >
                {secondaryLabel}
              </Button>
            </div>
            <p className="mt-5 text-[11px] text-subtle-foreground">
              Open source · MIT · pre-launch v0.2 · no account, no telemetry
            </p>
          </div>
        </div>
      </Container>
    </Section>
  )
}
