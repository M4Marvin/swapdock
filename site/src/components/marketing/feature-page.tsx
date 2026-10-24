import { Link } from '@tanstack/react-router'
import { ArrowRight, Check } from 'lucide-react'
import type { FeatureDetail } from '@/content/features'
import { FEATURES } from '@/content/features'
import { Button } from '@/components/ui/button'
import { Container, PageHero, Section, SectionHead } from '@/components/section'
import { FeatureDiagram } from '@/components/marketing/diagrams'
import { FeatureCard } from '@/components/marketing/feature-card'
import { CodeBlock } from '@/components/marketing/code-block'
import { FinalCta } from '@/components/marketing/final-cta'
import { Reveal } from '@/components/reveal'

export function FeaturePage({ detail }: { detail: FeatureDetail }) {
  const related = detail.related
    .map((slug) => FEATURES.find((f) => f.slug === slug))
    .filter((f): f is NonNullable<typeof f> => Boolean(f))

  return (
    <>
      <PageHero
        eyebrow={
          <span className="flex items-center gap-2">
            <Link to="/features/" className="transition-colors hover:text-foreground">
              Features
            </Link>
            <span className="text-border-strong">/</span>
            <span className="text-foreground">{detail.kicker}</span>
          </span>
        }
        title={detail.title}
        lead={detail.lead}
      >
        <div className="flex flex-wrap gap-3">
          <Button className="h-11 rounded-lg px-5 text-[13.5px]" render={<Link to="/docs/" />}>
            Read the quickstart <ArrowRight className="size-4" />
          </Button>
          <Button variant="outline" className="h-11 rounded-lg px-5 text-[13.5px]" render={<Link to="/path/" />}>
            Start the path
          </Button>
        </div>
      </PageHero>

      <Section>
        <Container>
          <div className="grid gap-10 lg:grid-cols-[1fr_1fr] lg:gap-16">
            <div>
              <p className="label-eyebrow">The problem</p>
              <h2 className="display-3 mt-4">{detail.problem.title}</h2>
            </div>
            <div className="space-y-4">
              {detail.problem.body.map((p) => (
                <p key={p} className="text-[14px] leading-relaxed text-muted-foreground">
                  {p}
                </p>
              ))}
            </div>
          </div>
        </Container>
      </Section>

      <Section bordered className="bg-surface/25">
        <Container>
          <SectionHead eyebrow="The mechanism" title={detail.mechanism.title} lead={detail.mechanism.body[0]} />
          <div className="grid grid-cols-1 items-start gap-8 lg:grid-cols-[1.1fr_.9fr]">
            <FeatureDiagram kind={detail.mechanism.diagram} />
            <div>
              <div className="space-y-4">
                {detail.mechanism.body.slice(1).map((p) => (
                  <p key={p} className="text-[13px] leading-relaxed text-muted-foreground">
                    {p}
                  </p>
                ))}
              </div>
              <ul className="mt-6 space-y-2.5">
                {detail.mechanism.bullets.map((b) => (
                  <li key={b} className="flex items-start gap-2.5 text-[12.5px] leading-relaxed">
                    <Check className="mt-0.5 size-3.5 shrink-0 text-live" strokeWidth={3} />
                    {b}
                  </li>
                ))}
              </ul>
              {detail.mechanism.code ? (
                <CodeBlock
                  className="mt-6"
                  title={detail.mechanism.code.title}
                  lines={detail.mechanism.code.lines}
                  prompt={detail.mechanism.code.prompt}
                />
              ) : null}
            </div>
          </div>
        </Container>
      </Section>

      <Section>
        <Container>
          <SectionHead eyebrow="What you get" title="The outcome, not the feature list." />
          <div className="grid gap-3 sm:grid-cols-3">
            {detail.outcomes.map((o, i) => (
              <Reveal key={o} delay={i * 50}>
                <div className="h-full rounded-xl border border-border bg-card p-5">
                  <Check className="size-4 text-live" strokeWidth={3} />
                  <p className="mt-4 text-[13px] leading-relaxed">{o}</p>
                </div>
              </Reveal>
            ))}
          </div>
        </Container>
      </Section>

      <Section bordered className="bg-surface/25">
        <Container>
          <div className="mb-8 flex items-end justify-between gap-4">
            <SectionHead eyebrow="Related" title="Keep going." className="mb-0" />
            <Link to="/features/" className="mb-2 hidden items-center gap-1.5 text-[12px] font-medium text-accent hover:underline sm:inline-flex">
              All features <ArrowRight className="size-3.5" />
            </Link>
          </div>
          <div className="grid gap-3 md:grid-cols-3">
            {related.map((f) => (
              <FeatureCard key={f.slug} feature={f} />
            ))}
          </div>
        </Container>
      </Section>

      <FinalCta
        title="Try it on one app."
        lead="The first cutover takes minutes. The rollback drill is the point."
      />
    </>
  )
}
