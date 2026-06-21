import { GitPullRequest, LayoutGrid, ListChecks, Settings, ShieldCheck } from "lucide-react";
import Link from "next/link";
import type { ReactNode } from "react";
import { NavLink } from "@/components/nav-link";

/** The console chrome (ADR-0016): a left sidebar nav + a slim top bar around contained content.
 * `admin` reveals the approval screen's nav entry (Epic #75) — the control plane still enforces it. */
export function ConsoleShell({
  user,
  admin,
  children,
}: {
  user: string;
  admin?: boolean;
  children: ReactNode;
}) {
  return (
    <div className="grid min-h-dvh grid-cols-[15rem_1fr] max-md:grid-cols-1">
      <aside className="flex flex-col gap-6 border-r border-border bg-surface px-3 py-4 max-md:hidden">
        <Link href="/dashboard" className="flex items-center gap-2 px-2.5">
          <span className="flex size-6 items-center justify-center rounded-md bg-accent text-accent-foreground text-xs font-semibold">
            L
          </span>
          <span className="text-sm font-medium tracking-tight">Lightbridge</span>
        </Link>
        <nav className="flex flex-col gap-0.5">
          <NavLink href="/dashboard" exact icon={<LayoutGrid className="size-4" />}>
            Overview
          </NavLink>
          <NavLink href="/dashboard/runs" icon={<ListChecks className="size-4" />}>
            Runs
          </NavLink>
          <NavLink href="/dashboard/repositories" icon={<GitPullRequest className="size-4" />}>
            Repositories
          </NavLink>
          {admin && (
            <NavLink href="/dashboard/admin" icon={<ShieldCheck className="size-4" />}>
              Approvals
            </NavLink>
          )}
          <NavLink href="/dashboard/settings" icon={<Settings className="size-4" />}>
            Settings
          </NavLink>
        </nav>
      </aside>

      <div className="flex min-w-0 flex-col">
        <header className="flex h-12 items-center justify-between gap-4 border-b border-border bg-surface px-4">
          <span className="text-sm font-medium tracking-tight md:hidden">Lightbridge</span>
          <div className="flex flex-1 items-center justify-end gap-3 text-sm text-muted-foreground">
            <span className="truncate" title={user}>
              {user}
            </span>
            <a
              href="/api/auth/logout"
              className="rounded-md border border-border px-2.5 py-1 text-xs text-foreground transition-colors hover:bg-muted"
            >
              Sign out
            </a>
          </div>
        </header>
        <main className="mx-auto w-full max-w-5xl flex-1 px-4 py-6 md:px-6 md:py-8">
          {children}
        </main>
      </div>
    </div>
  );
}
