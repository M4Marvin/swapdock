import { useEffect, useState, type ReactNode } from 'react'
import { useForm, useStore, type AnyFieldApi } from '@tanstack/react-form'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { toast } from 'sonner'
import { z } from 'zod'
import { TriangleAlert } from 'lucide-react'
import { api, type App } from '../api'
import { queryKeys } from '@/lib/query-keys'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldError,
  FieldLabel,
} from '@/components/ui/field'

// ---------------------------------------------------------------------------
// schema
// ---------------------------------------------------------------------------

/** A required whole number in a range; the draft keeps it as a string. */
function requiredInt(min: number, max: number, message: string) {
  return z.string().refine((v) => {
    if (v.trim() === '') return false
    const n = Number(v)
    return Number.isInteger(n) && n >= min && n <= max
  }, message)
}

/** An optional whole number in a range; empty string means "unset". */
function optionalInt(min: number, max: number, message: string) {
  return z.string().refine((v) => {
    if (v.trim() === '') return true
    const n = Number(v)
    return Number.isInteger(n) && n >= min && n <= max
  }, message)
}

const PORT_MESSAGE = 'must be a port between 1 and 65535'

/**
 * Client-side mirror of the server's App constraints.
 *
 * Numeric fields stay strings in the draft so a half-typed value never becomes
 * `NaN`; the refinements enforce the same shape the API would.
 */
const draftSchema = z
  .object({
    strategy: z
      .string()
      .refine(
        (v: string): boolean => v === 'swap' || v === 'replace',
        'must be "swap" or "replace"',
      ),
    hostnames: z.string(),
    listen: z.string(),
    front_port: requiredInt(1, 65535, PORT_MESSAGE),
    slot: requiredInt(0, 127, 'must be a slot between 0 and 127'),
    live_port: optionalInt(1, 65535, PORT_MESSAGE),
    old_port: optionalInt(1, 65535, PORT_MESSAGE),
    writes_state: z.boolean(),
    image_repo: z.string(),
    registry: z
      .string()
      .refine(
        (v) => v === '' || ['ghcr', 'docker-hub', 'local'].includes(v),
        'one of ghcr, docker-hub, local',
      ),
    build_host: z.string(),
    release: z.string(),
    old_release: z.string(),
    health_url: z
      .string()
      .refine(
        (v) => v === '' || v.startsWith('http'),
        'must start with http:// or https://',
      ),
    compose_dir: z.string(),
    compose_svc: z.string(),
    env_name: z
      .string()
      .refine(
        (v) => v === '' || /^[A-Z][A-Z0-9_]*$/.test(v),
        'must be uppercase, e.g. CHAT_PORT',
      ),
    git_remote: z.string(),
    branch: z.string(),
    repo: z.string(),
    root: z.string(),
    build_repo: z.string(),
  })
  // A front port doubles as a back-end port only if it collides with this
  // app's own live/old port; cross-app collisions need the whole registry.
  .superRefine((d, ctx) => {
    const front = Number(d.front_port)
    if (!Number.isInteger(front)) return
    if (d.live_port.trim() !== '' && front === Number(d.live_port)) {
      ctx.addIssue({
        code: 'custom',
        path: ['front_port'],
        message: `front port also used as live port (${front})`,
      })
    } else if (d.old_port.trim() !== '' && front === Number(d.old_port)) {
      ctx.addIssue({
        code: 'custom',
        path: ['front_port'],
        message: `front port also used as old port (${front})`,
      })
    }
  })

type Draft = z.infer<typeof draftSchema>

// ---------------------------------------------------------------------------
// draft <-> App
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// field renderer
// ---------------------------------------------------------------------------

/** One text/number field, wired to a TanStack Form field and shadcn Field. */
function FieldInput({
  field,
  label,
  placeholder,
  description,
  mono,
  type,
}: {
  field: AnyFieldApi
  label: string
  placeholder?: string
  description?: ReactNode
  mono?: boolean
  type?: 'text' | 'number'
}) {
  const isInvalid = field.state.meta.isTouched && !field.state.meta.isValid
  return (
    <Field data-invalid={isInvalid}>
      <FieldLabel htmlFor={field.name}>{label}</FieldLabel>
      <Input
        id={field.name}
        name={field.name}
        type={type}
        value={(field.state.value as string) ?? ''}
        onBlur={field.handleBlur}
        onChange={(e) => field.handleChange(e.target.value)}
        aria-invalid={isInvalid}
        placeholder={placeholder}
        className={mono ? 'font-mono' : undefined}
        autoComplete="off"
      />
      {description && <FieldDescription>{description}</FieldDescription>}
      {isInvalid && <FieldError errors={field.state.meta.errors} />}
    </Field>
  )
}

/**
 * Read/edit view of every registry field.
 *
 * Validates the draft in the browser and saves through
 * `PUT /api/apps/{name}`, which refuses estates with errors and keeps a
 * `.bak` of the previous file. Fields that deploys own (strategy, ports,
 * release chain) live behind the danger-zone dialog, but share the same form.
 */
