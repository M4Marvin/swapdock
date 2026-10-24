import { useEffect, useRef, type ReactNode } from 'react'
import { cn } from '@/lib/utils'

export function Reveal({
  children,
  className,
  delay = 0,
  as: Tag = 'div',
}: {
  children: ReactNode
  className?: string
  delay?: number
  as?: 'div' | 'section' | 'li' | 'span'
}) {
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const el = ref.current
    if (!el) return
    if (typeof IntersectionObserver === 'undefined') {
      el.classList.add('is-in')
      return
    }
    const io = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            el.classList.add('is-in')
            io.disconnect()
          }
        }
      },
      { rootMargin: '0px 0px -12% 0px', threshold: 0.05 },
    )
    io.observe(el)
    return () => io.disconnect()
  }, [])

  return (
    <Tag
      ref={ref as never}
      data-reveal=""
      style={{ transitionDelay: `${delay}ms` }}
      className={cn(className)}
    >
      {children}
    </Tag>
  )
}
