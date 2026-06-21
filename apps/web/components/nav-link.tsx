"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import type { ReactNode } from "react";
import { cn } from "@/lib/cn";

/** Sidebar nav item — active when the current path matches (exactly, or as a prefix for sections). */
export function NavLink({
  href,
  exact,
  icon,
  children,
}: {
  href: string;
  exact?: boolean;
  icon: ReactNode;
  children: ReactNode;
}) {
  const pathname = usePathname();
  const active = exact ? pathname === href : pathname === href || pathname.startsWith(`${href}/`);
  return (
    <Link
      href={href}
      aria-current={active ? "page" : undefined}
      className={cn(
        "flex items-center gap-2.5 rounded-md px-2.5 py-1.5 text-sm transition-colors",
        active
          ? "bg-base-300 font-medium text-base-content"
          : "text-base-content/60 hover:bg-base-300 hover:text-base-content",
      )}
    >
      <span className="flex size-4 items-center justify-center text-current">{icon}</span>
      {children}
    </Link>
  );
}
