import { Link, createFileRoute } from '@tanstack/react-router'
import { ArrowRight, BookOpen, Boxes, Terminal, Wrench } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Container, PageHero, Section, SectionHead } from '@/components/section'
import { CodeBlock } from '@/components/marketing/code-block'
import { FinalCta } from '@/components/marketing/final-cta'
import { DOCS, LAUNCH, REPO } from '@/lib/site'
import { pageHead } from '@/lib/seo'

export const Route = createFileRoute('/docs')({
  head: () =>
    pageHead({
      title: 'Docs & quickstart — swapdock',
      description:
        'Install, scaffold one app, validate the registry, check the host, and make the first health-gated deploy. The quickstart plus the full docs.',
      path: '/docs',
    }),
  component: Docs,
})

const cards = [
  {
    icon: Terminal,
    title: 'Install',
    body: 'Build the dist tree, run the installer, and see what it leaves alone.',
    href: `${REPO}/blob/main/docs/install.md`,
    label: 'docs/install.md',
  },
  {
    icon: BookOpen,
    title: 'Quickstart',
    body: 'One app end to end: init, edit, validate, doctor, deploy.',
    href: `${REPO}/blob/main/docs/quickstart.md`,
    label: 'docs/quickstart.md',
  },
  {
    icon: Boxes,
    title: 'Concepts',
    body: 'Registry, ports, strategies, the journal, and the run log.',
    href: '/how-it-works',
    label: 'How it works',
    internal: true,
  },
  {
    icon: Wrench,
    title: 'CLI & API',
    body: 'Every command, and the endpoints that mirror them.',
    href: `${REPO}#readme`,
    label: 'README',
  },
]

function Docs() {
  return (
    <>
      <PageHero
        eyebrow="Docs"
        title="Ten minutes to the first boring deploy."
        lead="There is no published release yet. Build from source, install, describe one app, and check the host before the first cutover."
      >
        <div className="flex flex-wrap gap-3">
          <Button className="h-11 rounded-lg px-5 text-[13.5px]" render={<a href={`${REPO}/blob/main/docs/quickstart.md`} target="_blank" rel="noreferrer" />}>
            Open the quickstart <ArrowRight className="size-4" />
          </Button>
          <Button variant="outline" className="h-11 rounded-lg px-5 text-[13.5px]" render={<a href={LAUNCH} target="_blank" rel="noreferrer" />}>
            Launch gates
          </Button>
        </div>
      </PageHero>

      <Section>
        <Container>
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
            {cards.map((card) =>
              card.internal ? (
                <Link key={card.title} to="/how-it-works/" className="group rounded-xl border border-border bg-card p-5 transition-colors hover:border-border-strong hover:bg-elevated">
                  <card.icon className="size-4 text-accent" />
                  <h2 className="mt-4 text-[14px] font-semibold">{card.title}</h2>
                  <p className="mt-1.5 text-[12px] leading-relaxed text-muted-foreground">{card.body}</p>
                  <span className="mt-4 inline-flex items-center gap-1 text-[11px] font-medium text-accent">
                    {card.label} <ArrowRight className="size-3" />
                  </span>
                </Link>
              ) : (
                <a
                  key={card.title}
                  href={card.href}
                  target="_blank"
                  rel="noreferrer"
                  className="group rounded-xl border border-border bg-card p-5 transition-colors hover:border-border-strong hover:bg-elevated"
                >
                  <card.icon className="size-4 text-accent" />
                  <h2 className="mt-4 text-[14px] font-semibold">{card.title}</h2>
                  <p className="mt-1.5 text-[12px] leading-relaxed text-muted-foreground">{card.body}</p>
                  <span className="mt-4 inline-flex items-center gap-1 text-[11px] font-medium text-accent">
                    {card.label} <ArrowRight className="size-3" />
                  </span>
                </a>
              ),
            )}
          </div>
        </Container>
      </Section>

      <Section bordered className="bg-surface/25">
        <Container>
          <div className="grid grid-cols-1 gap-10 lg:grid-cols-[.85fr_1.15fr] lg:gap-16">
            <div>
              <SectionHead
                eyebrow="Quickstart"
                title="Build, install, describe, check."
                lead="The first four commands are the whole onboarding. Deploy is a separate, explicit decision."
                className="mb-6"
              />
              <p className="text-[12.5px] leading-relaxed text-muted-foreground">
                Rust 1.98+ is needed only when building from source. The installer writes{' '}
                <span className="mono">/srv/swapdock</span>, the nginx snippets, logrotate, and a
                sudoers entry for the binary.
              </p>
              <p className="mt-4 text-[12.5px] leading-relaxed text-muted-foreground">
                There are no release binaries and no crates.io package yet. The pre-launch gates are in{' '}
                <a href={LAUNCH} className="text-accent underline underline-offset-2" target="_blank" rel="noreferrer">
                  LAUNCH.md
                </a>
                .
              </p>
            </div>

            <div className="space-y-3">
              <CodeBlock
                title="build from source (Rust 1.98+)"
                lines={[
                  `git clone ${REPO}.git`,
                  'cd swapdock',
                  'bash scripts/package-dist.sh',
                  'sudo bash swapdock-dist/install-swapdock.sh swapdock-dist',
                ]}
              />
              <CodeBlock
                title="one app"
                lines={[
                  'swapdock init blog',
                  '# edit blog.toml',
                  'swapdock --registry . validate',
                  'swapdock --registry . doctor',
                  'sudo swapdock --registry . up blog --release 9c1f2ab',
                ]}
              />
            </div>
          </div>
        </Container>
      </Section>

      <Section>
        <Container>
          <SectionHead
            eyebrow="More"
            title="The docs behind the path."
            lead="The long-form documentation lives in the repository, next to the code it describes."
          />
          <div className="grid gap-x-10 gap-y-3 sm:grid-cols-2">
            {[
              ['Import an existing app', `${REPO}/blob/main/docs/import.md`],
              ['Backup and restore', `${REPO}/blob/main/docs/backup-restore.md`],
              ['Run without root', `${REPO}/blob/main/docs/non-root.md`],
              ['TLS and the tunnel', `${REPO}/blob/main/docs/tls.md`],
              ['Notifications', `${REPO}/blob/main/docs/notifications.md`],
              ['Troubleshooting', `${REPO}/blob/main/docs/troubleshooting.md`],
              ['Upgrade', `${REPO}/blob/main/docs/upgrade.md`],
              ['Security', `${REPO}/blob/main/SECURITY.md`],
            ].map(([label, href]) => (
              <a
                key={label}
                href={href}
                target="_blank"
                rel="noreferrer"
                className="flex items-center justify-between border-b border-border py-3 text-[13px] text-muted-foreground transition-colors hover:text-foreground"
              >
                {label}
                <ArrowRight className="size-3.5" />
              </a>
            ))}
          </div>
        </Container>
      </Section>

      <FinalCta title="Read it once. Run it once." lead="The first app is the best documentation." />
    </>
  )
}
