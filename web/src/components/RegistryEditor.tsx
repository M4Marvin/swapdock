import { useEffect, useState, type ReactNode } from 'react'
import { api, type App } from '../api'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'

/**
 * A draft of every editable App field, with numbers and lists kept as strings
 * so a half-typed value never becomes `NaN` and the caret does not jump.
 */
interface Draft {
  strategy: string
  hostnames: string
  listen: string
  front_port: string
  slot: string
  live_port: string
  old_port: string
  writes_state: boolean
  image_repo: string
  registry: string
  build_host: string
  release: string
  old_release: string
  health_url: string
  compose_dir: string
  compose_svc: string
  env_name: string
  git_remote: string
  branch: string
  repo: string
  root: string
  build_repo: string
}

type DraftErrors = Partial<Record<keyof Draft, string>>

function toDraft(app: App): Draft {
  return {
    strategy: app.strategy ?? '',
    hostnames: app.hostnames.join(', '),
    listen: app.listen.join(', '),
    front_port: app.front_port != null ? String(app.front_port) : '',
    slot: app.slot != null ? String(app.slot) : '',
    live_port: app.live_port != null ? String(app.live_port) : '',
    old_port: app.old_port != null ? String(app.old_port) : '',
    writes_state: app.writes_state,
    image_repo: app.image_repo ?? '',
    registry: app.registry ?? '',
    build_host: app.build_host ?? '',
    release: app.release ?? '',
    old_release: app.old_release ?? '',
    health_url: app.health_url ?? '',
    compose_dir: app.compose_dir ?? '',
    compose_svc: app.compose_svc ?? '',
    env_name: app.env_name ?? '',
    git_remote: app.git_remote ?? '',
    branch: app.branch ?? '',
    repo: app.repo ?? '',
    root: app.root ?? '',
    build_repo: app.build_repo ?? '',
  }
}

/** Validates the draft client-side; empty string means "no error". */
function validateDraft(d: Draft): DraftErrors {
  const errors: DraftErrors = {}

  if (d.strategy !== 'swap' && d.strategy !== 'replace') {
    errors.strategy = 'must be "swap" or "replace"'
  }

  const port = (raw: string, key: keyof Draft, required: boolean) => {
    const v = raw.trim()
    if (v === '') {
      if (required) errors[key] = 'required'
      return
    }
    const n = Number(v)
    if (!Number.isInteger(n) || n < 1 || n > 65535) {
      errors[key] = 'must be a port between 1 and 65535'
    }
  }
  port(d.front_port, 'front_port', true)
  port(d.live_port, 'live_port', false)
  port(d.old_port, 'old_port', false)

  const slot = d.slot.trim()
  if (slot === '') {
    errors.slot = 'required'
  } else {
    const n = Number(slot)
    if (!Number.isInteger(n) || n < 0 || n > 127) {
      errors.slot = 'must be a slot between 0 and 127'
    }
  }

  // A front port doubles as a back-end port only if it collides with this
  // app's own live/old port; cross-app collisions need the whole registry.
  const front = Number(d.front_port)
  const live = Number(d.live_port)
  const old = Number(d.old_port)
  if (Number.isInteger(front)) {
    if (d.live_port.trim() !== '' && front === live) {
      errors.front_port = `front port also used as live port (${front})`
    } else if (d.old_port.trim() !== '' && front === old) {
      errors.front_port = `front port also used as old port (${front})`
    }
  }

  const health = d.health_url.trim()
  if (health !== '' && !/^https?:\/\//.test(health)) {
    errors.health_url = 'must start with http:// or https://'
  }

  const env = d.env_name.trim()
  if (env === '') {
    errors.env_name = 'required'
  } else if (!/^[A-Z][A-Z0-9_]*$/.test(env)) {
    errors.env_name = 'must be uppercase, e.g. CHAT_PORT'
  }

  if (d.compose_dir.trim() === '') errors.compose_dir = 'required'
  if (d.compose_svc.trim() === '') errors.compose_svc = 'required'

  const registry = d.registry.trim()
  if (
    registry !== '' &&
    !['ghcr', 'docker-hub', 'local'].includes(registry)
  ) {
    errors.registry = 'one of ghcr, docker-hub, local'
  }

  return errors
}

