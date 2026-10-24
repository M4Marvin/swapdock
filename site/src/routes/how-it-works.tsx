import { Link, createFileRoute } from '@tanstack/react-router'
import { ArrowRight, ShieldCheck } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Container, PageHero, Section, SectionHead } from '@/components/section'
import { CutoverRail } from '@/components/marketing/cutover-rail'
import { StrategiesDiagram } from '@/components/marketing/diagrams'
import { CodeBlock } from '@/components/marketing/code-block'
import { Reveal } from '@/components/reveal'
import { FinalCta } from '@/components/marketing/final-cta'
import { pageHead } from '@/lib/seo'

export const Route = createFileRoute('/how-it-works')({
  head: () =>
    pageHead({
      title: 'How swapdock works — the deploy loop',
      description:
        'Twenty recorded steps: locks, preflight, a green container, a health gate, a tested nginx flip, a front-door probe, a drain, and a registry commit last.',
      path: '/how-it-works',
    }),
  component: How,
})

const phases = [
  {
    name: 'Prepare',
    steps: [
      ['lock', 'estate, then app; held through commit'],
      ['resolve-release', 'flag or recorded release; refuse an accidental redeploy'],
      ['pull', 'image, or verify a local one exists'],
      ['swap-preflight', 'merged Compose model; named volumes and deps block'],
      ['backup', 'optional pre-deploy hook, argv not shell'],
    ],
  },
  {
    name: 'Start',
    steps: [
      ['green-start', 'own Compose project, free port of the pair'],
      ['green-override', 'only the container name; image and port come from env'],
      ['per-release-project', 'the old project is never the desired state'],
    ],
  },
  {
    name: 'Gate',
    steps: [
      ['health-wait-discover', 'find the one container publishing the port'],
      ['health-wait-inspect', 'docker inspect Health.Status'],
      ['health-wait-healthy', 'must be healthy within 120 s'],
      ['health-wait-probe', 'HTTP GET on the candidate port'],
    ],
  },
  {
    name: 'Flip',
    steps: [
      ['render-validate', 'simulate the post-commit registry, validate every app'],
      ['apply-stage', 'write a draft next to the target'],
      ['apply-backup', 'copy the live file to .bak'],
      ['apply-commit', 'atomic rename over the target'],
      ['nginx-test', 'nginx -t on the real full config'],
      ['nginx-reload', 'signal, not restart'],
    ],
  },
  {
    name: 'Verify',
    steps: [
      ['probe-front', '10 tries, Host header, 500 ms apart'],
      ['drain-wait', '15 s for old keepalives'],
      ['retain-old-image', 'tag the outgoing image for rollback'],
    ],
  },
  {
    name: 'Commit',
    steps: [
      ['stop-old', 'stop and remove by published port'],
      ['registry-commit', 'advance release and live_port; last'],
      ['journal-clear', 'remove the pending record after the save'],
      ['cleanup-green', 'remove the retired project and its network'],
    ],
  },
]

const matrix = [
  ['Before the flip', 'Traffic stays on the old release. A started candidate may remain for inspection.', 'Old release serving'],
  ['After the flip, swap', 'Front door returns to the old port, candidate stopped, green project removed.', 'Old release serving'],
  ['After the stop, replace', 'Previous release restarted and gated again. Both failures named in one error.', 'Previous release, if it passes'],
  ['Static deploy', 'Previous symlink restored, unless the registry was already saved.', 'Previous files'],
  ['Between stop and commit', 'Nothing is rolled back automatically. The journal stays; reconcile decides.', 'Evidence + refusal'],
]

