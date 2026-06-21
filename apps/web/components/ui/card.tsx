import type { ReactNode } from "react";
import { cn } from "@/lib/cn";

/** Flat surface card on the daisyUI `card` primitive (ADR-0027): hairline border (`card-border`),
 * no shadow, base-200 surface. Header/body keep a dense, hairline-divided rhythm. */
export function Card({ className, children }: { className?: string; children: ReactNode }) {
  return <div className={cn("card card-border bg-base-200", className)}>{children}</div>;
}

export function CardHeader({ className, children }: { className?: string; children: ReactNode }) {
  return <div className={cn("border-b border-border px-4 py-3", className)}>{children}</div>;
}

export function CardTitle({ className, children }: { className?: string; children: ReactNode }) {
  return <h2 className={cn("text-sm font-medium tracking-tight", className)}>{children}</h2>;
}

export function CardBody({ className, children }: { className?: string; children: ReactNode }) {
  return <div className={cn("px-4 py-3", className)}>{children}</div>;
}
