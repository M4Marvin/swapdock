import { Check } from 'lucide-react'
import { COMPARE } from '@/content/compare'

export function CompareTable() {
  return (
    <div className="overflow-x-auto rounded-xl border border-border bg-card">
      <table className="w-full min-w-[640px] border-collapse text-left">
        <thead>
          <tr className="border-b border-border">
            <th className="label-eyebrow px-5 py-4">Task</th>
            <th className="label-eyebrow px-5 py-4">The hand-rolled way</th>
            <th className="label-eyebrow px-5 py-4 text-foreground">With swapdock</th>
          </tr>
        </thead>
        <tbody>
          {COMPARE.map(([task, manual, swapdock]) => (
            <tr key={task} className="border-b border-border last:border-0">
              <th className="px-5 py-3.5 text-[13px] font-medium">{task}</th>
              <td className="px-5 py-3.5 text-[13px] text-muted-foreground">{manual}</td>
              <td className="px-5 py-3.5 text-[13px]">
                <span className="flex items-start gap-2">
                  <Check className="mt-0.5 size-3.5 shrink-0 text-live" strokeWidth={3} />
                  {swapdock}
                </span>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
