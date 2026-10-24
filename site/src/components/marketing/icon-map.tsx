import {
  ArrowLeftRight,
  Database,
  FileCode2,
  Gauge,
  Lock,
  Plug,
  ScrollText,
  ShieldCheck,
  Undo2,
} from 'lucide-react'
import type { LucideIcon } from 'lucide-react'

export const ICONS: Record<string, LucideIcon> = {
  swap: ArrowLeftRight,
  database: Database,
  undo: Undo2,
  file: FileCode2,
  shield: ShieldCheck,
  log: ScrollText,
  lock: Lock,
  plug: Plug,
  gauge: Gauge,
}
