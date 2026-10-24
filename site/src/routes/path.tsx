import { Link, createFileRoute } from '@tanstack/react-router'
import { ArrowRight, Check, Clock } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Container, PageHero, Section, SectionHead } from '@/components/section'
import { CodeBlock } from '@/components/marketing/code-block'
import { Reveal } from '@/components/reveal'
import { FinalCta } from '@/components/marketing/final-cta'
import { STAGES } from '@/content/path'
import { pageHead } from '@/lib/seo'

export const Route = createFileRoute('/path')({
  head: () =>
    pageHead({
      title: 'The path — from compose up to boring deploys',
      description:
        'A seven-stage route from a hand-rolled Docker host to health-gated cutovers, tested rollbacks, safe stateful apps, and crash recovery. Commands and outcomes at each step.',
      path: '/path',
    }),
  component: PathPage,
})

const starts = [
  {
    title: 'A new app',
    body: 'Describe it once, deploy it, roll it back. Start at stage 01.',
    href: '#stage-install',
    label: 'Install first',
  },
  {
    title: 'An existing Compose app',
    body: 'Import reads the merged Compose model and writes the registry entry. Start at stage 02.',
    href: '#stage-first-app',
    label: 'Describe one app',
  },
  {
    title: 'A legacy container',
    body: 'Adopt it at the commit it is already running, then use the normal loop. Start at stage 06.',
    href: '#stage-estate',
    label: 'Adopt the estate',
  },
]