function splitList(raw: string): string[] {
  return raw
    .split(',')
    .map((s) => s.trim())
    .filter((s) => s !== '')
}

/** Builds the PUT body from the app identity plus the validated draft. */
function fromDraft(app: App, d: Draft): App {
  const num = (raw: string): number | null => {
    const v = raw.trim()
    if (v === '') return null
    const n = Number(v)
    return Number.isInteger(n) ? n : null
  }
  const str = (raw: string): string | null => {
    const v = raw.trim()
    return v === '' ? null : v
  }
  return {
    ...app,
    strategy: d.strategy.trim(),
    hostnames: splitList(d.hostnames),
    listen: splitList(d.listen),
    front_port: num(d.front_port) ?? app.front_port,
    slot: num(d.slot) ?? app.slot,
    live_port: num(d.live_port),
    old_port: num(d.old_port),
    writes_state: d.writes_state,
    image_repo: str(d.image_repo),
    registry: str(d.registry),
    build_host: str(d.build_host),
    release: str(d.release),
    old_release: str(d.old_release),
    health_url: str(d.health_url),
    compose_dir: str(d.compose_dir),
    compose_svc: str(d.compose_svc),
    env_name: str(d.env_name),
    git_remote: str(d.git_remote),
    branch: str(d.branch),
    repo: str(d.repo),
    root: str(d.root),
    build_repo: str(d.build_repo),
  }
}

/** Renders a string as a TOML basic string. */
function q(s: string): string {
  return JSON.stringify(s)
}

/** Renders a draft as TOML, omitting empty optional fields. */
function toToml(app: App, d: Draft): string {
  const out: string[] = []
  const str = (key: string, value: string) => {
    if (value.trim() !== '') out.push(`${key} = ${q(value.trim())}`)
  }
  const num = (key: string, value: string) => {
    const v = value.trim()
    if (v === '') return
    out.push(`${key} = ${/^\d+$/.test(v) ? v : q(v)}`)
  }
  const arr = (key: string, value: string) => {
    const items = splitList(value)
    out.push(`${key} = [${items.map(q).join(', ')}]`)
  }

  str('name', app.name)
  str('kind', app.kind)
  str('strategy', d.strategy)
  arr('hostnames', d.hostnames)
  arr('listen', d.listen)
  num('front_port', d.front_port)
  num('slot', d.slot)
  num('live_port', d.live_port)
  num('old_port', d.old_port)
  out.push(`writes_state = ${d.writes_state}`)
  str('image_repo', d.image_repo)
  str('registry', d.registry)
  str('release', d.release)
  str('old_release', d.old_release)
  str('build_host', d.build_host)
  str('root', d.root)
  str('health_url', d.health_url)
  str('compose_dir', d.compose_dir)
  str('compose_svc', d.compose_svc)
  str('env_name', d.env_name)
  str('git_remote', d.git_remote)
  str('branch', d.branch)
  str('repo', d.repo)
  str('build_repo', d.build_repo)

  return out.join('\n') + '\n'
}

function Field({
  label,
  error,
  children,
}: {
  label: string
  error?: string
  children: ReactNode
}) {
  return (
    <div className="flex flex-col gap-1">
      <label className="text-xs text-muted-foreground">{label}</label>
      {children}
      {error && <p className="text-xs text-destructive">{error}</p>}
    </div>
  )
}

/**
 * Read/edit view of every registry field.
 *
 * Validates the draft in the browser and saves through
 * `PUT /api/apps/{name}`, which refuses estates with errors and keeps a
 * `.bak` of the previous file. **Revert to saved** re-reads the registry.
 */
