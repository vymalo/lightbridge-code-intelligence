import { cn } from "@/lib/cn";
import { type StatusVariant, statusVisual } from "@/lib/tasks";

const VARIANT_CLASS: Record<StatusVariant, string> = {
  pending: "status-pending",
  active: "status-active",
  success: "status-success",
  error: "status-error",
  muted: "status-muted",
};

/** Capsule status badge for a task run (ADR-0016 status model). */
export function StatusPill({ status, className }: { status: string; className?: string }) {
  const { variant, label } = statusVisual(status);
  return <span className={cn("status-pill", VARIANT_CLASS[variant], className)}>{label}</span>;
}
