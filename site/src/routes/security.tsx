import { createFileRoute } from '@tanstack/react-router'
import { KeyRound, Lock, ShieldCheck } from 'lucide-react'
import { Container, PageHero, Section, SectionHead } from '@/components/section'
import { PrivilegeDiagram } from '@/components/marketing/diagrams'
import { FinalCta } from '@/components/marketing/final-cta'
import { REPO } from '@/lib/site'
import { pageHead } from '@/lib/seo'

export const Route = createFileRoute('/security')({
  head: () =>
    pageHead({
      title: 'Security & trust — swapdock',
      description:
        'Root stays in a small broker, the API uses one bearer token, there is no telemetry, and every privileged operation is narrow and logged.',
      path: '/security',
    }),
  component: Security,
})

const limits = [
  ['Registry file', 'parsed with deny_unknown_fields; bad keys fail the app'],
  ['Lock wait', '300 s, then a named timeout'],
  ['Container health', '120 s, then a named failure'],
  ['Front probe', '10 tries, 500 ms apart'],
  ['Command timeout', '120 s default, per-step override'],
  ['Dry run', 'records the plan, spawns nothing, mutates nothing'],
]

function Security() {
  return (
    <>
      <PageHero
        eyebrow="Security & trust"
        title="Root stays in a small box."
        lead="A deploy tool writes nginx config and talks to Docker. That is real power. swapdock keeps it behind a small surface, a token, and a log."
      >
        <p className="flex items-center gap-2 text-[12px] text-muted-foreground">
          <ShieldCheck className="size-4 text-live" />
          No telemetry · no account · MIT
        </p>
      </PageHero>

      <Section>
        <Container>
          <SectionHead
            eyebrow="Privilege"
            title="Two shapes, one narrow surface."
            lead="Run the CLI as root with a sudoers entry, or run the API unprivileged and put the privileged calls behind a local broker."
          />
          <PrivilegeDiagram />
          <p className="mt-5 max-w-3xl text-[12.5px] leading-relaxed text-muted-foreground">
            The broker accepts two things: a rendered nginx config to install and reload, and a narrow
            allowlist of Docker commands. The socket is local; the API token is the authorization;{' '}
            <span className="mono text-foreground">/run/swapdock</span> must not be world-writable.
          </p>
        </Container>
      </Section>

      <Section bordered className="bg-surface/25">
        <Container>
          <SectionHead
            eyebrow="The model"
            title="What is exposed, and to whom."
            lead="No secrets in argv, no shell strings from the registry, no unauthenticated mutation."
          />
          <div className="grid gap-3 md:grid-cols-3">
            {[
              [KeyRound, 'One bearer token', 'The API compares one token in constant time. The console trades it for a short session. GET /health is the only open route.'],
              [Lock, 'No shell from config', 'Backup and restore commands are argv, never a shell string. Unsafe argv is refused before it runs.'],
              [ShieldCheck, 'Narrow privileged calls', 'The broker owns nginx apply and an allowlisted set of Docker commands. The API never needs the Docker group.'],
            ].map(([Icon, title, body]) => {
              const I = Icon as typeof KeyRound
              return (
                <div key={title as string} className="rounded-xl border border-border bg-card p-5">
                  <I className="size-4 text-accent" />
                  <h3 className="mt-4 text-[13.5px] font-semibold">{title as string}</h3>
                  <p className="mt-1.5 text-[12px] leading-relaxed text-muted-foreground">{body as string}</p>
                </div>
              )
            })}
          </div>
        </Container>
      </Section>

      <Section>
        <Container>
          <div className="grid grid-cols-1 gap-10 lg:grid-cols-[1fr_1.2fr] lg:gap-16">
            <div>
              <SectionHead
                eyebrow="Limits"
                title="Bounded waits, bounded files."
                lead="Every wait has a ceiling and a named failure. Nothing can hang forever or grow without a bound."
                className="mb-6"
              />
              <p className="text-[12.5px] leading-relaxed text-muted-foreground">
                The detailed model is in{' '}
                <a href={`${REPO}/blob/main/SECURITY.md`} className="text-accent underline underline-offset-2" target="_blank" rel="noreferrer">
                  SECURITY.md
                </a>
                . Report a vulnerability privately through GitHub advisories, not a public issue.
              </p>
            </div>
            <div className="overflow-hidden rounded-xl border border-border bg-card">
              <table className="w-full border-collapse text-left">
                <thead>
                  <tr className="border-b border-border">
                    <th className="label-eyebrow px-5 py-3.5">Surface</th>
                    <th className="label-eyebrow px-5 py-3.5">Bound</th>
                  </tr>
                </thead>
                <tbody>
                  {limits.map(([surface, bound]) => (
                    <tr key={surface} className="border-b border-border last:border-0">
                      <th className="px-5 py-3 text-[12.5px] font-medium">{surface}</th>
                      <td className="px-5 py-3 text-[12px] text-muted-foreground">{bound}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        </Container>
      </Section>

      <FinalCta title="Trust is a smaller surface." lead="Read the threat model before you give a tool the front door." />
    </>
  )
}