function How() {
  return (
    <>
      <PageHero
        eyebrow="How it works"
        title="Twenty steps you never see."
        lead="A deploy is not a restart. It is a sequence with locks, a second container, two gates, an atomic config change, a drain, and a commit that happens last."
      >
        <div className="flex flex-wrap gap-3">
          <Button className="h-11 rounded-lg px-5 text-[13.5px]" render={<Link to="/path/" />}>
            Start the path <ArrowRight className="size-4" />
          </Button>
          <Button variant="outline" className="h-11 rounded-lg px-5 text-[13.5px]" render={<Link to="/features/" />}>
            Browse features
          </Button>
        </div>
      </PageHero>

      <Section>
        <Container>
          <div className="surface-card p-5 md:p-7">
            <div className="mb-5 flex flex-wrap items-center justify-between gap-3">
              <span className="label-eyebrow">The cutover</span>
              <span className="mono text-[10.5px] text-subtle-foreground">swap · stateless path</span>
            </div>
            <CutoverRail />
            <p className="mt-5 max-w-3xl text-[12px] leading-relaxed text-muted-foreground">
              The old container is stopped only after the new release passes container health and the
              public front door. The registry commit is last: if the process dies before it, the
              journal and <span className="mono text-foreground">reconcile</span> decide what is true.
            </p>
          </div>
        </Container>
      </Section>

      <Section bordered className="bg-surface/25">
        <Container>
          <SectionHead
            eyebrow="The run log"
            title="Every step has a name."
            lead="The run log groups by these identifiers, so a deploy is auditable and resumable. The order is the contract."
          />

          <div className="grid gap-3 md:grid-cols-2 lg:grid-cols-3">
            {phases.map((phase, i) => (
              <Reveal key={phase.name} delay={i * 40}>
                <div className="h-full rounded-xl border border-border bg-card p-5">
                  <div className="flex items-center justify-between">
                    <h3 className="text-[13.5px] font-semibold">{phase.name}</h3>
                    <span className="mono text-[9.5px] text-subtle-foreground">0{i + 1}</span>
                  </div>
                  <ul className="mt-4 space-y-2.5">
                    {phase.steps.map(([step, note]) => (
                      <li key={step} className="border-t border-border pt-2.5">
                        <p className="mono text-[10.5px] text-foreground">{step}</p>
                        <p className="mt-1 text-[11px] leading-relaxed text-muted-foreground">{note}</p>
                      </li>
                    ))}
                  </ul>
                </div>
              </Reveal>
            ))}
          </div>
        </Container>
      </Section>

      <Section>
        <Container>
          <div className="grid grid-cols-1 gap-12 lg:grid-cols-[.95fr_1.05fr] lg:items-center">
            <div>
              <SectionHead
                eyebrow="Two strategies"
                title="Swap when it is safe. Replace when it is not."
                lead="The registry decides. The validator enforces the rule before anything starts."
                className="mb-6"
              />
              <div className="space-y-4">
                <p className="text-[13px] leading-relaxed text-muted-foreground">
                  <span className="font-medium text-foreground">swap</span> runs two containers and
                  alternates the back ports. It needs a free port and no shared file.
                </p>
                <p className="text-[13px] leading-relaxed text-muted-foreground">
                  <span className="font-medium text-foreground">replace</span> stops first and starts on
                  the same port. It has a short gap, but one writer. A SQLite file cannot be open in two
                  containers, so <span className="mono">writes_state</span> forces this path.
                </p>
              </div>
              <CodeBlock
                className="mt-6"
                title="the rule"
                prompt={false}
                lines={['writes_state = true', 'strategy = "replace"']}
              />
            </div>
            <StrategiesDiagram />
          </div>
        </Container>
      </Section>

      <Section bordered className="bg-surface/25">
        <Container>
          <SectionHead
            eyebrow="The health gate"
            title="A port is not a health check."
            lead="Two phases answer two different questions. A missing healthcheck is a failure, not a pass."
          />
          <div className="grid gap-3 md:grid-cols-3">
            {[
              ['Discover', 'docker ps finds the container publishing the port. Zero is an error; two is an error.'],
              ['Inspect', 'Health.Status must become healthy within 120 s. No healthcheck means no gate.'],
              ['Probe', 'GET on 127.0.0.1:port + the health path. 30 tries, 2 s apart, 2xx only.'],
            ].map(([title, body], i) => (
              <div key={title} className="rounded-xl border border-border bg-card p-5">
                <span className="mono text-[10px] text-accent">0{i + 1}</span>
                <h3 className="mt-3 text-[13.5px] font-semibold">{title}</h3>
                <p className="mt-1.5 text-[12px] leading-relaxed text-muted-foreground">{body}</p>
              </div>
            ))}
          </div>
        </Container>
      </Section>

      <Section>
        <Container>
          <SectionHead
            eyebrow="Failure"
            title="Every failure leaves a known state."
            lead="The compensation path depends on where the deploy stopped. The journal covers the window with no automatic answer."
          />
          <div className="overflow-x-auto rounded-xl border border-border bg-card">
            <table className="w-full min-w-[680px] border-collapse text-left">
              <thead>
                <tr className="border-b border-border">
                  <th className="label-eyebrow px-5 py-4">Where it fails</th>
                  <th className="label-eyebrow px-5 py-4">What swapdock does</th>
                  <th className="label-eyebrow px-5 py-4">What is serving</th>
                </tr>
              </thead>
              <tbody>
                {matrix.map(([where, what, serving]) => (
                  <tr key={where} className="border-b border-border last:border-0">
                    <th className="px-5 py-3.5 text-[12.5px] font-medium">{where}</th>
                    <td className="px-5 py-3.5 text-[12.5px] text-muted-foreground">{what}</td>
                    <td className="px-5 py-3.5 text-[12.5px]">
                      <span className="flex items-start gap-2">
                        <ShieldCheck className="mt-0.5 size-3.5 shrink-0 text-live" />
                        {serving}
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <p className="mt-4 text-[11.5px] text-subtle-foreground">
            A crash between the stop and the commit is the one case with no automatic rollback. That is
            deliberate: <Link to="/features/recovery/" className="text-accent underline underline-offset-2">reconcile</Link>{' '}
            proves the state before it writes.
          </p>
        </Container>
      </Section>

      <FinalCta
        title="Read the loop, then run it once."
        lead="The shortest path to understanding is a deploy you can roll back."
      />
    </>
  )
}