export function RegistryEditor({ app }: { app: App }) {
  const [draft, setDraft] = useState<Draft>(() => toDraft(app))
  const [saved, setSaved] = useState<Draft>(() => toDraft(app))
  const [reverting, setReverting] = useState(false)
  const [revertError, setRevertError] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const [saveError, setSaveError] = useState<string | null>(null)
  const [saveOk, setSaveOk] = useState(false)
  const [copied, setCopied] = useState(false)

  useEffect(() => {
    const d = toDraft(app)
    setDraft(d)
    setSaved(d)
  }, [app])

  const errors = validateDraft(draft)
  const dirty = JSON.stringify(draft) !== JSON.stringify(saved)
  const errorCount = Object.keys(errors).length

  const set = <K extends keyof Draft>(key: K, value: Draft[K]) =>
    setDraft((cur) => ({ ...cur, [key]: value }))

  const revert = async () => {
    setReverting(true)
    setRevertError(null)
    try {
      const entry = await api.app(app.name)
      const d = toDraft(entry.app)
      setDraft(d)
      setSaved(d)
    } catch (e) {
      setRevertError(String(e))
    } finally {
      setReverting(false)
    }
  }

  const tomlText = toToml(app, draft)

  const save = async () => {
    if (errorCount > 0) return
    setSaving(true)
    setSaveError(null)
    setSaveOk(false)
    try {
      await api.updateApp(app.name, fromDraft(app, draft))
      setSaved({ ...draft })
      setSaveOk(true)
    } catch (e) {
      setSaveError(String(e))
    } finally {
      setSaving(false)
    }
  }

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(tomlText)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1500)
    } catch {
      // Clipboard access can be denied; the <details> block still shows it.
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Registry editor</CardTitle>
        <p className="text-sm text-muted-foreground">
          Field errors block saving; the server re-validates the whole estate
          and refuses on any error, keeping a <span className="font-mono">.bak</span> of
          the previous file.
        </p>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant={errorCount > 0 ? 'destructive' : 'outline'}>
            {errorCount} field error{errorCount === 1 ? '' : 's'}
          </Badge>
          {dirty && <Badge variant="secondary">unsaved changes</Badge>}
          {saveOk && !dirty && <Badge>saved</Badge>}
          <span className="flex-1" />
          <Button
            variant="outline"
            size="sm"
            disabled={reverting}
            onClick={revert}
          >
            {reverting ? 'Reverting…' : 'Revert to saved'}
          </Button>
          <Button
            size="sm"
            disabled={saving || !dirty || errorCount > 0}
            onClick={save}
          >
            {saving ? 'Saving…' : 'Save'}
          </Button>
          <Button variant="outline" size="sm" onClick={copy}>
            {copied ? 'Copied' : 'Copy as TOML'}
          </Button>
        </div>

        {saveError && (
          <p className="text-sm text-destructive">{saveError}</p>
        )}

        {revertError && (
          <p className="text-sm text-destructive">{revertError}</p>
        )}

        <div className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-3 text-sm">
          <span className="pt-1.5 text-muted-foreground">name</span>
          <span className="pt-1.5 font-mono">{app.name}</span>
          <span className="pt-1.5 text-muted-foreground">kind</span>
          <span className="pt-1.5 font-mono">{app.kind}</span>
        </div>

        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          <Field label="strategy" error={errors.strategy}>
            <Input
              value={draft.strategy}
              onChange={(e) => set('strategy', e.target.value)}
              aria-invalid={errors.strategy != null}
            />
          </Field>
          <Field label="front_port" error={errors.front_port}>
            <Input
              type="number"
              value={draft.front_port}
              onChange={(e) => set('front_port', e.target.value)}
              aria-invalid={errors.front_port != null}
            />
          </Field>
          <Field label="slot" error={errors.slot}>
            <Input
              type="number"
              value={draft.slot}
              onChange={(e) => set('slot', e.target.value)}
              aria-invalid={errors.slot != null}
            />
          </Field>
          <Field label="live_port" error={errors.live_port}>
            <Input
              type="number"
              value={draft.live_port}
              onChange={(e) => set('live_port', e.target.value)}
              aria-invalid={errors.live_port != null}
            />
          </Field>
          <Field label="old_port" error={errors.old_port}>
            <Input
              type="number"
              value={draft.old_port}
              onChange={(e) => set('old_port', e.target.value)}
              aria-invalid={errors.old_port != null}
            />
          </Field>
          <Field label="registry" error={errors.registry}>
            <Input
              value={draft.registry}
              onChange={(e) => set('registry', e.target.value)}
              placeholder="ghcr / docker-hub / local"
              aria-invalid={errors.registry != null}
            />
          </Field>
          <Field label="hostnames (comma-separated)">
            <Input
              value={draft.hostnames}
              onChange={(e) => set('hostnames', e.target.value)}
              className="font-mono"
            />
          </Field>
          <Field label="listen (comma-separated)">
            <Input
              value={draft.listen}
              onChange={(e) => set('listen', e.target.value)}
              className="font-mono"
            />
          </Field>
          <Field label="image_repo">
            <Input
              value={draft.image_repo}
              onChange={(e) => set('image_repo', e.target.value)}
              className="font-mono"
            />
          </Field>
          <Field label="release">
            <Input
              value={draft.release}
              onChange={(e) => set('release', e.target.value)}
              className="font-mono"
            />
          </Field>
          <Field label="old_release">
            <Input
              value={draft.old_release}
              onChange={(e) => set('old_release', e.target.value)}
              className="font-mono"
            />
          </Field>
          <Field label="build_host">
            <Input
              value={draft.build_host}
              onChange={(e) => set('build_host', e.target.value)}
            />
          </Field>
          <Field label="health_url" error={errors.health_url}>
            <Input
              value={draft.health_url}
              onChange={(e) => set('health_url', e.target.value)}
              className="font-mono"
              aria-invalid={errors.health_url != null}
            />
          </Field>
          <Field label="env_name" error={errors.env_name}>
            <Input
              value={draft.env_name}
              onChange={(e) => set('env_name', e.target.value)}
              className="font-mono"
              aria-invalid={errors.env_name != null}
            />
          </Field>
          <Field label="compose_dir" error={errors.compose_dir}>
            <Input
              value={draft.compose_dir}
              onChange={(e) => set('compose_dir', e.target.value)}
              className="font-mono"
              aria-invalid={errors.compose_dir != null}
            />
          </Field>
          <Field label="compose_svc" error={errors.compose_svc}>
            <Input
              value={draft.compose_svc}
              onChange={(e) => set('compose_svc', e.target.value)}
              className="font-mono"
              aria-invalid={errors.compose_svc != null}
            />
          </Field>
          <Field label="root">
            <Input
              value={draft.root}
              onChange={(e) => set('root', e.target.value)}
              className="font-mono"
            />
          </Field>
          <Field label="git_remote">
            <Input
              value={draft.git_remote}
              onChange={(e) => set('git_remote', e.target.value)}
            />
          </Field>
          <Field label="branch">
            <Input
              value={draft.branch}
              onChange={(e) => set('branch', e.target.value)}
            />
          </Field>
          <Field label="repo">
            <Input
              value={draft.repo}
              onChange={(e) => set('repo', e.target.value)}
              className="font-mono"
            />
          </Field>
          <Field label="build_repo">
            <Input
              value={draft.build_repo}
              onChange={(e) => set('build_repo', e.target.value)}
              className="font-mono"
            />
          </Field>
        </div>

        <div className="flex items-center gap-2">
          <input
            id="writes_state"
            type="checkbox"
            checked={draft.writes_state}
            onChange={(e) => set('writes_state', e.target.checked)}
            className="size-4 accent-primary"
          />
          <label htmlFor="writes_state" className="text-sm">
            writes_state
          </label>
          <span className="text-xs text-muted-foreground">
            app keeps shared local state; forbids the swap strategy
          </span>
        </div>

        <details className="text-xs">
          <summary className="cursor-pointer select-none text-muted-foreground">
            Draft TOML
          </summary>
          <pre className="mt-2 overflow-x-auto rounded-lg border bg-muted/40 p-3 font-mono">
            {tomlText}
          </pre>
        </details>
      </CardContent>
    </Card>
  )
}
