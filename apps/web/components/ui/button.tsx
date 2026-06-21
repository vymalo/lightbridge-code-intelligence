import { cva, type VariantProps } from "class-variance-authority";
import type { ButtonHTMLAttributes } from "react";
import { cn } from "@/lib/cn";

/** daisyUI `btn` variants (ADR-0027). `primary` = the dracula accent; `neutral` = a plain surface
 * button; `ghost` = borderless, for low-emphasis inline actions; `outline` = hairline. */
const buttonVariants = cva("btn", {
  variants: {
    variant: {
      primary: "btn-primary",
      neutral: "",
      ghost: "btn-ghost",
      outline: "btn-outline",
    },
    size: {
      xs: "btn-xs",
      sm: "btn-sm",
      md: "btn-md",
    },
  },
  defaultVariants: { variant: "neutral", size: "sm" },
});

export type ButtonVariants = VariantProps<typeof buttonVariants>;

/** Compose the daisyUI `btn` classes for a variant/size. Use this for `<a>`/`<Link>` styled as
 * buttons; use [`Button`] for real `<button>`s. */
export function buttonClass(
  variant?: ButtonVariants["variant"],
  size?: ButtonVariants["size"],
): string {
  return cn(buttonVariants({ variant, size }));
}

/** A daisyUI `btn`. Forwards every native button prop (`type`, `disabled`, `formAction`, …) so it
 * drops into `<form action={…}>` server-action submits unchanged. */
export function Button({
  variant,
  size,
  className,
  type = "button",
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & ButtonVariants) {
  return (
    <button type={type} className={cn(buttonVariants({ variant, size }), className)} {...props} />
  );
}
