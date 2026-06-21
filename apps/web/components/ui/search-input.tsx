import { Search } from "lucide-react";
import type { InputHTMLAttributes } from "react";
import { cn } from "@/lib/cn";

/** A search box on the daisyUI `input` primitive (ADR-0027): the `input` class styles the wrapping
 * label, the leading magnifier sits inside it, and the real `<input>` is borderless within. Sizing /
 * width come from `className` on the wrapper (e.g. `input-xs w-44`). Forwards native input props. */
export function SearchInput({
  className,
  ...props
}: InputHTMLAttributes<HTMLInputElement> & { className?: string }) {
  return (
    <label className={cn("input input-sm", className)}>
      <Search className="size-3.5 shrink-0 opacity-60" />
      <input type="search" {...props} />
    </label>
  );
}
