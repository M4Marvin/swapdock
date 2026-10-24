import { Check } from 'lucide-react'

const numbers = [
  ['530+', 'Rust tests, unit and end-to-end'],
  ['3', 'lock domains: estate, app, nginx'],
  ['20', 'named steps in a swap run'],
  ['0', 'telemetry, analytics, phone-home'],
]

const bullets = [
  'One function may spawn a process, and a test fails if that changes.',
  'Registry writes are staged, fsynced, and renamed, with ownership preserved.',
  'Every command has a timeout; a hung deploy cannot wedge the host.',
  'Dry-run records the full plan and touches nothing.',
]

export function Proof() {
  return (
    <div className="rounded-2xl border border-border bg-card p-8 md:p-10">
      <div className="grid gap-8 sm:grid-cols-2 lg:grid-cols-4">
        {numbers.map(([value, label]) => (
          <div key={label}>
            <div className="font-mono text-4xl font-medium tracking-[-0.04em] text-foreground md:text-5xl">
              {value}
            </div>
            <p className="mt-2 text-[12px] leading-snug text-muted-foreground">{label}</p>
          </div>
        ))}
      </div>
      <div className="mt-10 grid gap-x-10 gap-y-3 border-t border-border pt-7 sm:grid-cols-2">
        {bullets.map((b) => (
          <p key={b} className="flex items-start gap-2.5 text-[12.5px] leading-relaxed text-muted-foreground">
            <Check className="mt-0.5 size-3.5 shrink-0 text-live" strokeWidth={3} />
            {b}
          </p>
        ))}
      </div>
    </div>
  )
}
