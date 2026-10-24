import { Link, createFileRoute } from '@tanstack/react-router'
import { ArrowRight } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Container, Eyebrow, Section, SectionHead } from '@/components/section'
import { ProductBoard } from '@/components/marketing/product-board'
import { CutoverRail } from '@/components/marketing/cutover-rail'
import { FeatureCard } from '@/components/marketing/feature-card'
import { PathRail } from '@/components/marketing/path-rail'
import { CompareTable } from '@/components/marketing/compare-table'
import { Proof } from '@/components/marketing/proof'
import { FinalCta } from '@/components/marketing/final-cta'
import { Reveal } from '@/components/reveal'
import { CodeBlock } from '@/components/marketing/code-block'
import { FEATURES } from '@/content/features'
import { pageHead } from '@/lib/seo'

export const Route = createFileRoute('/')({
  head: () =>
    pageHead({
      title: 'swapdock — deploy like a platform, still one box',
      description:
        'Blue/green deploys, health-gated cutovers and one-command rollbacks for a single Linux host. Built on Docker Compose and nginx. Open source, MIT.',
      path: '/',
    }),
  component: Home,
})

const homeFeatures = ['zero-downtime', 'stateful-apps', 'rollback', 'nginx', 'recovery', 'operations']

const stack = ['Docker Compose v2', 'nginx', 'Cloudflare Tunnel', 'Tailscale', 'systemd', 'SQLite']

