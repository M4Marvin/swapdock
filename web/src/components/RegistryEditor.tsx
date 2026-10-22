import { useEffect, useState, type ReactNode } from "react"
import { useForm, useStore, type AnyFieldApi } from "@tanstack/react-form"
import { useMutation, useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"
import { z } from "zod"
import {
  Check,
  Code2,
  Copy,
  Save,
  SlidersHorizontal,
  TriangleAlert,
} from "lucide-react"
import { api, type App } from "@/api"
import { queryKeys } from "@/lib/query-keys"
import { errorText, plural } from "@/lib/format"
import { useCopy } from "@/lib/hooks"
import { cn } from "cn"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import { Checkbox } from "@/components/ui/checkbox"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  Field,
  FieldContent,
  FieldDescription,
  FieldError,
  FieldLabel,
} from "@/components/ui/field"
import { Callout, SectionHeading } from "@/components/kit"

// ---------------------------------------------------------------------------
// schema
// ---------------------------------------------------------------------------

/** A required whole number in a range; the draft keeps it as a string. */
function requiredInt(min: number, max: number, message: string) {
  return z.string().refine((value) => {
    if (value.trim() === "") return false
    const n = Number(value)
    return Number.isInteger(n) && n >= min && n <= max
  }, message)
}

/** An optional whole number in a range; empty string means "unset". */
function optionalInt(min: number, max: number, message: string) {
  return z.string().refine((value) => {
    if (value.trim() === "") return true
    const n = Number(value)
    return Number.isInteger(n) && n >= min && n <= max
  }, message)
}

const PORT_MESSAGE = "must be a port between 1 and 65535"

