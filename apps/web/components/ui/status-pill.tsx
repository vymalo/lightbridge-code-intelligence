import { cva } from "class-variance-authority";
import { type StatusVariant, statusVisual } from "@/lib/domain/tasks";
import { cn } from "@/lib/utils/cn";

// Status variant → daisyUI badge color (ADR-0027). `badge-soft` keeps the calm, tinted fill from the
// ADR-0016 status model; pending/cancelled stay neutral (ghost). Keyed by the domain `StatusVariant`.
const pillVariants = cva("badge badge-sm gap-1.5", {
  variants: {
    variant: {
      pending: "badge-ghost",
      active: "badge-info badge-soft",
      success: "badge-success badge-soft",
      error: "badge-error badge-soft",
      muted: "badge-ghost",
    } satisfies Record<StatusVariant, string>,
  },
  defaultVariants: { variant: "pending" },
});

/** A small status capsule (daisyUI `badge`) with a leading status dot — pulsing while active. Shared
 * by run statuses ([`StatusPill`]) and repo approval states (`approvalVisual`). */
export function Pill({
  variant,
  label,
  pulse,
  className,
}: {
  variant: StatusVariant;
  label: string;
  pulse?: boolean;
  className?: string;
}) {
  return (
    <span className={cn(pillVariants({ variant }), className)}>
      <span className={cn("size-1.5 rounded-full bg-current", pulse && "animate-pulse")} />
      {label}
    </span>
  );
}

/** Capsule status badge for a task run (ADR-0016 status model). */
export function StatusPill({ status, className }: { status: string; className?: string }) {
  const { variant, label } = statusVisual(status);
  return (
    <Pill variant={variant} label={label} pulse={variant === "active"} className={className} />
  );
}
