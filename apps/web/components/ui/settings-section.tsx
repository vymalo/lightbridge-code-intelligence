import type { ReactNode } from "react";
import { cn } from "@/lib/cn";

/** A grouped settings section (ADR-0024, Cursor pattern): an uppercase section label above a card of
 * label/description/control rows separated by hairline dividers. */
export function SettingsSection({
  title,
  children,
  className,
}: {
  title: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={cn("flex flex-col gap-2", className)}>
      <h2 className="px-1 text-xs font-medium uppercase tracking-wide text-muted-foreground">
        {title}
      </h2>
      <div className="divide-y divide-border rounded-card border border-border bg-surface">
        {children}
      </div>
    </section>
  );
}

/** One row in a [`SettingsSection`]: a label (+ optional description) on the left, a control on the
 * right. Pass the control via `control`; falls back to `children`. */
export function SettingsRow({
  label,
  description,
  control,
  children,
}: {
  label: string;
  description?: ReactNode;
  control?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-x-6 gap-y-2 px-4 py-3">
      <div className="min-w-0">
        <div className="text-sm font-medium">{label}</div>
        {description && <div className="mt-0.5 text-xs text-muted-foreground">{description}</div>}
      </div>
      <div className="shrink-0 text-sm text-muted-foreground">{control ?? children}</div>
    </div>
  );
}
