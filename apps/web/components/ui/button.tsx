import type { ButtonHTMLAttributes } from "react";
import { cn } from "@/lib/cn";

/** daisyUI `btn` variants we use (ADR-0027). `primary` = the dracula accent; `neutral` = a plain
 * surface button; `ghost` = borderless, for low-emphasis inline actions; `outline` = hairline. */
export type ButtonVariant = "primary" | "neutral" | "ghost" | "outline";
export type ButtonSize = "xs" | "sm" | "md";

const VARIANT: Record<ButtonVariant, string> = {
  primary: "btn-primary",
  neutral: "",
  ghost: "btn-ghost",
  outline: "btn-outline",
};

/** Compose the daisyUI `btn` classes for a variant/size. Use this for `<a>`/`<Link>` styled as
 * buttons; use [`Button`] for real `<button>`s. */
export function buttonClass(variant: ButtonVariant = "neutral", size: ButtonSize = "sm"): string {
  return cn("btn", `btn-${size}`, VARIANT[variant]);
}

/** A daisyUI `btn`. Forwards every native button prop (`type`, `disabled`, `formAction`, …) so it
 * drops into `<form action={…}>` server-action submits unchanged. */
export function Button({
  variant = "neutral",
  size = "sm",
  className,
  type = "button",
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: ButtonVariant;
  size?: ButtonSize;
}) {
  return <button type={type} className={cn(buttonClass(variant, size), className)} {...props} />;
}