function PathPage() {
  return (
    <>
      <PageHero
        eyebrow="The path"
        title="From compose up to boring deploys."
        lead="Seven stages. Each has a command, a result, and a time. You do not have to migrate everything at once: one app proves the loop."
      >
        <div className="flex flex-wrap items-center gap-4 text-[12px] text-muted-foreground">
          <span className="inline-flex items-center gap-2">
            <Clock className="size-3.5 text-accent" /> about 1 hour 40, plus the estate
          </span>
          <span className="inline-flex items-center gap-2">
            <Check className="size-3.5 text-live" /> first cutover in ~25 minutes
          </span>
        </div>
      </PageHero>

      <Section className="pb-10 md:pb-14">
        <Container>
          <div className="grid gap-3 md:grid-cols-3">
            {starts.map((s) => (
              <a
                key={s.title}
                href={s.href}
                className="group rounded-xl border border-border bg-card p-5 transition-colors hover:border-border-strong hover:bg-elevated"
              >
                <div className="flex items-center justify-between">
                  <h2 className="text-[14px] font-semibold">{s.title}</h2>
                  <ArrowRight className="size-3.5 text-subtle-foreground transition-transform group-hover:translate-x-0.5" />
                </div>
                <p className="mt-2 text-[12px] leading-relaxed text-muted-foreground">{s.body}</p>
                <span className="mt-4 inline-block text-[11px] font-medium text-accent">{s.label}</span>
              </a>
            ))}
          </div>
        </Container>
      </Section>

      <Section bordered className="pt-14">
        <Container>
          <div className="grid grid-cols-1 gap-12 lg:grid-cols-[220px_minmax(0,1fr)] lg:gap-16">
            <aside className="hidden lg:block">
              <div className="sticky top-24">
                <p className="label-eyebrow">Seven stages</p>
                <nav className="mt-4 space-y-1" aria-label="Path stages">
                  {STAGES.map((s) => (
                    <a
                      key={s.id}
                      href={`#stage-${s.id}`}
                      className="group flex items-center gap-3 rounded-md px-2 py-2 text-[12.5px] text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
                    >
                      <span className="mono text-[9.5px] text-subtle-foreground">{s.n}</span>
                      {s.title}
                    </a>
                  ))}
                </nav>
                <div className="mt-6 border-t border-border pt-4 text-[11px] leading-relaxed text-subtle-foreground">
                  The first four stages are the shortest route to a tested rollback.
                </div>
              </div>
            </aside>

            <div className="min-w-0">
              <div className="mb-6 flex max-w-full gap-2 overflow-x-auto pb-1 lg:hidden">
                {STAGES.map((s) => (
                  <a
                    key={s.id}
                    href={`#stage-${s.id}`}
                    className="shrink-0 rounded-full border border-border px-3 py-1.5 text-[11px] text-muted-foreground"
                  >
                    {s.n} · {s.title}
                  </a>
                ))}
              </div>

              <div className="space-y-4">
                {STAGES.map((stage, i) => (
                  <Reveal key={stage.id} delay={Math.min(i * 30, 120)}>
                    <article
                      id={`stage-${stage.id}`}
                      className="scroll-mt-24 rounded-xl border border-border bg-card p-6 md:p-8"
                    >
                      <div className="flex items-start gap-4">
                        <span className="mono mt-1 text-[10px] text-subtle-foreground">{stage.n}</span>
                        <div className="min-w-0 flex-1">
                          <div className="flex flex-wrap items-baseline justify-between gap-2">
                            <h2 className="display-3">{stage.title}</h2>
                            <span className="mono text-[10.5px] text-subtle-foreground">{stage.time}</span>
                          </div>
                          <p className="mt-3 max-w-2xl text-[13px] leading-relaxed text-muted-foreground">
                            {stage.detail}
                          </p>

                          <div className="mt-6 grid gap-5 sm:grid-cols-2">
                            <div>
                              <p className="label-eyebrow">You have</p>
                              <p className="mt-2 text-[12.5px] leading-relaxed text-muted-foreground">
                                {stage.have}
                              </p>
                            </div>
                            <div>
                              <p className="label-eyebrow">You get</p>
                              <p className="mt-2 flex items-start gap-2 text-[12.5px] leading-relaxed text-foreground">
                                <Check className="mt-0.5 size-3.5 shrink-0 text-live" strokeWidth={3} />
                                {stage.get}
                              </p>
                            </div>
                          </div>

                          <CodeBlock className="mt-5" lines={stage.run} />
                        </div>
                      </div>
                    </article>
                  </Reveal>
                ))}
              </div>
            </div>
          </div>
        </Container>
      </Section>

      <Section className="bg-surface/25" bordered>
        <Container>
          <SectionHead
            eyebrow="After the path"
            title="What you have at the end."
            lead="Not a platform to operate. A routine you can repeat, and a state you can recover."
          />
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
            {[
              ['A boring deploy', 'One command, two gates, no maintenance window.'],
              ['A tested rollback', 'The previous release, through the same gate.'],
              ['A safe stateful app', 'One writer, backup hook, replace by rule.'],
              ['A readable estate', 'Journal, run log, verify, monitor, one console.'],
            ].map(([title, body]) => (
              <div key={title} className="rounded-xl border border-border bg-card p-5">
                <Check className="size-4 text-live" strokeWidth={3} />
                <h3 className="mt-4 text-[13.5px] font-semibold">{title}</h3>
                <p className="mt-1.5 text-[12px] leading-relaxed text-muted-foreground">{body}</p>
              </div>
            ))}
          </div>
          <div className="mt-8 flex flex-wrap gap-3">
            <Button className="h-11 rounded-lg px-5 text-[13.5px]" render={<Link to="/docs/" />}>
              Read the quickstart <ArrowRight className="size-4" />
            </Button>
            <Button variant="outline" className="h-11 rounded-lg px-5 text-[13.5px]" render={<Link to="/features/recovery/" />}>
              Recovery in detail
            </Button>
          </div>
        </Container>
      </Section>

      <FinalCta
        title="Start where you are."
        lead="One app, one cutover, one rollback drill. The rest of the estate can wait."
        primaryTo="/docs/"
        primaryLabel="Read the quickstart"
        secondaryTo="/how-it-works/"
        secondaryLabel="The full loop"
      />
    </>
  )
}
