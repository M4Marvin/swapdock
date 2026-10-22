import { mergeProps } from "@base-ui/react/merge-props"
import { useRender } from "@base-ui/react/use-render"
import { cva, type VariantProps } from "class-variance-authority"
import { cn } from "cn"

const badgeVariants = cva(
  "group/badge inline-flex h-5 w-fit shrink-0 items-center justify-center gap-1 overflow-hidden rounded-full border border-transparent px-2 py-0.5 text-2xs font-medium whitespace-nowrap transition-colors focus-visible:ring-[3px] focus-visible:ring-ring/40 has-data-[icon=inline-end]:pr-1.5 has-data-[icon=inline-start]:pl-1.5 aria-invalid:border-destructive [&>svg]:pointer-events-none [&>svg]:size-3!",
  {
    variants: {
      variant: {
        default:
          "border-border bg-muted/70 text-muted-foreground [a]:hover:bg-muted [a]:hover:text-foreground",
        secondary:
          "border-border bg-secondary text-secondary-foreground [a]:hover:bg-secondary/80",
        destructive:
          "border-destructive/20 bg-destructive/10 text-destructive-strong focus-visible:ring-destructive/20 [a]:hover:bg-destructive/15",
        success:
          "border-success/20 bg-success/10 text-success-strong focus-visible:ring-success/20 [a]:hover:bg-success/15",
        warning:
          "border-warning/25 bg-warning/10 text-warning-strong focus-visible:ring-warning/20 [a]:hover:bg-warning/15",
        info: "border-info/20 bg-info/10 text-info-strong [a]:hover:bg-info/15",
        accent:
          "border-accent/20 bg-accent-soft text-accent [a]:hover:bg-accent/15",
        outline:
          "border-border bg-transparent text-muted-foreground [a]:hover:bg-muted [a]:hover:text-foreground",
        ghost: "hover:bg-muted hover:text-muted-foreground",
        link: "text-accent underline-offset-4 hover:underline",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  }
)

function Badge({
  className,
  variant = "default",
  render,
  ...props
}: useRender.ComponentProps<"span"> & VariantProps<typeof badgeVariants>) {
  return useRender({
    defaultTagName: "span",
    props: mergeProps<"span">(
      {
        className: cn(badgeVariants({ variant }), className),
      },
      props
    ),
    render,
    state: {
      slot: "badge",
      variant,
    },
  })
}

export { Badge, badgeVariants }
