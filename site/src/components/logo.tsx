import { cn } from '@/lib/utils'

export function LogoMark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 32 32" aria-hidden="true" className={cn('size-7', className)}>
      <rect x="2" y="8" width="12" height="16" rx="4" fill="var(--accent)" />
      <rect x="18" y="8" width="12" height="16" rx="4" fill="var(--live)" />
      <path d="M9 16h14" stroke="var(--foreground)" strokeWidth="1.5" strokeOpacity=".35" />
    </svg>
  )
}

export function Logo({ className }: { className?: string }) {
  return (
    <span className={cn('inline-flex items-center gap-2.5', className)}>
      <LogoMark />
      <span className="text-[15px] font-semibold tracking-[-0.02em]">swapdock</span>
    </span>
  )
}
