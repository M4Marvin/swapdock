import { Check, Copy } from 'lucide-react'
import { useState } from 'react'
import { cn } from '@/lib/utils'

export function CodeBlock({
  title,
  lines,
  className,
  prompt = true,
}: {
  title?: string
  lines: string[]
  className?: string
  prompt?: boolean
}) {
  const [copied, setCopied] = useState(false)
  const text = lines.join('\n')

  async function copy() {
    try {
      await navigator.clipboard.writeText(text)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1400)
    } catch {
      /* clipboard is optional */
    }
  }

  return (
    <div className={cn('w-full min-w-0 overflow-hidden rounded-xl border border-border bg-[#0a0b0d]', className)}>
      <div className="flex items-center justify-between border-b border-white/8 px-3.5 py-2">
        <span className="mono text-[10px] tracking-wide text-white/55">{title ?? 'shell'}</span>
        <button
          type="button"
          onClick={copy}
          aria-label="Copy commands"
          className="inline-flex items-center gap-1.5 rounded-md px-1.5 py-1 text-[10px] text-white/55 transition hover:bg-white/5 hover:text-white/70"
        >
          {copied ? <Check className="size-3" /> : <Copy className="size-3" />}
          {copied ? 'copied' : 'copy'}
        </button>
      </div>
      <pre className="max-w-full overflow-x-auto px-4 py-3.5 text-[12px] leading-[1.75] text-white/80">
        <code>
          {lines.map((line) => (
            <span key={line} className="block">
              {line.startsWith('#') ? <span className="text-white/55">{line}</span> : (
                <>
                  <span className="select-none text-white/55">$ </span>
                  {line}
                </>
              )}
            </span>
          ))}
        </code>
      </pre>
    </div>
  )
}
