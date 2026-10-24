import { ArrowRight, Check, Lock, ShieldAlert } from 'lucide-react'
import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'

function Frame({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div className={cn('rounded-xl border border-border bg-surface/50 p-4 sm:p-6', className)}>
      {children}
    </div>
  )
}

export function TopologyDiagram() {
  return (
    <Frame>
      <svg viewBox="0 0 640 330" className="h-auto w-full" role="img" aria-label="nginx on the front port routes to the live release on port 9001 while the previous release stays on 9000">
        <defs>
          <marker id="arrow-live" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
            <path d="M 0 1 L 9 5 L 0 9 z" fill="var(--live)" />
          </marker>
          <marker id="arrow-muted" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
            <path d="M 0 1 L 9 5 L 0 9 z" fill="var(--subtle-foreground)" />
          </marker>
        </defs>

        <text x="24" y="24" fill="var(--subtle-foreground)" fontSize="10" letterSpacing="1.6" fontFamily="var(--font-mono)">FRONT DOOR</text>
        <rect x="24" y="118" width="168" height="76" rx="10" fill="var(--card)" stroke="var(--border-strong)" />
        <text x="108" y="146" textAnchor="middle" fill="var(--foreground)" fontSize="12" fontFamily="var(--font-mono)">nginx :8001</text>
        <text x="108" y="164" textAnchor="middle" fill="var(--subtle-foreground)" fontSize="10">loopback only</text>
        <text x="108" y="180" textAnchor="middle" fill="var(--subtle-foreground)" fontSize="9" fontFamily="var(--font-mono)">blog.example.com</text>

        <rect x="414" y="42" width="202" height="76" rx="10" fill="var(--card)" stroke="var(--live)" strokeOpacity=".55" />
        <text x="515" y="70" textAnchor="middle" fill="var(--foreground)" fontSize="12" fontFamily="var(--font-mono)">127.0.0.1:9001</text>
        <text x="515" y="88" textAnchor="middle" fill="var(--live)" fontSize="10">live · 9c1f2ab</text>
        <circle cx="515" cy="102" r="3" fill="var(--live)" />

        <rect x="414" y="204" width="202" height="76" rx="10" fill="var(--card)" stroke="var(--border)" />
        <text x="515" y="232" textAnchor="middle" fill="var(--subtle-foreground)" fontSize="12" fontFamily="var(--font-mono)">127.0.0.1:9000</text>
        <text x="515" y="250" textAnchor="middle" fill="var(--subtle-foreground)" fontSize="10">previous · kept for rollback</text>

        <path d="M192 150 C 300 150, 300 80, 410 80" fill="none" stroke="var(--live)" strokeWidth="1.6" markerEnd="url(#arrow-live)" />
        <path d="M192 164 C 300 164, 300 242, 410 242" fill="none" stroke="var(--subtle-foreground)" strokeWidth="1.2" strokeDasharray="4 5" markerEnd="url(#arrow-muted)" />
        <text x="300" y="106" fill="var(--live)" fontSize="9.5" fontFamily="var(--font-mono)" textAnchor="middle">traffic</text>
        <text x="300" y="278" fill="var(--subtle-foreground)" fontSize="9.5" fontFamily="var(--font-mono)" textAnchor="middle">rollback path</text>
      </svg>
    </Frame>
  )
}

export function StrategiesDiagram() {
  return (
    <Frame>
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="rounded-lg border border-border bg-card p-4">
          <div className="flex items-center justify-between">
            <span className="mono text-[10px] uppercase tracking-[0.12em] text-accent">swap</span>
            <span className="text-[10px] text-subtle-foreground">stateless</span>
          </div>
          <div className="mt-4 space-y-2">
            <div className="flex items-center justify-between rounded-md border border-live/30 bg-live-soft px-2.5 py-2">
              <span className="mono text-[10.5px]">:9001</span>
              <span className="text-[9.5px] text-live">new · live</span>
            </div>
            <div className="flex items-center justify-between rounded-md border border-border px-2.5 py-2">
              <span className="mono text-[10.5px] text-muted-foreground">:9000</span>
              <span className="text-[9.5px] text-subtle-foreground">old · draining</span>
            </div>
          </div>
          <p className="mt-3 text-[11px] leading-relaxed text-muted-foreground">
            Two containers, two ports, one health gate. No gap.
          </p>
        </div>

        <div className="rounded-lg border border-border bg-card p-4">
          <div className="flex items-center justify-between">
            <span className="mono text-[10px] uppercase tracking-[0.12em] text-warning">replace</span>
            <span className="text-[10px] text-subtle-foreground">stateful</span>
          </div>
          <div className="mt-4 space-y-2">
            <div className="flex items-center justify-between rounded-md border border-border px-2.5 py-2">
              <span className="mono text-[10.5px] text-muted-foreground line-through">:9002 old</span>
              <span className="text-[9.5px] text-muted-foreground">stopped</span>
            </div>
            <div className="flex items-center justify-between rounded-md border border-live/30 bg-live-soft px-2.5 py-2">
              <span className="mono text-[10.5px]">:9002 new</span>
              <span className="text-[9.5px] text-live">live</span>
            </div>
          </div>
          <p className="mt-3 text-[11px] leading-relaxed text-muted-foreground">
            One writer, short gap. Enforced when <span className="mono">writes_state</span> is true.
          </p>
        </div>
      </div>
      <div className="mt-3 flex items-start gap-2 rounded-lg border border-warning/25 bg-warning/5 px-3 py-2.5 text-[11px] text-muted-foreground">
        <ShieldAlert className="mt-0.5 size-3.5 shrink-0 text-warning" />
        The validator refuses swap for a stateful app. There is no override flag.
      </div>
    </Frame>
  )
}

