import { useId } from "react"
import { cn } from "cn"

/**
 * swapdock's mark: two arrows trading places over a dock.
 *
 * The gradient is the product's only decorative colour: blue-green for the
 * two slots a deploy swaps between. Everything else in the UI stays neutral
 * unless it carries status.
 */
export function BrandMark({ className }: { className?: string }) {
  const id = useId()
  return (
    <svg
      viewBox="0 0 32 32"
      aria-hidden="true"
      className={cn("size-7", className)}
      fill="none"
    >
      <defs>
        <linearGradient id={id} x1="8" y1="8" x2="24" y2="24" gradientUnits="userSpaceOnUse">
          <stop stopColor="#7B87FF" />
          <stop offset="1" stopColor="#46D894" />
        </linearGradient>
      </defs>
      <path
        d="M8.5 12.5h14m0 0-3.75-3.75M22.5 12.5l-3.75 3.75"
        stroke={`url(#${id})`}
        strokeWidth="2.1"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M23.5 19.5h-14m0 0 3.75-3.75M9.5 19.5l3.75 3.75"
        stroke={`url(#${id})`}
        strokeWidth="2.1"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  )
}

export function Wordmark({ className }: { className?: string }) {
  return (
    <span className={cn("inline-flex items-center gap-2", className)}>
      <BrandMark className="size-[22px]" />
      <span className="text-[15px] font-semibold tracking-[-0.03em] text-foreground">
        swapdock
      </span>
    </span>
  )
}