const draftSchema = z
  .object({
    strategy: z
      .string()
      .refine((value: string): boolean => value === "swap" || value === "replace", 'must be "swap" or "replace"'),
    hostnames: z.string(),
    listen: z.string(),
    front_port: requiredInt(1, 65535, PORT_MESSAGE),
    slot: requiredInt(0, 127, "must be a slot between 0 and 127"),
    live_port: optionalInt(1, 65535, PORT_MESSAGE),
    old_port: optionalInt(1, 65535, PORT_MESSAGE),
    writes_state: z.boolean(),
    image_repo: z.string(),
    registry: z
      .string()
      .refine(
        (value) => value === "" || ["ghcr", "docker-hub", "local"].includes(value),
        "one of ghcr, docker-hub, local"
      ),
    build_host: z.string(),
    release: z.string(),
    old_release: z.string(),
    health_url: z
      .string()
      .refine((value) => value === "" || value.startsWith("http"), "must start with http:// or https://"),
    compose_dir: z.string(),
    compose_svc: z.string(),
    env_name: z
      .string()
      .refine((value) => value === "" || /^[A-Z][A-Z0-9_]*$/.test(value), "must be uppercase, e.g. CHAT_PORT"),
    git_remote: z.string(),
    branch: z.string(),
    repo: z.string(),
    root: z.string(),
    spa: z.boolean(),
    build_repo: z.string(),
    compose_override: z.string(),
    public_url: z
      .string()
      .refine(
        (value) => value === "" || value.startsWith("http://") || value.startsWith("https://"),
        "must start with http:// or https://"
      ),
    backup_command: z.string(),
    backup_restore_command: z.string(),
    backup_verify_command: z.string(),
    backup_keep: z.string(),
    backup_offsite_command: z.string(),
    backup_dir: z.string(),
    backup_before_deploy: z.boolean(),
  })
  // A front port doubles as a back-end port only if it collides with this
  // app's own live/old port; cross-app collisions need the whole registry.
  .superRefine((draft, ctx) => {
    const front = Number(draft.front_port)
    if (!Number.isInteger(front)) return
    if (draft.live_port.trim() !== "" && front === Number(draft.live_port)) {
      ctx.addIssue({
        code: "custom",
        path: ["front_port"],
        message: `front port also used as live port (${front})`,
      })
    } else if (draft.old_port.trim() !== "" && front === Number(draft.old_port)) {
      ctx.addIssue({
        code: "custom",
        path: ["front_port"],
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
    strategy: app.strategy ?? "",
    hostnames: app.hostnames.join(", "),
    listen: app.listen.join(", "),
    front_port: app.front_port != null ? String(app.front_port) : "",
    slot: app.slot != null ? String(app.slot) : "",
    live_port: app.live_port != null ? String(app.live_port) : "",
    old_port: app.old_port != null ? String(app.old_port) : "",
    writes_state: app.writes_state,
    image_repo: app.image_repo ?? "",
    registry: app.registry ?? "",
    build_host: app.build_host ?? "",
    release: app.release ?? "",
    old_release: app.old_release ?? "",
    health_url: app.health_url ?? "",
    compose_dir: app.compose_dir ?? "",
    compose_svc: app.compose_svc ?? "",
    env_name: app.env_name ?? "",
    git_remote: app.git_remote ?? "",
    branch: app.branch ?? "",
    repo: app.repo ?? "",
    root: app.root ?? "",
    spa: app.spa ?? false,
    build_repo: app.build_repo ?? "",
    compose_override: app.compose_override ?? "",
    public_url: app.public_url ?? "",
    backup_command: app.backup?.command.join("\n") ?? "",
    backup_restore_command: app.backup?.restore_command?.join("\n") ?? "",
    backup_verify_command: app.backup?.verify_command?.join("\n") ?? "",
    backup_keep: app.backup?.keep != null ? String(app.backup.keep) : "",
    backup_offsite_command: app.backup?.offsite_command?.join("\n") ?? "",
    backup_dir: app.backup?.dir ?? "",
    backup_before_deploy: app.backup?.before_deploy ?? false,
  }
}

function splitList(raw: string): string[] {
  return raw
    .split(",")
    .map((value) => value.trim())
    .filter((value) => value !== "")
}

/** Builds the PUT body from the app identity plus the validated draft. */
function fromDraft(app: App, draft: Draft): App {
  const num = (raw: string): number | null => {
    const value = raw.trim()
    if (value === "") return null
    const n = Number(value)
    return Number.isInteger(n) ? n : null
  }
  const str = (raw: string): string | null => {
    const value = raw.trim()
    return value === "" ? null : value
  }
  const backupCommand = draft.backup_command
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "")
  const restoreCommand = draft.backup_restore_command
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "")
  const verifyCommand = draft.backup_verify_command
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "")
  const offsiteCommand = draft.backup_offsite_command
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "")
  const keep = draft.backup_keep.trim() === "" ? null : Number(draft.backup_keep)
  return {
    ...app,
    strategy: draft.strategy.trim(),
    hostnames: splitList(draft.hostnames),
    listen: splitList(draft.listen),
    front_port: num(draft.front_port) ?? app.front_port,
    slot: num(draft.slot) ?? app.slot,
    live_port: num(draft.live_port),
    old_port: num(draft.old_port),
    writes_state: draft.writes_state,
    image_repo: str(draft.image_repo),
    registry: str(draft.registry),
    build_host: str(draft.build_host),
    release: str(draft.release),
    old_release: str(draft.old_release),
    health_url: str(draft.health_url),
    compose_dir: str(draft.compose_dir),
    compose_svc: str(draft.compose_svc),
    env_name: str(draft.env_name),
    git_remote: str(draft.git_remote),
    branch: str(draft.branch),
    repo: str(draft.repo),
    root: str(draft.root),
    spa: draft.spa,
    build_repo: str(draft.build_repo),
    compose_override: str(draft.compose_override),
    public_url: str(draft.public_url),
    backup:
      backupCommand.length > 0
        ? {
            command: backupCommand,
            restore_command: restoreCommand.length > 0 ? restoreCommand : null,
            verify_command: verifyCommand.length > 0 ? verifyCommand : null,
            keep: keep != null && Number.isInteger(keep) && keep > 0 ? keep : null,
            offsite_command: offsiteCommand.length > 0 ? offsiteCommand : null,
            dir: str(draft.backup_dir),
            before_deploy: draft.backup_before_deploy,
          }
        : null,
  }
}

/** Renders a string as a TOML basic string. */
function quote(value: string): string {
  return JSON.stringify(value)
}