export function RegistryEditor({ app }: { app: App }) {
  const queryClient = useQueryClient()
  const [dangerOpen, setDangerOpen] = useState(false)
  const [copied, setCopied] = useState(false)

  const saveMutation = useMutation({
    mutationFn: (payload: App) => api.updateApp(app.name, payload),
    onSuccess: (result) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.app(app.name) })
      queryClient.invalidateQueries({ queryKey: queryKeys.apps })
      queryClient.invalidateQueries({ queryKey: queryKeys.validate })
      toast.success('Registry saved', {
        description: `saved, .bak created${result.backup ? ` (${result.backup})` : ''}`,
      })
    },
    onError: (error) => {
      toast.error('Registry save failed', {
        description: error instanceof Error ? error.message : String(error),
      })
    },
  })

  const form = useForm({
    defaultValues: toDraft(app),
    validators: {
      onChange: draftSchema,
      onSubmit: draftSchema,
    },
    onSubmit: async ({ value }) => {
      try {
        await saveMutation.mutateAsync(fromDraft(app, value))
        // Adopt the saved values as the new baseline for Reset / dirty state.
        form.reset(value)
        setDangerOpen(false)
      } catch {
        // Surfaced by saveMutation.error in the dialog banner.
      }
    },
  })

  const values = useStore(form.store, (s) => s.values)
  const isDirty = useStore(form.store, (s) => s.isDirty)
  const errorCount = useStore(
    form.store,
    (s) =>
      Object.values(s.fieldMeta).filter(
        (m) => m && m.errors && m.errors.length > 0,
      ).length,
  )

  // Adopt server-side changes (e.g. a deploy rewrote the release) when there
  // are no unsaved edits; never clobber an in-progress draft.
  useEffect(() => {
    if (!isDirty) form.reset(toDraft(app))
  }, [app, isDirty, form])

  const saving = saveMutation.isPending
  const saveHint = errorCount > 0
    ? 'fix field errors first'
    : !isDirty
      ? 'no changes to save'
      : undefined
  const saveError = saveMutation.error
    ? saveMutation.error instanceof Error
      ? saveMutation.error.message
      : String(saveMutation.error)
    : null

  const tomlText = toToml(app, values)

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(tomlText)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1500)
    } catch {
      toast.error('Copy failed', {
        description: 'Select the Draft TOML below and copy it manually.',
      })
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Registry editor</CardTitle>
        <p className="text-sm text-muted-foreground">
          Field errors block saving; the server re-validates the whole estate
          and refuses on any error, keeping a{' '}
          <span className="font-mono">.bak</span> of the previous file.
        </p>
      </CardHeader>
      <CardContent className="space-y-4">
        <form
          id="registry-form"
          className="space-y-4"
          onSubmit={(e) => {
            e.preventDefault()
            form.handleSubmit()
          }}
        >
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant={errorCount > 0 ? 'destructive' : 'outline'}>
              {errorCount} field error{errorCount === 1 ? '' : 's'}
            </Badge>
            {isDirty && <Badge variant="secondary">unsaved changes</Badge>}
            {saveMutation.isSuccess && !isDirty && <Badge>saved</Badge>}
            <span className="flex-1" />
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => {
                form.reset()
                saveMutation.reset()
              }}
            >
              Reset
            </Button>
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => setDangerOpen(true)}
              className="border-destructive/50 text-destructive hover:bg-destructive/10 hover:text-destructive"
            >
              <TriangleAlert className="size-4" />
              Danger zone
            </Button>
            <Button type="button" variant="outline" size="sm" onClick={copy}>
              {copied ? 'Copied' : 'Copy as TOML'}
            </Button>
            <Button
              type="submit"
              size="sm"
              disabled={saving || !isDirty || errorCount > 0}
              title={(saving || !isDirty || errorCount > 0) ? saveHint : undefined}
            >
              {saving ? 'Saving…' : 'Save'}
            </Button>
          </div>

          {saveError && !dangerOpen && (
            <p role="alert" className="text-sm text-destructive">
              {saveError}
            </p>
          )}

          <div className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-3 text-sm">
            <span className="pt-1.5 text-muted-foreground">name</span>
            <span className="pt-1.5 font-mono">{app.name}</span>
            <span className="pt-1.5 text-muted-foreground">kind</span>
            <span className="pt-1.5 font-mono">{app.kind}</span>
          </div>

          <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
            <form.Field name="registry">
              {(field) => (
                <FieldInput
                  field={field}
                  label="registry"
                  placeholder="ghcr / docker-hub / local"
                />
              )}
            </form.Field>
            <form.Field name="hostnames">
              {(field) => (
                <FieldInput field={field} label="hostnames (comma-separated)" placeholder="a.example, b.example" />
              )}
            </form.Field>
            <form.Field name="listen">
              {(field) => (
                <FieldInput field={field} label="listen (comma-separated)" placeholder="0.0.0.0:8080" />
              )}
            </form.Field>
            <form.Field name="image_repo">
              {(field) => <FieldInput field={field} label="image_repo" mono />}
            </form.Field>
            <form.Field name="build_host">
              {(field) => <FieldInput field={field} label="build_host" />}
            </form.Field>
            <form.Field name="health_url">
              {(field) => (
                <FieldInput field={field} label="health_url" mono placeholder="https://…" />
              )}
            </form.Field>
            <form.Field name="env_name">
              {(field) => (
                <FieldInput field={field} label="env_name" mono placeholder="CHAT_PORT" />
              )}
            </form.Field>
            <form.Field name="compose_dir">
              {(field) => <FieldInput field={field} label="compose_dir" mono />}
            </form.Field>
            <form.Field name="compose_svc">
              {(field) => <FieldInput field={field} label="compose_svc" mono />}
            </form.Field>
            <form.Field name="root">
              {(field) => <FieldInput field={field} label="root" mono />}
            </form.Field>
            <form.Field name="git_remote">
              {(field) => <FieldInput field={field} label="git_remote" />}
            </form.Field>
            <form.Field name="branch">
              {(field) => <FieldInput field={field} label="branch" />}
            </form.Field>
            <form.Field name="repo">
              {(field) => <FieldInput field={field} label="repo" mono />}
            </form.Field>
            <form.Field name="build_repo">
              {(field) => <FieldInput field={field} label="build_repo" mono />}
            </form.Field>
          </div>

          <details className="text-xs">
            <summary className="cursor-pointer select-none text-muted-foreground">
              Draft TOML
            </summary>
            <pre className="mt-2 overflow-x-auto rounded-lg border bg-muted/40 p-3 font-mono">
              {tomlText}
            </pre>
          </details>
        </form>

        <Dialog open={dangerOpen} onOpenChange={setDangerOpen}>
          <DialogContent className="max-w-2xl">
            <DialogHeader>
              <DialogTitle className="flex items-center gap-2 text-destructive">
                <TriangleAlert className="size-5" />
                Danger zone
              </DialogTitle>
              <DialogDescription>
                These values are managed by deploys or shared across the whole
                estate. A wrong port, slot, or strategy can take down this app
                — or every app behind nginx. Hand-editing the release chain can
                brick rollbacks. Change them only if you know why.
              </DialogDescription>
            </DialogHeader>
            <form
              id="registry-danger-form"
              className="space-y-4"
              onSubmit={(e) => {
                e.preventDefault()
                form.handleSubmit()
              }}
            >
              <div className="grid gap-4 sm:grid-cols-2">
                <form.Field name="strategy">
                  {(field) => (
                    <FieldInput field={field} label="strategy" placeholder="swap / replace" />
                  )}
                </form.Field>
                <form.Field name="writes_state">
                  {(field) => {
                    const isInvalid =
                      field.state.meta.isTouched && !field.state.meta.isValid
                    return (
                      <Field orientation="horizontal" data-invalid={isInvalid}>
                        <Checkbox
                          id={field.name}
                          name={field.name}
                          checked={field.state.value}
                          onCheckedChange={(checked) =>
                            field.handleChange(checked)
                          }
                          aria-invalid={isInvalid}
                        />
                        <FieldContent>
                          <FieldLabel
                            htmlFor={field.name}
                            className="font-normal"
                          >
                            writes_state
                          </FieldLabel>
                          <FieldDescription>
                            forbids the swap strategy
                          </FieldDescription>
                        </FieldContent>
                        {isInvalid && (
                          <FieldError errors={field.state.meta.errors} />
                        )}
                      </Field>
                    )
                  }}
                </form.Field>
                <form.Field name="front_port">
                  {(field) => (
                    <FieldInput field={field} label="front_port" type="number" />
                  )}
                </form.Field>
                <form.Field name="slot">
                  {(field) => (
                    <FieldInput field={field} label="slot" type="number" />
                  )}
                </form.Field>
                <form.Field name="live_port">
                  {(field) => (
                    <FieldInput field={field} label="live_port" type="number" />
                  )}
                </form.Field>
                <form.Field name="old_port">
                  {(field) => (
                    <FieldInput field={field} label="old_port" type="number" />
                  )}
                </form.Field>
                <form.Field name="release">
                  {(field) => (
                    <FieldInput field={field} label="release" mono />
                  )}
                </form.Field>
                <form.Field name="old_release">
                  {(field) => (
                    <FieldInput field={field} label="old_release" mono />
                  )}
                </form.Field>
              </div>

              {saveError && (
                <div
                  role="alert"
                  className="rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive"
                >
                  {saveError}
                </div>
              )}

              <DialogFooter>
                <div className="flex w-full flex-wrap items-center gap-2">
                  {isDirty && <Badge variant="secondary">unsaved changes</Badge>}
                  <span className="flex-1" />
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => setDangerOpen(false)}
                  >
                    Cancel
                  </Button>
                  <Button
                    type="submit"
                    size="sm"
                    disabled={saving || !isDirty || errorCount > 0}
                    title={(saving || !isDirty || errorCount > 0) ? saveHint : undefined}
                  >
                    {saving ? 'Saving…' : 'Save'}
                  </Button>
                </div>
              </DialogFooter>
            </form>
          </DialogContent>
        </Dialog>
      </CardContent>
    </Card>
  )
}
