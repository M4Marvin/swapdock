import { Link } from '@tanstack/react-router'
import { Logo } from '@/components/logo'
import { FOOTER, LAUNCH, REPO } from '@/lib/site'

export function SiteFooter() {
  return (
    <footer className="border-t border-border bg-surface/40">
      <div className="container-page py-14">
        <div className="grid gap-10 md:grid-cols-[1.4fr_repeat(3,1fr)]">
          <div>
            <Logo />
            <p className="mt-4 max-w-xs text-[13.5px] leading-relaxed text-muted-foreground">
              Registry-driven blue/green deploys for a multi-app Docker host, behind one nginx front
              door.
            </p>
            <div className="mt-5 inline-flex items-center gap-2 rounded-full border border-border px-3 py-1.5 text-[11px] font-medium text-muted-foreground">
              <span className="live-dot" />
              Pre-launch v0.2 · MIT
            </div>
          </div>

          {FOOTER.map((group) => (
            <div key={group.title}>
              <h3 className="label-eyebrow mb-4">{group.title}</h3>
              <ul className="space-y-2.5">
                {group.links.map((link) => (
                  <li key={link.label}>
                    {'to' in link ? (
                      <Link
                        to={link.to}
                        className="text-[13.5px] text-muted-foreground transition-colors hover:text-foreground"
                      >
                        {link.label}
                      </Link>
                    ) : (
                      <a
                        href={link.href}
                        target="_blank"
                        rel="noreferrer"
                        className="text-[13.5px] text-muted-foreground transition-colors hover:text-foreground"
                      >
                        {link.label}
                      </a>
                    )}
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </div>

        <div className="mt-12 flex flex-col gap-3 border-t border-border pt-6 text-[12px] text-subtle-foreground sm:flex-row sm:items-center sm:justify-between">
          <p>© {new Date().getFullYear()} swapdock. Open source under the MIT license.</p>
          <p className="flex flex-wrap items-center gap-x-4 gap-y-1">
            <a href={LAUNCH} target="_blank" rel="noreferrer" className="hover:text-foreground">
              Launch gates
            </a>
            <a href={REPO} target="_blank" rel="noreferrer" className="hover:text-foreground">
              GitHub
            </a>
            <span>Built with TanStack Start</span>
          </p>
        </div>
      </div>
    </footer>
  )
}