/** Renders a draft as TOML, omitting empty optional fields. */
function toToml(app: App, draft: Draft): string {
  const out: string[] = []
  const str = (key: string, value: string) => {
    if (value.trim() !== "") out.push(`${key} = ${quote(value.trim())}`)
  }
  const num = (key: string, value: string) => {
    const trimmed = value.trim()
    if (trimmed === "") return
    out.push(`${key} = ${/^\d+$/.test(trimmed) ? trimmed : quote(trimmed)}`)
  }
  const arr = (key: string, value: string) => {
    const items = splitList(value)
    out.push(`${key} = [${items.map(quote).join(", ")}]`)
  }

  str("name", app.name)
  str("kind", app.kind)
  str("strategy", draft.strategy)
  arr("hostnames", draft.hostnames)
  arr("listen", draft.listen)
  num("front_port", draft.front_port)
  num("slot", draft.slot)
  num("live_port", draft.live_port)
  num("old_port", draft.old_port)
  out.push(`writes_state = ${draft.writes_state}`)
  str("image_repo", draft.image_repo)
  str("registry", draft.registry)
  str("release", draft.release)
  str("old_release", draft.old_release)
  str("build_host", draft.build_host)
  str("root", draft.root)
  if (draft.spa) out.push("spa = true")
  str("health_url", draft.health_url)
  str("compose_dir", draft.compose_dir)
  str("compose_svc", draft.compose_svc)
  str("env_name", draft.env_name)
  str("git_remote", draft.git_remote)
  str("branch", draft.branch)
  str("repo", draft.repo)
  str("build_repo", draft.build_repo)
  str("compose_override", draft.compose_override)
  str("public_url", draft.public_url)

  const backupCommand = draft.backup_command
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "")
  if (backupCommand.length > 0) {
    out.push("[backup]")
    out.push(`command = [${backupCommand.map(quote).join(", ")}]`)
    const restoreCommand = draft.backup_restore_command
      .split("\n")
      .map((line) => line.trim())
      .filter((line) => line !== "")
    if (restoreCommand.length > 0) {
      out.push(`restore_command = [${restoreCommand.map(quote).join(", ")}]`)
    }
    const verifyCommand = draft.backup_verify_command
      .split("\n")
      .map((line) => line.trim())
      .filter((line) => line !== "")
    if (verifyCommand.length > 0) {
      out.push(`verify_command = [${verifyCommand.map(quote).join(", ")}]`)
    }
    const offsiteCommand = draft.backup_offsite_command
      .split("\n")
      .map((line) => line.trim())
      .filter((line) => line !== "")
    if (offsiteCommand.length > 0) {
      out.push(`offsite_command = [${offsiteCommand.map(quote).join(", ")}]`)
    }
    if (draft.backup_keep.trim() !== "") {
      const keep = Number(draft.backup_keep)
      if (Number.isInteger(keep) && keep > 0) out.push(`keep = ${keep}`)
    }
    str("dir", draft.backup_dir)
    if (draft.backup_before_deploy) out.push("before_deploy = true")
  }

  return out.join("\n") + "\n"
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
  className,
}: {
  field: AnyFieldApi
  label: string
  placeholder?: string
  description?: ReactNode
  mono?: boolean
  type?: "text" | "number"
  className?: string
}) {
  const isInvalid = field.state.meta.isTouched && !field.state.meta.isValid
  return (
    <Field data-invalid={isInvalid} className={className}>
      <FieldLabel htmlFor={field.name}>{label}</FieldLabel>
      <Input
        id={field.name}
        name={field.name}
        type={type}
        value={(field.state.value as string) ?? ""}
        onBlur={field.handleBlur}
        onChange={(event) => field.handleChange(event.target.value)}
        aria-invalid={isInvalid}
        placeholder={placeholder}
        className={mono ? "font-mono" : undefined}
        autoComplete="off"
        spellCheck={false}
      />
      {description ? <FieldDescription>{description}</FieldDescription> : null}
      {isInvalid ? <FieldError errors={field.state.meta.errors} /> : null}
    </Field>
  )
}

function ConfigSection({
  title,
  description,
  children,
  className,
}: {
  title: string
  description: string
  children: ReactNode
  className?: string
}) {
  return (
    <section
      className={cn(
        "overflow-hidden rounded-xl border border-border bg-card shadow-[var(--shadow-card)]",
        className
      )}
    >
      <div className="border-b border-border/60 px-4 py-3">
        <h2 className="text-sm font-medium tracking-[-0.01em]">{title}</h2>
        <p className="mt-0.5 text-pretty text-xs leading-relaxed text-muted-foreground">
          {description}
        </p>
      </div>
      <div className="grid gap-4 p-4 sm:grid-cols-2">{children}</div>
    </section>
  )
}

