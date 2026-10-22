import { Link } from "@tanstack/react-router"
import { ArrowRight, CircleAlert, TriangleAlert } from "lucide-react"
import type { App, Problem } from "@/api"
import { plural } from "@/lib/format"
import { cn } from "cn"
import { Button } from "@/components/ui/button"

/**
 * Registry findings for this app, shown where they can be acted on.
 * Host-level findings stay in Doctor; these are the ones the registry editor
 * can fix.
 */
export function ProblemsCard({ app, problems }: { app: App; problems: Problem[] }) {
  if (problems.length === 0) return null
  const errors = problems.filter((problem) => problem.severity === "error").length

  return (
    <section
      className={cn(
        "overflow-hidden rounded-xl border bg-card shadow-[var(--shadow-card)]",
        errors > 0 ? "border-destructive/25" : "border-warning/25"
      )}
    >
      <div className="flex items-start justify-between gap-3 border-b border-border/60 px-4 py-3.5">
        <div className="flex items-start gap-2.5">
          {errors > 0 ? (
            <CircleAlert className="mt-0.5 size-4 shrink-0 text-destructive-strong" />
          ) : (
            <TriangleAlert className="mt-0.5 size-4 shrink-0 text-warning" />
          )}
          <div>
            <h2 className="text-sm font-medium tracking-[-0.01em]">
              {plural(problems.length, "registry finding")}
            </h2>
            <p className="text-2xs text-muted-foreground">
              {errors > 0
                ? "The next mutating command will refuse until these are fixed."
                : "Deploys still run; these are worth fixing when you next edit the app."}
            </p>
          </div>
        </div>
        <Button
          variant="outline"
          size="xs"
          render={
            <Link to="/apps/$appName" params={{ appName: app.name }} search={{ tab: "configuration" }} />
          }
        >
          Edit registry
          <ArrowRight />
        </Button>
      </div>
      <div>
        {problems.map((problem, index) => (
          <div
            key={`${problem.code}-${index}`}
            className="flex items-start gap-2.5 border-b border-border/60 px-4 py-2.5 text-xs last:border-0"
          >
            <span
              className={cn(
                "mt-1 size-1.5 shrink-0 rounded-full",
                problem.severity === "error" ? "bg-destructive" : "bg-warning"
              )}
            />
            <div className="min-w-0">
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="font-mono text-2xs text-subtle-foreground">{problem.code}</span>
                <span className="text-2xs text-subtle-foreground">·</span>
                <span className="text-2xs text-subtle-foreground">{problem.severity}</span>
              </div>
              <p className="mt-0.5 text-pretty leading-relaxed text-muted-foreground">
                {problem.message}
              </p>
            </div>
          </div>
        ))}
      </div>
    </section>
  )
}