export function RecoveryDiagram() {
  const phases = ['pending', 'flipped', 'stopping', 'committed']
  return (
    <Frame>
      <div className="flex flex-wrap items-center gap-2">
        {phases.map((phase, i) => (
          <div key={phase} className="flex items-center gap-2">
            <span
              className={cn(
                'mono rounded-md border px-2.5 py-1.5 text-[10.5px]',
                phase === 'committed'
                  ? 'border-live/35 bg-live-soft text-live'
                  : 'border-border bg-card text-muted-foreground',
              )}
            >
              {phase}
            </span>
            {i < phases.length - 1 ? <ArrowRight className="size-3 text-subtle-foreground" /> : <Check className="size-3 text-live" />}
          </div>
        ))}
      </div>
      <div className="mt-4 grid gap-2 sm:grid-cols-2">
        <div className="rounded-lg border border-border bg-card px-3 py-2.5">
          <p className="text-[11px] font-medium">Evidence required</p>
          <p className="mt-1 text-[10.5px] leading-relaxed text-muted-foreground">
            exact proxy_pass line, container image id, health, front door
          </p>
        </div>
        <div className="rounded-lg border border-warning/25 bg-warning/5 px-3 py-2.5">
          <p className="text-[11px] font-medium text-warning">Any mismatch</p>
          <p className="mt-1 text-[10.5px] leading-relaxed text-muted-foreground">
            named refusal, no registry write, record left in place
          </p>
        </div>
      </div>
    </Frame>
  )
}

export function OperationsDiagram() {
  const items = [
    ['verify', 'access-log verdict'],
    ['monitor', 'ports · certs · disk'],
    ['prune', 'keep rollback images'],
    ['doctor', 'host preflight'],
  ] as const
  return (
    <Frame>
      <div className="grid gap-2 sm:grid-cols-2">
        {items.map(([name, note]) => (
          <div key={name} className="flex items-center gap-3 rounded-lg border border-border bg-card px-3 py-2.5">
            <span className="size-1.5 rounded-full bg-accent" />
            <span className="mono text-[11px]">{name}</span>
            <span className="ml-auto text-[10px] text-subtle-foreground">{note}</span>
          </div>
        ))}
      </div>
    </Frame>
  )
}

export function PrivilegeDiagram() {
  return (
    <Frame>
      <div className="grid gap-3 sm:grid-cols-[1fr_auto_1fr] sm:items-center">
        <div className="rounded-lg border border-border bg-card p-4">
          <span className="label-eyebrow">non-root</span>
          <p className="mt-2 text-[12px] font-medium">API · console</p>
          <p className="mt-1 text-[10.5px] leading-relaxed text-muted-foreground">
            reads the registry, starts jobs, owns no nginx file
          </p>
        </div>
        <div className="hidden justify-center sm:flex">
          <ArrowRight className="size-4 text-subtle-foreground" />
        </div>
        <div className="rounded-lg border border-border bg-card p-4">
          <span className="label-eyebrow flex items-center gap-1.5">
            <Lock className="size-3" /> root broker
          </span>
          <p className="mt-2 text-[12px] font-medium">nginx apply · docker allowlist</p>
          <p className="mt-1 text-[10.5px] leading-relaxed text-muted-foreground">
            Unix socket, token, narrow operations
          </p>
        </div>
      </div>
      <div className="mt-3 flex items-center gap-2 text-[10.5px] text-subtle-foreground">
        <span className="mono rounded border border-border px-2 py-1">swapdock CLI</span>
        <ArrowRight className="size-3" />
        <span>sudoers entry for the binary, or the same broker</span>
      </div>
    </Frame>
  )
}

export function FeatureDiagram({ kind }: { kind: 'topology' | 'strategies' | 'recovery' | 'operations' }) {
  switch (kind) {
    case 'topology':
      return <TopologyDiagram />
    case 'strategies':
      return <StrategiesDiagram />
    case 'recovery':
      return <RecoveryDiagram />
    case 'operations':
      return <OperationsDiagram />
  }
}
