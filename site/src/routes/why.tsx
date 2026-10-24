import { Link, createFileRoute } from '@tanstack/react-router'
import { ArrowRight, Database, EyeOff, FileWarning, History } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Container, PageHero, Section, SectionHead } from '@/components/section'
import { Reveal } from '@/components/reveal'
import { FinalCta } from '@/components/marketing/final-cta'
import { pageHead } from '@/lib/seo'

export const Route = createFileRoute('/why')({
  head: () =>
    pageHead({
      title: 'Why swapdock — the single-box deploy problem',
      description:
        'Hand-edited nginx, undocumented rollbacks, and stateful apps make a single-server deploy risky. Here is what breaks, what it costs, and what changes.',
      path: '/why',
    }),
  component: Why,
})

const failures = [
  {
    icon: FileWarning,
    title: 'nginx drift',
    body: 'The front door is a file that nobody reviews in a pull request. A port change, a missing semicolon, or a stale upstream takes down every app behind it — not just the one you touched.',
    cost: 'One typo, every app down.',
  },
  {
    icon: History,
    title: 'Rollback archaeology',
    body: 'The previous image is in the daemon, somewhere. The command is in shell history. The tag may have been overwritten. The health check is whether the page loads for you at 3am.',
    cost: 'Recovery time equals memory.',
  },
  {
    icon: Database,
    title: 'The stateful trap',
    body: 'Two containers, one SQLite file. The fast deploy corrupts it. The safe deploy has downtime. The decision is made from memory, under pressure, without a rule.',
    cost: 'Downtime or corruption.',
  },
  {
    icon: EyeOff,
    title: 'Invisible state',
    body: 'The registry says one release, nginx points at another, and the container is a third. A crash between those facts is discovered by a user, not by the tool.',
    cost: 'Nobody can say what is live.',
  },
]

const principles = [
  {
    n: '01',
    title: 'One source of truth',
    body: 'One TOML per app. The nginx config is generated from it, validated as a whole, and never hand-edited. The registry commits last, after traffic moves.',
  },
  {
    n: '02',
    title: 'One tested cutover',
    body: 'The new release starts on the free port, passes container health and an endpoint probe, then the front door moves. The old release is stopped only after both gates pass.',
  },
  {
    n: '03',
    title: 'One recoverable state',
    body: 'A journal records the phase before the flip. Rollback is a deploy through the same gate. A crashed deploy is repaired from evidence, or refused with a specific reason.',
  },
]

function Why() {
  return (
    <>
      <PageHero
        eyebrow="Why swapdock"
        title="Your deploy is the weakest part of your stack."
        lead="The box is fine. The routine around it is not: a hand-edited front door, a rollback nobody has tested, and state that makes every deploy a gamble."
      >
        <div className="flex flex-wrap gap-3">
          <Button className="h-11 rounded-lg px-5 text-[13.5px]" render={<Link to="/path/" />}>
            See the path <ArrowRight className="size-4" />
          </Button>
          <Button variant="outline" className="h-11 rounded-lg px-5 text-[13.5px]" render={<Link to="/how-it-works/" />}>
            How it works
          </Button>
        </div>
      </PageHero>

      <Section>
        <Container>
          <SectionHead
            eyebrow="Four ways it bites"
            title="The failures are predictable."
            lead="None of these are exotic. They are the normal outcome of a deploy routine made of memory and shell history."
          />
          <div className="grid gap-3 md:grid-cols-2">
            {failures.map((f, i) => (
              <Reveal key={f.title} delay={i * 50}>
                <div className="surface-card h-full p-6 md:p-7">
                  <f.icon className="size-4 text-accent" />
                  <h3 className="mt-5 text-[16px] font-semibold">{f.title}</h3>
                  <p className="mt-2.5 text-[13px] leading-relaxed text-muted-foreground">{f.body}</p>
                  <p className="mt-5 border-t border-border pt-3 text-[11px] font-medium text-critical">
                    {f.cost}
                  </p>
                </div>
              </Reveal>
            ))}
          </div>
        </Container>
      </Section>

      <Section bordered className="bg-surface/25">
        <Container>
          <SectionHead
            eyebrow="The shift"
            title="Three things stop being your job."
            lead="swapdock does not make the box more complex. It moves the dangerous parts into a checked sequence."
          />
          <div className="grid gap-8 md:grid-cols-3">
            {principles.map((p, i) => (
              <Reveal key={p.n} delay={i * 60}>
                <div className="border-t border-border pt-5">
                  <span className="mono text-[10px] text-subtle-foreground">{p.n}</span>
                  <h3 className="mt-4 text-[16px] font-semibold">{p.title}</h3>
                  <p className="mt-2 text-[13px] leading-relaxed text-muted-foreground">{p.body}</p>
                </div>
              </Reveal>
            ))}
          </div>
        </Container>
      </Section>

      <Section>
        <Container>
          <SectionHead
            eyebrow="Positioning"
            title="What this is not."
            lead="The constraint is the feature. swapdock is for one host, and it stays small enough to reason about."
          />
          <div className="grid gap-3 md:grid-cols-3">
            {[
              ['Not Kubernetes', 'No scheduler, no control plane, no cluster to babysit. One host, one registry, one front door.'],
              ['Not a PaaS', 'No hosted account and no vendor in the traffic path. A binary, your files, your box.'],
              ['Not multi-host', 'No distributed consensus, no cross-host networking. The failure modes stay local and observable.'],
            ].map(([title, body]) => (
              <div key={title} className="rounded-xl border border-border p-6">
                <h3 className="text-[14px] font-semibold">{title}</h3>
                <p className="mt-2 text-[12.5px] leading-relaxed text-muted-foreground">{body}</p>
              </div>
            ))}
          </div>
        </Container>
      </Section>

      <FinalCta
        title="Start with the failure you already have."
        lead="One app, one cutover, one rollback drill. The path takes about an hour."
      />
    </>
  )
}
