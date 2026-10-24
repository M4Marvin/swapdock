import { Moon, Sun } from 'lucide-react'
import { Button } from '@/components/ui/button'

function toggle() {
  const root = document.documentElement
  const nextLight = root.classList.contains('dark')
  root.classList.toggle('dark', !nextLight)
  try {
    localStorage.setItem('swapdock-site-theme', nextLight ? 'light' : 'dark')
  } catch {
    /* private mode */
  }
  const meta = document.querySelector('meta[name="theme-color"]')
  if (meta) meta.setAttribute('content', nextLight ? '#f7f7f4' : '#08090a')
}

export function ThemeToggle() {
  return (
    <Button
      variant="ghost"
      size="icon"
      onClick={toggle}
      aria-label="Toggle color theme"
      className="text-muted-foreground hover:text-foreground"
    >
      <Moon className="size-4 dark:hidden" />
      <Sun className="hidden size-4 dark:block" />
    </Button>
  )
}
