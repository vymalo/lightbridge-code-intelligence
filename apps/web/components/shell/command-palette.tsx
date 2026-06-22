"use client";

import { Command } from "cmdk";
import {
  ExternalLink,
  GitPullRequest,
  LayoutGrid,
  ListChecks,
  LogOut,
  Search,
  Settings,
  ShieldCheck,
} from "lucide-react";
import { useRouter } from "next/navigation";
import type { ReactNode } from "react";
import { useEffect, useState } from "react";

// Task ids are UUIDs; when the query is one, offer a direct jump to that run.
const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** ⌘K command palette (ADR-0024, Raycast pattern): a top-bar search affordance + a dialog for
 * jump-to (nav destinations, a run by id) and quick actions. Headless cmdk styled with our tokens. */
export function CommandPalette({ admin, githubAppUrl }: { admin: boolean; githubAppUrl: string }) {
  const router = useRouter();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen((o) => !o);
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);

  const go = (href: string) => {
    setOpen(false);
    router.push(href);
  };

  const trimmed = query.trim();
  const runId = UUID_RE.test(trimmed) ? trimmed : null;

  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        aria-label="Search"
        className="btn btn-sm btn-ghost gap-2 font-normal text-base-content/60"
      >
        <Search className="size-3.5" />
        <span className="hidden sm:inline">Search</span>
        {/* Decorative; the button's aria-label is its accessible name, so this kbd isn't announced. */}
        <kbd className="kbd kbd-xs ml-1 hidden font-sans sm:inline">⌘K</kbd>
      </button>

      <Command.Dialog
        open={open}
        onOpenChange={setOpen}
        label="Command menu"
        overlayClassName="fixed inset-0 z-50 bg-black/40 backdrop-blur-sm"
        contentClassName="fixed left-1/2 top-[18%] z-50 w-[min(36rem,calc(100vw-2rem))] -translate-x-1/2 overflow-hidden rounded-box border border-base-content/15 bg-base-200"
      >
        <div className="flex items-center gap-2 border-b border-base-content/15 px-3">
          <Search className="size-4 shrink-0 text-base-content/60" />
          <Command.Input
            value={query}
            onValueChange={setQuery}
            placeholder="Search or jump to…"
            className="h-11 w-full bg-transparent text-sm outline-none placeholder:text-base-content/60"
          />
        </div>
        <Command.List className="max-h-80 overflow-y-auto p-1.5">
          <Command.Empty className="px-3 py-6 text-center text-sm text-base-content/60">
            No matches.
          </Command.Empty>

          {runId && (
            <Group heading="Jump to">
              {/* value = the raw query so this always matches what the user typed. */}
              <Item value={trimmed} onSelect={() => go(`/dashboard/runs/${runId}`)}>
                <ListChecks className="size-4" />
                Go to run {runId.slice(0, 8)}…
              </Item>
            </Group>
          )}

          <Group heading="Navigation">
            <Item value="overview dashboard" onSelect={() => go("/dashboard")}>
              <LayoutGrid className="size-4" />
              Overview
            </Item>
            <Item value="runs tasks" onSelect={() => go("/dashboard/runs")}>
              <ListChecks className="size-4" />
              Runs
            </Item>
            <Item value="repositories repos" onSelect={() => go("/dashboard/repositories")}>
              <GitPullRequest className="size-4" />
              Repositories
            </Item>
            {admin && (
              <Item value="approvals admin" onSelect={() => go("/dashboard/admin")}>
                <ShieldCheck className="size-4" />
                Approvals
              </Item>
            )}
            <Item value="settings" onSelect={() => go("/dashboard/settings")}>
              <Settings className="size-4" />
              Settings
            </Item>
          </Group>

          <Group heading="Actions">
            <Item
              value="open github app install"
              onSelect={() => {
                // Open synchronously first so the user-gesture context isn't lost (popup blocker).
                window.open(githubAppUrl, "_blank", "noopener,noreferrer");
                setOpen(false);
              }}
            >
              <ExternalLink className="size-4" />
              Open the GitHub App
            </Item>
            <Item
              value="sign out logout"
              onSelect={() => {
                window.location.href = "/api/auth/logout";
              }}
            >
              <LogOut className="size-4" />
              Sign out
            </Item>
          </Group>
        </Command.List>
      </Command.Dialog>
    </>
  );
}

function Group({ heading, children }: { heading: string; children: ReactNode }) {
  return (
    <Command.Group
      heading={heading}
      className="px-1 pb-1 text-xs text-base-content/60 [&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1.5 [&_[cmdk-group-heading]]:font-medium"
    >
      {children}
    </Command.Group>
  );
}

function Item({
  value,
  onSelect,
  children,
}: {
  value: string;
  onSelect: () => void;
  children: ReactNode;
}) {
  return (
    <Command.Item
      value={value}
      onSelect={onSelect}
      className="flex cursor-pointer items-center gap-2.5 rounded-md px-2 py-2 text-sm text-base-content data-[selected=true]:bg-base-300"
    >
      {children}
    </Command.Item>
  );
}
