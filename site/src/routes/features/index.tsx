import { Link, createFileRoute } from '@tanstack/react-router'
import { ArrowRight } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Container, PageHero, Section, SectionHead } from '@/components/section'
import { FeatureCard } from '@/components/marketing/feature-card'
import { FinalCta } from '@/components/marketing/final-cta'
import { Reveal } from '@/components/reveal'
import { FEATURES } from '@/content/features'
import { pageHead } from '@/lib/seo'

export const Route = createFileRoute('/features/')({
  head: () =>
    pageHead({
      title: 'Features — swapdock',
      description:
        'Zero-downtime cutovers, SQLite-safe replace deploys, one-command rollback, generated nginx, crash recovery, a run log, locks, onboarding and day-two operations.',
      path: '/features',
    }),
  component: Features,
})

function Features() {
  return (
    <>
      <PageHero
        eyebrow="Features"
        title="Small tool. Sharp edges."
        lead="Nine guarantees, each backed by a rule in the validator, a step in the run log, or a test that fails if the rule breaks."
      >
        <Button className="h-11 rounded-lg px-5 text-[13.5px]" render={<Link to="/path/" />}>
          Start the path <ArrowRight className="size-4" />
        </Button>
      </PageHero>

      <Section>
        <Container>
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {FEATURES.map((f, i) => (
              <Reveal key={f.slug} delay={(i % 3) * 50}>
                <FeatureCard feature={f} />
              </Reveal>
            ))}
          </div>
        </Container>
      </Section>

      <Section bordered className="bg-surface/25">
        <Container>
          <SectionHead
            eyebrow="The guarantees"
            title="What the tool will not do."
            lead="A feature list is easy. The useful part is the refusal."
          />
          <div className="grid gap-3 md:grid-cols-2">
            {[
              ['It will not swap a stateful app.', 'writes_state forces replace. There is no override.'],
              ['It will not apply a broken config.', 'nginx -t runs before the reload, and the backup is restored on failure.'],
              ['It will not guess after a crash.', 'reconcile proves nginx, the image, and health before it writes.'],
              ['It will not delete a rollback image.', 'prune keeps the release, old_release, and the newest rollback tags.'],
            ].map(([title, body]) => (
              <div key={title} className="rounded-xl border border-border bg-card p-5">
                <h3 className="text-[13.5px] font-semibold">{title}</h3>
                <p className="mt-1.5 text-[12px] leading-relaxed text-muted-foreground">{body}</p>
              </div>
            ))}
          </div>
          <Link to="/how-it-works/" className="mt-7 inline-flex items-center gap-1.5 text-[12px] font-medium text-accent hover:underline">
            See the full loop <ArrowRight className="size-3.5" />
          </Link>
        </Container>
      </Section>

      <FinalCta />
    </>
  )
}
