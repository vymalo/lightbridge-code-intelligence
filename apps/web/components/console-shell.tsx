import { GitPullRequest, LayoutGrid, ListChecks, Settings, ShieldCheck } from "lucide-react";
import Link from "next/link";
import type { ReactNode } from "react";
import { CommandPalette } from "@/components/command-palette";
import { NavLink } from "@/components/nav-link";
import { buttonClass } from "@/components/ui/button";
import { githubAppInstallUrl } from "@/lib/config";

/** The console chrome (ADR-0016; grouped nav + ⌘K palette in ADR-0024): a left sidebar nav split
 * into hairline-separated groups + a slim top bar with a command palette, around contained content.
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
      <aside className="flex flex-col gap-6 border-r border-base-content/15 bg-base-200 px-3 py-4 max-md:hidden">
        <Link href="/dashboard" className="flex items-center gap-2 px-2.5">
          <span className="flex size-6 items-center justify-center rounded-md bg-primary text-primary-content text-xs font-semibold">
            L
          </span>
          <span className="text-sm font-medium tracking-tight">Lightbridge</span>
        </Link>
        <nav className="flex flex-col gap-3">
          <div className="flex flex-col gap-0.5">
            <NavLink href="/dashboard" exact icon={<LayoutGrid className="size-4" />}>
              Overview
            </NavLink>
            <NavLink href="/dashboard/runs" icon={<ListChecks className="size-4" />}>
              Runs
            </NavLink>
            <NavLink href="/dashboard/repositories" icon={<GitPullRequest className="size-4" />}>
              Repositories
            </NavLink>
          </div>
          {/* Hairline separator between the primary group and the system/admin group. */}
          <hr className="mx-2.5 border-base-content/15" />
          <div className="flex flex-col gap-0.5">
            {admin && (
              <NavLink href="/dashboard/admin" icon={<ShieldCheck className="size-4" />}>
                Approvals
              </NavLink>
            )}
            <NavLink href="/dashboard/settings" icon={<Settings className="size-4" />}>
              Settings
            </NavLink>
          </div>
        </nav>
      </aside>

      <div className="flex min-w-0 flex-col">
        <header className="flex h-12 items-center gap-4 border-b border-base-content/15 bg-base-200 px-4">
          <span className="text-sm font-medium tracking-tight md:hidden">Lightbridge</span>
          <CommandPalette admin={Boolean(admin)} githubAppUrl={githubAppInstallUrl()} />
          <div className="ml-auto flex items-center gap-3 text-sm text-base-content/60">
            <span className="truncate" title={user}>
              {user}
            </span>
            <a href="/api/auth/logout" className={buttonClass("ghost", "xs")}>
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