/**
 * The registry file as a form.
 *
 * Field errors block saving; the server re-validates the whole estate and
 * refuses on any error, keeping a `.bak` of the previous file. Deploy-owned
 * values (ports, slot, release chain, strategy) live behind the advanced
 * dialog with an explicit warning.
 */
export function RegistryEditor({ app }: { app: App }) {
  const queryClient = useQueryClient()
  const [advancedOpen, setAdvancedOpen] = useState(false)
  const { copied, copy } = useCopy()

  const saveMutation = useMutation({
    mutationFn: (payload: App) => api.updateApp(app.name, payload),
    onSuccess: (result) => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.app(app.name) })
      void queryClient.invalidateQueries({ queryKey: queryKeys.apps })
      void queryClient.invalidateQueries({ queryKey: queryKeys.validate })
      void queryClient.invalidateQueries({ queryKey: queryKeys.runs })
      toast.success("Registry saved", {
        description: `Re-validated · backup ${result.backup ? "written" : "kept"}${result.backup ? ` (${result.backup})` : ""}`,
      })
    },
    onError: (error) => {
      toast.error("Registry save refused", { description: errorText(error) })
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
        form.reset(value)
        setAdvancedOpen(false)
      } catch {
        // Surfaced from saveMutation.error next to the action bar and in the dialog.
      }
    },
  })

  const values = useStore(form.store, (state) => state.values)
  const isDirty = useStore(form.store, (state) => state.isDirty)
  const errorCount = useStore(
    form.store,
    (state) =>
      Object.values(state.fieldMeta ?? {}).filter(
        (meta) => meta && meta.errors && meta.errors.length > 0
      ).length
  )

  // Adopt server-side changes (a deploy rewrote the release) when there are no
  // unsaved edits; never clobber an in-progress draft.
  useEffect(() => {
    if (!isDirty) form.reset(toDraft(app))
  }, [app, isDirty, form])

  const saving = saveMutation.isPending
  const saveHint =
    errorCount > 0 ? "fix field errors first" : !isDirty ? "no changes to save" : undefined
  const saveError = saveMutation.error ? errorText(saveMutation.error) : null
  const tomlText = toToml(app, values)

  const save = () => {
    if (saving || !isDirty || errorCount > 0) return
    void form.handleSubmit()
  }

  const container = app.kind === "container"
  const staticApp = app.kind === "static"

  return (
    <div className="space-y-4 pb-16">
      <SectionHeading
        title="Registry configuration"
        description="The TOML file is the source of truth. Every save re-validates the whole estate and keeps a .bak of the previous file."
      />

      <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-border bg-card px-4 py-3 shadow-[var(--shadow-card)]">
        <div className="flex flex-wrap items-center gap-x-5 gap-y-2 text-xs">
          <span className="flex items-center gap-1.5">
            <span className="label-eyebrow">Name</span>
            <span className="font-mono text-foreground">{app.name}</span>
          </span>
          <span className="flex items-center gap-1.5">
            <span className="label-eyebrow">Kind</span>
            <span className="text-foreground">{app.kind}</span>
          </span>
          <span className="flex items-center gap-1.5">
            <span className="label-eyebrow">Strategy</span>
            <span className="text-foreground">{app.strategy}</span>
          </span>
          <span className="flex items-center gap-1.5">
            <span className="label-eyebrow">Live</span>
            <span className="font-mono text-foreground">{app.live_port ?? "—"}</span>
          </span>
          {isDirty ? <Badge variant="warning">unsaved changes</Badge> : null}
          {errorCount > 0 ? (
            <Badge variant="destructive">{plural(errorCount, "field error")}</Badge>
          ) : null}
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => {
              form.reset()
              saveMutation.reset()
            }}
            disabled={!isDirty && !saveMutation.isError}
          >
            Reset
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => void copy(tomlText)}
            title="Copy the draft as a TOML file"
          >
            {copied ? <Check className="text-success" /> : <Copy />}
            {copied ? "Copied" : "Copy TOML"}
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="border-warning/30 text-warning hover:bg-warning/10 hover:text-warning"
            onClick={() => setAdvancedOpen(true)}
          >
            <SlidersHorizontal />
            Advanced
          </Button>
          <Button
            type="button"
            size="sm"
            disabled={saving || !isDirty || errorCount > 0}
            title={saving || !isDirty || errorCount > 0 ? saveHint : undefined}
            onClick={save}
          >
            <Save />
            {saving ? "Saving…" : "Save"}
          </Button>
        </div>
      </div>

      {saveError && !advancedOpen ? (
        <Callout tone="danger" title="The server refused this draft">
          {saveError}
        </Callout>
      ) : null}

      <form
        id="registry-form"
        onSubmit={(event) => {
          event.preventDefault()
          void form.handleSubmit()
        }}
        className="space-y-4"
      >
        <ConfigSection
          title="Routing & domains"
          description="Where the app is reachable. nginx owns the front port; listen adds bind addresses beyond loopback."
        >
          <form.Field name="hostnames">
            {(field) => (
              <FieldInput
                field={field}
                label="Hostnames"
                placeholder="chat.example.com, www.chat.example.com"
                description="Comma-separated. Empty means no hostname routes here yet."
                mono
              />
            )}
          </form.Field>
          <form.Field name="listen">
            {(field) => (
              <FieldInput
                field={field}
                label="Listen addresses"
                placeholder="127.0.0.1, 10.0.0.2"
                description="Additional bind addresses beyond loopback."
                mono
              />
            )}
          </form.Field>
          <form.Field name="health_url">
            {(field) => (
              <FieldInput
                field={field}
                label="Health URL"
                placeholder="http://127.0.0.1:3000/up"
                description="The path is used; the port is the candidate's."
                mono
              />
            )}
          </form.Field>
          <form.Field name="public_url">
            {(field) => (
              <FieldInput
                field={field}
                label="Public URL"
                placeholder="https://chat.example.com/health"
                description="Optional. Used by Probe and public checks."
                mono
              />
            )}
          </form.Field>
        </ConfigSection>

        {container ? (
          <>
            <ConfigSection
              title="Image & source"
              description="The release image, the checkout it comes from, and where it is built."
            >
              <form.Field name="registry">
                {(field) => (
                  <FieldInput
                    field={field}
                    label="Registry"
                    placeholder="ghcr / docker-hub / local"
                    description="Local means the image is built on this host."
                  />
                )}
              </form.Field>
              <form.Field name="image_repo">
                {(field) => (
                  <FieldInput
                    field={field}
                    label="Image repository"
                    placeholder="charon"
                    description="The release tag is appended: repo:release."
                    mono
                  />
                )}
              </form.Field>
              <form.Field name="build_host">
                {(field) => (
                  <FieldInput
                    field={field}
                    label="Build host"
                    placeholder="local or an ssh destination"
                    description="A config value, not code."
                  />
                )}
              </form.Field>
              <form.Field name="git_remote">
                {(field) => (
                  <FieldInput field={field} label="Git remote" placeholder="org/repo" />
                )}
              </form.Field>
              <form.Field name="branch">
                {(field) => (
                  <FieldInput field={field} label="Branch" placeholder="main" mono />
                )}
              </form.Field>
              <form.Field name="repo">
                {(field) => (
                  <FieldInput
                    field={field}
                    label="Repository path"
                    placeholder="/srv/apps/charon"
                    description="The checkout swapdock syncs and reads HEAD from."
                    mono
                  />
                )}
              </form.Field>
              <form.Field name="build_repo">
                {(field) => (
                  <FieldInput
                    field={field}
                    label="Build repository"
                    placeholder="optional separate checkout"
                    description="Only when the build host uses a different checkout."
                    mono
                    className="sm:col-span-2"
                  />
                )}
              </form.Field>
            </ConfigSection>

            <ConfigSection
              title="Compose"
              description="How swapdock starts the app. The generated override injects the name, port and image at deploy time."
            >
              <form.Field name="compose_dir">
                {(field) => (
                  <FieldInput
                    field={field}
                    label="Compose directory"
                    placeholder="/home/marv/apps"
                    description="The directory holding compose.yml."
                    mono
                  />
                )}
              </form.Field>
              <form.Field name="compose_svc">
                {(field) => (
                  <FieldInput field={field} label="Compose service" placeholder="chat" mono />
                )}
              </form.Field>
              <form.Field name="env_name">
                {(field) => (
                  <FieldInput
                    field={field}
                    label="Port env variable"
                    placeholder="CHAT_PORT"
                    description="The compose file reads 127.0.0.1:${CHAT_PORT}:80."
                    mono
                  />
                )}
              </form.Field>
              <form.Field name="compose_override">
                {(field) => (
                  <FieldInput
                    field={field}
                    label="Compose override"
                    placeholder="/srv/swapdock/overrides/chat.yml"
                    description="Generated by adopt, when present."
                    mono
                  />
                )}
              </form.Field>
            </ConfigSection>
          </>
        ) : null}

        {staticApp ? (
          <ConfigSection
            title="Static release"
            description="A static app serves a directory; the flip is an atomic symlink rename."
          >
            <form.Field name="root">
              {(field) => (
                <FieldInput
                  field={field}
                  label="Root directory"
                  placeholder="/srv/www/my-site"
                  description="The directory nginx serves."
                  mono
                  className="sm:col-span-2"
                />
              )}
            </form.Field>
            <form.Field name="spa">
              {(field) => {
                const invalid = field.state.meta.isTouched && !field.state.meta.isValid
                return (
                  <Field orientation="horizontal" data-invalid={invalid} className="sm:col-span-2">
                    <Checkbox
                      id={field.name}
                      name={field.name}
                      checked={field.state.value}
                      onCheckedChange={(checked) => field.handleChange(checked === true)}
                      aria-invalid={invalid}
                    />
                    <FieldContent>
                      <FieldLabel htmlFor={field.name} className="font-normal">
                        Single-page app
                      </FieldLabel>
                      <FieldDescription>
                        Extensionless paths fall back to index.html; a missing asset keeps
                        its 404.
                      </FieldDescription>
                    </FieldContent>
                    {invalid ? <FieldError errors={field.state.meta.errors} /> : null}
                  </Field>
                )
              }}
            </form.Field>
          </ConfigSection>
        ) : null}

        <ConfigSection
          title="Backup"
          description="An argv command, not a shell string. Runs once from the CLI or before every deploy when enabled."
        >
          <form.Field name="backup_command">
            {(field) => {
              const invalid = field.state.meta.isTouched && !field.state.meta.isValid
              return (
                <Field data-invalid={invalid} className="sm:col-span-2">
                  <FieldLabel htmlFor={field.name}>Command</FieldLabel>
                  <Textarea
                    id={field.name}
                    name={field.name}
                    value={(field.state.value as string) ?? ""}
                    onBlur={field.handleBlur}
                    onChange={(event) => field.handleChange(event.target.value)}
                    aria-invalid={invalid}
                    placeholder={"sqlite3\n/srv/apps/chat/data/app.db\n.backup {file}"}
                    className="min-h-24 font-mono"
                    spellCheck={false}
                  />
                  <FieldDescription>
                    One argument per line. Placeholders: <span className="font-mono">{"{file}"}</span>,{" "}
                    <span className="font-mono">{"{app}"}</span>,{" "}
                    <span className="font-mono">{"{release}"}</span>,{" "}
                    <span className="font-mono">{"{timestamp}"}</span>.
                  </FieldDescription>
                  {invalid ? <FieldError errors={field.state.meta.errors} /> : null}
                </Field>
              )
            }}
          </form.Field>
          <form.Field name="backup_restore_command">
            {(field) => {
              const invalid = field.state.meta.isTouched && !field.state.meta.isValid
              return (
                <Field data-invalid={invalid} className="sm:col-span-2">
                  <FieldLabel htmlFor={field.name}>Restore command</FieldLabel>
                  <Textarea
                    id={field.name}
                    name={field.name}
                    value={(field.state.value as string) ?? ""}
                    onBlur={field.handleBlur}
                    onChange={(event) => field.handleChange(event.target.value)}
                    aria-invalid={invalid}
                    placeholder={"sqlite3\n/srv/apps/chat/data/app.db\n.restore {file}"}
                    className="min-h-24 font-mono"
                    spellCheck={false}
                  />
                  <FieldDescription>
                    Optional argv for <span className="font-mono">swapdock restore</span>. Same
                    placeholders as the backup command. A snapshot without a tested restore
                    path is not a backup.
                  </FieldDescription>
                  {invalid ? <FieldError errors={field.state.meta.errors} /> : null}
                </Field>
              )
            }}
          </form.Field>
          <form.Field name="backup_verify_command">
            {(field) => {
              const invalid = field.state.meta.isTouched && !field.state.meta.isValid
              return (
                <Field data-invalid={invalid} className="sm:col-span-2">
                  <FieldLabel htmlFor={field.name}>Verify command</FieldLabel>
                  <Textarea
                    id={field.name}
                    name={field.name}
                    value={(field.state.value as string) ?? ""}
                    onBlur={field.handleBlur}
                    onChange={(event) => field.handleChange(event.target.value)}
                    aria-invalid={invalid}
                    placeholder={"gzip\n-t\n{file}"}
                    className="min-h-24 font-mono"
                    spellCheck={false}
                  />
                  <FieldDescription>
                    Optional argv that checks the new snapshot without restoring it. A non-zero
                    exit fails the backup. Same placeholders and program allowlist as the dump.
                  </FieldDescription>
                  {invalid ? <FieldError errors={field.state.meta.errors} /> : null}
                </Field>
              )
            }}
          </form.Field>
          <form.Field name="backup_offsite_command">
            {(field) => {
              const invalid = field.state.meta.isTouched && !field.state.meta.isValid
              return (
                <Field data-invalid={invalid} className="sm:col-span-2">
                  <FieldLabel htmlFor={field.name}>Offsite command</FieldLabel>
                  <Textarea
                    id={field.name}
                    name={field.name}
                    value={(field.state.value as string) ?? ""}
                    onBlur={field.handleBlur}
                    onChange={(event) => field.handleChange(event.target.value)}
                    aria-invalid={invalid}
                    placeholder={"rclone\ncopy\n{file}\nremote:swapdock/chat/"}
                    className="min-h-24 font-mono"
                    spellCheck={false}
                  />
                  <FieldDescription>
                    Optional argv that copies the snapshot off the host. Same placeholders and
                    program allowlist; a non-zero exit fails the backup.
                  </FieldDescription>
                  {invalid ? <FieldError errors={field.state.meta.errors} /> : null}
                </Field>
              )
            }}
          </form.Field>
          <form.Field name="backup_keep">
            {(field) => (
              <FieldInput
                field={field}
                label="Keep snapshots"
                placeholder="14"
                description="Keep the newest N snapshots in the backup directory; empty disables pruning."
                mono
              />
            )}
          </form.Field>
          <form.Field name="backup_dir">
            {(field) => (
              <FieldInput
                field={field}
                label="Backup directory"
                placeholder="/srv/swapdock/backups/chat"
                description="Where {file} is written when the command uses it."
                mono
              />
            )}
          </form.Field>
          <form.Field name="backup_before_deploy">
            {(field) => {
              const invalid = field.state.meta.isTouched && !field.state.meta.isValid
              return (
                <Field orientation="horizontal" data-invalid={invalid} className="items-center">
                  <Checkbox
                    id={field.name}
                    name={field.name}
                    checked={field.state.value}
                    onCheckedChange={(checked) => field.handleChange(checked === true)}
                    aria-invalid={invalid}
                  />
                  <FieldContent>
                    <FieldLabel htmlFor={field.name} className="font-normal">
                      Back up before deploy
                    </FieldLabel>
                    <FieldDescription>Run this command before every up.</FieldDescription>
                  </FieldContent>
                  {invalid ? <FieldError errors={field.state.meta.errors} /> : null}
                </Field>
              )
            }}
          </form.Field>
        </ConfigSection>

        <details className="group rounded-xl border border-border bg-card">
          <summary className="flex cursor-pointer list-none items-center gap-2 px-4 py-3 text-sm font-medium select-none">
            <Code2 className="size-4 text-subtle-foreground" />
            Draft TOML
            <span className="ml-auto text-2xs font-normal text-subtle-foreground">
              {isDirty ? "unsaved" : "matches the registry"}
            </span>
          </summary>
          <div className="border-t border-border/60 p-4">
            <div className="mb-2 flex justify-end">
              <Button type="button" variant="outline" size="xs" onClick={() => void copy(tomlText)}>
                {copied ? <Check className="text-success" /> : <Copy />}
                {copied ? "Copied" : "Copy file"}
              </Button>
            </div>
            <pre className="code-surface max-h-80 overflow-auto rounded-lg border border-border p-3 font-mono text-[11px] leading-relaxed">
              {tomlText}
            </pre>
          </div>
        </details>
      </form>

      {isDirty ? (
        <div className="animate-slide-up fixed inset-x-4 bottom-4 z-40 sm:right-auto sm:left-6 sm:w-auto lg:left-[272px]">
          <div className="flex items-center gap-3 rounded-xl border border-border bg-popover px-3 py-2 shadow-[var(--shadow-pop)]">
            <span className="text-xs text-muted-foreground">
              {errorCount > 0 ? plural(errorCount, "field error") : "Unsaved registry changes"}
            </span>
            {errorCount > 0 ? <Badge variant="destructive">blocked</Badge> : null}
            <Button
              type="button"
              variant="ghost"
              size="xs"
              onClick={() => {
                form.reset()
                saveMutation.reset()
              }}
            >
              Reset
            </Button>
            <Button
              type="button"
              size="xs"
              disabled={saving || errorCount > 0}
              title={errorCount > 0 ? "fix field errors first" : undefined}
              onClick={save}
            >
              <Save />
              {saving ? "Saving…" : "Save changes"}
            </Button>
          </div>
        </div>
      ) : null}

      <Dialog open={advancedOpen} onOpenChange={setAdvancedOpen}>
        <DialogContent className="sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2 text-warning">
              <TriangleAlert className="size-4" />
              Advanced registry fields
            </DialogTitle>
            <DialogDescription>
              These values are managed by deploys or shared across the whole estate. A wrong
              port, slot or strategy can take down this app, or every app behind nginx.
            </DialogDescription>
          </DialogHeader>

          <form
            id="registry-advanced-form"
            className="space-y-4"
            onSubmit={(event) => {
              event.preventDefault()
              void form.handleSubmit()
            }}
          >
            <div className="grid gap-4 sm:grid-cols-2">
              <form.Field name="strategy">
                {(field) => <FieldInput field={field} label="Strategy" placeholder="swap / replace" />}
              </form.Field>
              <form.Field name="writes_state">
                {(field) => {
                  const invalid = field.state.meta.isTouched && !field.state.meta.isValid
                  return (
                    <Field orientation="horizontal" data-invalid={invalid} className="items-center">
                      <Checkbox
                        id={field.name}
                        name={field.name}
                        checked={field.state.value}
                        onCheckedChange={(checked) => field.handleChange(checked === true)}
                        aria-invalid={invalid}
                      />
                      <FieldContent>
                        <FieldLabel htmlFor={field.name} className="font-normal">
                          Writes state
                        </FieldLabel>
                        <FieldDescription>Forbids the swap strategy.</FieldDescription>
                      </FieldContent>
                    </Field>
                  )
                }}
              </form.Field>
              <form.Field name="front_port">
                {(field) => <FieldInput field={field} label="Front port" type="number" mono />}
              </form.Field>
              <form.Field name="slot">
                {(field) => <FieldInput field={field} label="Slot" type="number" />}
              </form.Field>
              <form.Field name="live_port">
                {(field) => <FieldInput field={field} label="Live port" type="number" mono />}
              </form.Field>
              <form.Field name="old_port">
                {(field) => <FieldInput field={field} label="Old port" type="number" mono />}
              </form.Field>
              <form.Field name="release">
                {(field) => <FieldInput field={field} label="Release" mono />}
              </form.Field>
              <form.Field name="old_release">
                {(field) => <FieldInput field={field} label="Old release" mono />}
              </form.Field>
            </div>

            <Callout tone="warning" title="Hand-editing the release chain can brick rollbacks">
              Deploys advance these fields atomically. Change them only if you know why.
            </Callout>

            {saveError ? (
              <Callout tone="danger" title="The server refused this draft">
                {saveError}
              </Callout>
            ) : null}
          </form>

          <DialogFooter>
            {isDirty ? <Badge variant="warning">unsaved changes</Badge> : null}
            <span className="flex-1" />
            <Button variant="outline" onClick={() => setAdvancedOpen(false)}>
              Cancel
            </Button>
            <Button
              type="button"
              disabled={saving || !isDirty || errorCount > 0}
              title={saving || !isDirty || errorCount > 0 ? saveHint : undefined}
              onClick={save}
            >
              <Save />
              {saving ? "Saving…" : "Save"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