function Home() {
  return (
    <>
      <section className="relative overflow-hidden pb-20 pt-14 md:pb-28 md:pt-24">
        <div className="grid-backdrop pointer-events-none absolute inset-0 opacity-70" />
        <div className="glow -top-40 left-1/3 h-[420px] w-[620px] bg-accent/15" />
        <Container>
          <div className="grid grid-cols-1 items-center gap-14 lg:grid-cols-[1.05fr_.95fr] lg:gap-10">
            <div>
              <span className="inline-flex items-center gap-2 rounded-full border border-border bg-card/70 px-3 py-1.5 text-[11px] font-medium text-muted-foreground">
                <span className="live-dot" />
                Open source · MIT · v0.2
              </span>

              <h1 className="display-1 mt-7 max-w-[12ch]">
                Deploy like a platform. <span className="text-accent">Still one box.</span>
              </h1>

              <p className="mt-6 max-w-[54ch] text-[17px] leading-relaxed text-muted-foreground">
                swapdock adds blue/green cutovers, health gates and one-command rollbacks to the
                Docker Compose and nginx you already run. No Kubernetes. No agents. No hand-edited
                nginx.
              </p>

              <div className="mt-8 flex flex-wrap items-center gap-3">
                <Button className="h-11 rounded-lg px-5 text-[13.5px]" render={<Link to="/path/" />}>
                  Start the path <ArrowRight className="size-4" />
                </Button>
                <Button
                  variant="outline"
                  className="h-11 rounded-lg px-5 text-[13.5px]"
                  render={<Link to="/how-it-works/" />}
                >
                  See how it works
                </Button>
              </div>

              <p className="mt-6 text-[11.5px] text-subtle-foreground">
                Ubuntu · Debian <span className="mx-1.5 opacity-40">/</span> Compose v2
                <span className="mx-1.5 opacity-40">/</span> nginx 1.24+
                <span className="mx-1.5 opacity-40">/</span> one host, on purpose
              </p>
            </div>

            <ProductBoard />
          </div>
        </Container>
      </section>

      <div className="border-y border-border bg-surface/40">
        <Container className="flex flex-wrap items-center gap-x-7 gap-y-3 py-4">
          <span className="label-eyebrow">Runs with</span>
          {stack.map((item) => (
            <span key={item} className="flex items-center gap-2 text-[11.5px] text-muted-foreground">
              <span className="size-1 rounded-full bg-accent/70" />
              {item}
            </span>
          ))}
          <span className="ml-auto flex items-center gap-2 text-[11px] font-medium text-live">
            <span className="size-1 rounded-full bg-live" />
            530+ tests · no telemetry
          </span>
        </Container>
      </div>

      <Section bordered className="bg-surface/25">
        <Container>
          <SectionHead
            eyebrow="The single-box tax"
            title={
              <>
                One server shouldn’t mean
                <br className="hidden sm:block" /> amateur deploys.
              </>
            }
            lead="You chose one box because it is simple to run. The deploy should not be the part that isn’t."
          />

          <div className="grid gap-3 md:grid-cols-3">
            {[
              {
                n: '01',
                title: 'nginx drift',
                body: 'The real routing lives in a file nobody reviews. One typo and every app behind the front door is down, not just the one you touched.',
              },
              {
                n: '02',
                title: 'The 3am rollback',
                body: 'The previous image is somewhere. The command is in shell history. The health check is your memory. Nothing about that is reproducible.',
              },
              {
                n: '03',
                title: 'The stateful trap',
                body: 'Two containers, one SQLite file. The fast deploy corrupts it; the safe deploy has downtime. The choice is made from memory, at the worst time.',
              },
            ].map((item, i) => (
              <Reveal key={item.n} delay={i * 60}>
                <div className="surface-card h-full p-6">
                  <span className="mono text-[10px] text-subtle-foreground">{item.n}</span>
                  <h3 className="mt-6 text-[15px] font-semibold">{item.title}</h3>
                  <p className="mt-2 text-[13px] leading-relaxed text-muted-foreground">{item.body}</p>
                </div>
              </Reveal>
            ))}
          </div>

          <Link
            to="/why/"
            className="mt-8 inline-flex items-center gap-1.5 text-[12.5px] font-medium text-accent hover:underline"
          >
            See what it costs in practice <ArrowRight className="size-3.5" />
          </Link>
        </Container>
      </Section>

      <Section>
        <Container>
          <div className="grid grid-cols-1 gap-10 lg:grid-cols-[.8fr_1.2fr] lg:items-end">
            <SectionHead
              eyebrow="The path"
              title="From compose up to boring deploys."
              lead="A clear route from the box you have to the deploy you want. Each stage has a command, a result, and a time."
              className="mb-0"
            />
            <p className="max-w-md text-[13px] leading-relaxed text-muted-foreground lg:justify-self-end">
              You do not need to migrate everything at once. Start with one app, prove the cutover,
              then bring the stateful ones and the rest of the estate.
            </p>
          </div>
          <Reveal className="mt-12">
            <PathRail />
          </Reveal>
        </Container>
      </Section>

      <Section bordered className="bg-surface/25">
        <Container>
          <div className="grid grid-cols-1 gap-12 lg:grid-cols-2 lg:items-center">
            <div>
              <SectionHead
                eyebrow="How it works"
                title="Three decisions, then a boring deploy."
                lead="Describe the app once. Deploy with one command. Roll back through the same gate."
                className="mb-8"
              />

              <div className="space-y-6">
                {[
                  ['01', 'Describe the app once', 'One TOML: hostname, front port, slot, Compose service, health path. The registry is the source of truth.'],
                  ['02', 'Deploy', 'The new release starts on the free port, passes container health and an endpoint probe, then nginx moves.'],
                  ['03', 'Roll back the same way', 'The previous release is deployed through the same strategy and the same health gate. No special path.'],
                ].map(([n, title, body]) => (
                  <div key={n} className="flex gap-4 border-t border-border pt-5">
                    <span className="mono text-[10px] text-subtle-foreground">{n}</span>
                    <div>
                      <h3 className="text-[14px] font-semibold">{title}</h3>
                      <p className="mt-1.5 text-[12.5px] leading-relaxed text-muted-foreground">{body}</p>
                    </div>
                  </div>
                ))}
              </div>

              <Link
                to="/how-it-works/"
                className="mt-7 inline-flex items-center gap-1.5 text-[12.5px] font-medium text-accent hover:underline"
              >
                Read the full loop <ArrowRight className="size-3.5" />
              </Link>
            </div>

            <div className="space-y-4">
              <div className="surface-card p-5">
                <div className="mb-4 flex items-center justify-between">
                  <span className="label-eyebrow">The cutover</span>
                  <span className="mono text-[10px] text-subtle-foreground">15 s drain</span>
                </div>
                <CutoverRail orientation="vertical" />
                <p className="mt-4 text-[11.5px] leading-relaxed text-muted-foreground">
                  The registry commit is last, after traffic already moved. A failed gate leaves the
                  old release serving.
                </p>
              </div>
              <CodeBlock title="one command" lines={['sudo swapdock --registry . up blog --release 9c1f2ab']} />
            </div>
          </div>
        </Container>
      </Section>

      <Section>
        <Container>
          <div className="flex flex-wrap items-end justify-between gap-6">
            <SectionHead
              eyebrow="Features"
              title="Platform habits, without the platform."
              lead="Each feature is a guarantee, not a checklist item."
              className="mb-0"
            />
            <Link
              to="/features/"
              className="mb-2 inline-flex items-center gap-1.5 text-[12.5px] font-medium text-accent hover:underline"
            >
              All features <ArrowRight className="size-3.5" />
            </Link>
          </div>

          <div className="mt-10 grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {FEATURES.filter((f) => homeFeatures.includes(f.slug)).map((f, i) => (
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
            eyebrow="Built for production"
            title="The parts you would otherwise write at 2am."
            lead="Small on purpose, strict where it matters: atomic writes, fsynced records, real locks, refused guesses."
          />
          <Proof />
        </Container>
      </Section>

      <Section>
        <Container>
          <SectionHead
            eyebrow="The difference"
            title="What changes on Monday."
            lead="Not a different architecture. The same box, with the dangerous parts handled."
          />
          <CompareTable />
        </Container>
      </Section>

      <FinalCta />
    </>
  )
}
