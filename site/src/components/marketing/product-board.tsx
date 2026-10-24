import { Check } from 'lucide-react'

const steps = [
  ['pull image', '0.8s'],
  ['start green', '1.4s'],
  ['health gate', '2.1s'],
  ['flip nginx', '0.2s'],
  ['probe front', '0.4s'],
  ['commit', '0.1s'],
] as const

export function ProductBoard() {
  return (
    <div className="relative mx-auto w-full max-w-[540px]">
      <div className="glow -top-16 right-0 h-56 w-72 bg-accent/20" />
      <div className="glow -bottom-10 left-4 h-40 w-56 bg-live/10" />

      <div className="relative overflow-hidden rounded-2xl border border-border bg-card" style={{ boxShadow: 'var(--shadow-float)' }}>
        <div className="flex items-center gap-2 border-b border-border px-4 py-3">
          <span className="flex gap-1.5">
            <i className="size-2 rounded-full bg-foreground/15" />
            <i className="size-2 rounded-full bg-foreground/15" />
            <i className="size-2 rounded-full bg-foreground/15" />
          </span>
          <span className="mono ml-2 text-[10.5px] text-subtle-foreground">swapdock / blog</span>
          <span className="ml-auto inline-flex items-center gap-1.5 rounded-full border border-live/25 bg-live-soft px-2 py-0.5 text-[10px] font-medium text-live">
            <span className="live-dot" />
            live
          </span>
        </div>

        <div className="p-4 sm:p-5">
          <div className="flex items-start justify-between">
            <div>
              <p className="text-[12.5px] font-medium">Deploy 128</p>
              <p className="mono mt-1 text-[10px] text-subtle-foreground">run 4f2c9a · 9c1f2ab</p>
            </div>
            <span className="mono text-[10px] text-live">12.3 s</span>
          </div>

          <ol className="mt-4 space-y-1">
            {steps.map(([label, time]) => (
              <li key={label} className="flex items-center gap-2 text-[11px] text-muted-foreground">
                <span className="grid size-3.5 place-items-center rounded-full bg-live-soft text-live">
                  <Check className="size-2.5" strokeWidth={3} />
                </span>
                <span className="mono">{label}</span>
                <span className="ml-auto mono text-[10px] text-subtle-foreground">{time}</span>
              </li>
            ))}
          </ol>

          <div className="mt-4 rounded-xl border border-border bg-surface/60 p-3">
            <div className="flex items-center justify-between">
              <span className="label-eyebrow">traffic</span>
              <span className="mono text-[9.5px] text-subtle-foreground">3 hosts</span>
            </div>
            <div className="mono mt-2 flex items-center gap-2 text-[10.5px]">
              <span className="text-subtle-foreground">nginx :8001</span>
              <span className="text-subtle-foreground">→</span>
              <span className="text-foreground">127.0.0.1:9001</span>
              <span className="ml-auto rounded bg-live-soft px-1.5 py-0.5 text-[9px] text-live">healthy</span>
            </div>
            <div className="mono mt-1.5 flex items-center gap-2 text-[10.5px] text-subtle-foreground">
              <span className="w-[74px]" />
              <span className="text-subtle-foreground">→</span>
              <span>127.0.0.1:9000</span>
              <span className="ml-auto rounded bg-surface px-1.5 py-0.5 text-[9px] text-subtle-foreground">
                rollback ready
              </span>
            </div>
          </div>
        </div>

        <div className="border-t border-white/10 bg-[#0a0b0d] px-4 py-3 sm:px-5">
          <p className="mono truncate text-[10.5px] text-muted-foreground">
            <span className="text-white/35">$ </span><span className="text-white/70">swapdock up blog</span>{' '}
            <span className="text-[#a8b0ff]">--release</span><span className="text-white/70"> 9c1f2ab</span>
          </p>
        </div>
      </div>
      <p className="mt-3 text-right text-[10.5px] text-subtle-foreground">
        Illustrative state. The real one is one <span className="mono">status</span> away.
      </p>
    </div>
  )
}
