import { useState } from "react"
import { useMutation, useQueryClient } from "@tanstack/react-query"
import { useNavigate } from "@tanstack/react-router"
import { toast } from "sonner"
import { Plus } from "lucide-react"
import { ApiError, api } from "@/api"
import { queryKeys } from "@/lib/query-keys"
import { errorText } from "@/lib/format"
import { cn } from "cn"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Callout } from "@/components/kit"

type Kind = "container" | "static"

const NAME_RE = /^[a-z0-9][a-z0-9-]*$/
const KINDS: { value: Kind; title: string; description: string }[] = [
  {
    value: "container",
    title: "Container app",
    description: "Built from an image or a source checkout and gated by a health check.",
  },
  {
    value: "static",
    title: "Static site",
    description: "Files served by nginx from a root symlink; no container or build.",
  },
]

function validateName(raw: string): string | null {
  const name = raw.trim()
  if (name === "") return "Enter an app name."
  if (name.length > 64) return "Use 64 characters or fewer."
  if (!NAME_RE.test(name)) return "Use lowercase letters, digits and dashes."
  if (name.endsWith("-")) return "The name cannot end with a dash."
  return null
}

/** Create one registry entry from the console. Deliberately not a wizard. */
export function NewAppDialog({
  open,
  onOpenChange,
  onCreated,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  onCreated?: () => void
}) {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const [mode, setMode] = useState<"new" | "import">("new")
  const [name, setName] = useState("")
  const [kind, setKind] = useState<Kind>("container")
  const [composeDir, setComposeDir] = useState("")
  const [domain, setDomain] = useState("")
  const [touched, setTouched] = useState(false)

  const nameError = validateName(name)
  const importApp = useMutation({
    mutationFn: () =>
      api.importApp({
        compose_dir: composeDir.trim(),
        name: name.trim() || undefined,
        domain: domain.trim() || undefined,
      }),
    onSuccess: (report) => {
      const appName = report.app.name
      toast.success("Compose app imported", {
        description: `${appName} was added from ${report.service}. Review its hostname, then deploy.`,
      })
      void queryClient.invalidateQueries({ queryKey: queryKeys.apps })
      reset()
      onOpenChange(false)
      onCreated?.()
      void navigate({
        to: "/apps/$appName",
        params: { appName },
        search: { tab: "overview" },
      })
    },
    onError: (error) => {
      toast.error("Compose app was not imported", { description: errorText(error) })
    },
  })

  const create = useMutation({
    mutationFn: () => api.createApp(name.trim(), kind),
    onSuccess: () => {
      const appName = name.trim()
      toast.success("App created", {
        description: `${appName} was added to the registry. Review its ports and hostname before deploying.`,
      })
      void queryClient.invalidateQueries({ queryKey: queryKeys.apps })
      setName("")
      setKind("container")
      setTouched(false)
      onOpenChange(false)
      onCreated?.()
      void navigate({
        to: "/apps/$appName",
        params: { appName },
        search: { tab: "overview" },
      })
    },
    onError: (error) => {
      toast.error("App was not created", { description: errorText(error) })
    },
  })

  const activeError = create.error ?? importApp.error
  const errorIsConflict = activeError instanceof ApiError && activeError.status === 409
  const errorIsAuth = activeError instanceof ApiError && activeError.status === 401

  const submit = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    setTouched(true)
    if (mode === "import") {
      if (composeDir.trim() === "") return
      importApp.mutate()
      return
    }
    if (nameError) return
    create.mutate()
  }

  const reset = () => {
    setName("")
    setKind("container")
    setComposeDir("")
    setDomain("")
    setMode("new")
    setTouched(false)
    create.reset()
    importApp.reset()
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) reset()
        onOpenChange(next)
      }}
    >
      <DialogContent>
        <form onSubmit={submit} className="space-y-4">
          <DialogHeader>
            <DialogTitle>New app</DialogTitle>
            <DialogDescription>
              Writes one registry entry. You choose the ports and hostname next; nothing is
              deployed from this dialog.
            </DialogDescription>
          </DialogHeader>

          <div className="grid grid-cols-2 gap-1 rounded-lg border border-border bg-surface/40 p-1">
            <button
              type="button"
              onClick={() => setMode("new")}
              className={cn(
                "rounded-md px-3 py-2 text-xs font-medium transition-colors",
                mode === "new" ? "bg-surface text-foreground shadow-sm" : "text-muted-foreground",
              )}
            >
              New app
            </button>
            <button
              type="button"
              onClick={() => setMode("import")}
              className={cn(
                "rounded-md px-3 py-2 text-xs font-medium transition-colors",
                mode === "import" ? "bg-surface text-foreground shadow-sm" : "text-muted-foreground",
              )}
            >
              Import Compose
            </button>
          </div>

          {mode === "import" ? (
            <div className="space-y-3">
              <div className="space-y-1.5">
                <label htmlFor="new-app-compose" className="text-xs font-medium text-muted-foreground">
                  Compose directory
                </label>
                <Input
                  id="new-app-compose"
                  value={composeDir}
                  onChange={(event) => setComposeDir(event.target.value)}
                  placeholder="/home/me/apps/shop"
                  autoComplete="off"
                  spellCheck={false}
                  className="font-mono"
                />
                <p className="text-2xs text-subtle-foreground">
                  Directory containing compose.yaml or docker-compose.yml. Your Compose file is not edited.
                </p>
              </div>
              <div className="space-y-1.5">
                <label htmlFor="new-app-domain" className="text-xs font-medium text-muted-foreground">
                  Hostname (optional)
                </label>
                <Input
                  id="new-app-domain"
                  value={domain}
                  onChange={(event) => setDomain(event.target.value)}
                  placeholder="shop.example.com"
                  autoComplete="off"
                  spellCheck={false}
                  className="font-mono"
                />
              </div>
              <div className="space-y-1.5">
                <label htmlFor="new-app-import-name" className="text-xs font-medium text-muted-foreground">
                  App name (optional)
                </label>
                <Input
                  id="new-app-import-name"
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                  placeholder="from the service name"
                  autoComplete="off"
                  spellCheck={false}
                  className="font-mono"
                />
              </div>
            </div>
          ) : (
          <>
          <div className="space-y-1.5">
            <label htmlFor="new-app-name" className="text-xs font-medium text-muted-foreground">
              Name
            </label>
            <Input
              id="new-app-name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              onBlur={() => setTouched(true)}
              placeholder="my-app"
              autoComplete="off"
              spellCheck={false}
              autoFocus
              aria-invalid={touched && nameError != null}
              aria-describedby="new-app-name-hint"
              className="font-mono"
            />
            <p id="new-app-name-hint" className="text-2xs text-subtle-foreground">
              Lowercase letters, digits and dashes. The file is named{" "}
              <span className="font-mono">{name.trim() || "my-app"}.toml</span>.
            </p>
            {touched && nameError ? (
              <p role="alert" className="text-2xs text-destructive-strong">
                {nameError}
              </p>
            ) : null}
          </div>

          <fieldset className="space-y-1.5">
            <legend className="text-xs font-medium text-muted-foreground">Kind</legend>
            <div className="grid gap-2 sm:grid-cols-2">
              {KINDS.map((option) => (
                <label
                  key={option.value}
                  className={cn(
                    "flex cursor-pointer items-start gap-2.5 rounded-lg border p-2.5 transition-colors",
                    kind === option.value
                      ? "border-accent/35 bg-accent-soft/40"
                      : "border-border bg-surface/40 hover:border-border-strong"
                  )}
                >
                  <input
                    type="radio"
                    name="new-app-kind"
                    value={option.value}
                    checked={kind === option.value}
                    onChange={() => setKind(option.value)}
                    className="mt-0.5 size-3.5 shrink-0 accent-[var(--accent)]"
                  />
                  <span className="min-w-0">
                    <span className="block text-xs font-medium text-foreground">
                      {option.title}
                    </span>
                    <span className="mt-0.5 block text-pretty text-2xs leading-relaxed text-muted-foreground">
                      {option.description}
                    </span>
                  </span>
                </label>
              ))}
            </div>
          </fieldset>
          </>
          )}

          {create.isError || importApp.isError ? (
            <Callout
              tone="danger"
              title={
                errorIsConflict
                  ? "That app already exists"
                  : errorIsAuth
                    ? "The API rejected this session"
                    : mode === "import"
                      ? "The Compose app was not imported"
                      : "The app was not created"
              }
            >
              {errorText(activeError)}
            </Callout>
          ) : null}

          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={create.isPending || importApp.isPending}>
              <Plus />
              {mode === "import"
                ? importApp.isPending
                  ? "Importing…"
                  : "Import app"
                : create.isPending
                  ? "Creating…"
                  : "Create app"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}

/** Sidebar/empty-state trigger, so both surfaces share one dialog implementation. */
export function NewAppButton({
  variant = "outline",
  size = "sm",
  className,
  children,
  onCreated,
}: {
  variant?: "default" | "outline" | "ghost"
  size?: "sm" | "default"
  className?: string
  children?: React.ReactNode
  onCreated?: () => void
}) {
  const [open, setOpen] = useState(false)
  return (
    <>
      <Button variant={variant} size={size} className={className} onClick={() => setOpen(true)}>
        {children ?? (
          <>
            <Plus />
            New app
          </>
        )}
      </Button>
      <NewAppDialog open={open} onOpenChange={setOpen} onCreated={onCreated} />
    </>
  )
}
