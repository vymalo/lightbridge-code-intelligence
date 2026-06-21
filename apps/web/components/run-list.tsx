"use client";

import { LayoutList, Search, Table2 } from "lucide-react";
import { useMemo, useState } from "react";
import { RunTable } from "@/components/run-table";
import { RunTimeline } from "@/components/run-timeline";
import { StatusLine } from "@/components/states";
import { cn } from "@/lib/cn";
import { repoLabel, type StatusVariant, statusVisual, type Task, triggerLabel } from "@/lib/tasks";

const FILTERS: { value: StatusVariant | "all"; label: string }[] = [
  { value: "all", label: "All" },
  { value: "active", label: "Running" },
  { value: "pending", label: "Pending" },
  { value: "success", label: "Succeeded" },
  { value: "error", label: "Failed" },
  { value: "muted", label: "Cancelled" },
];

type View = "timeline" | "table";

/** Run list with status + repo filters, text search, and a timeline/table view toggle (ADR-0024) —
 * all client-side over the fetched page. `now` is passed from the server so relative times don't
 * cause hydration drift. */
export function RunList({ tasks, now }: { tasks: Task[]; now: number }) {
  const [filter, setFilter] = useState<StatusVariant | "all">("all");
  const [repo, setRepo] = useState<string>("all");
  const [query, setQuery] = useState("");
  const [view, setView] = useState<View>("timeline");

  // Repos present in this page, for the repo filter dropdown.
  const repos = useMemo(
    () => Array.from(new Set(tasks.map(repoLabel))).sort((a, b) => a.localeCompare(b)),
    [tasks],
  );

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return tasks.filter((t) => {
      if (filter !== "all" && statusVisual(t.status).variant !== filter) return false;
      if (repo !== "all" && repoLabel(t) !== repo) return false;
      if (!q) return true;
      return `${repoLabel(t)} ${triggerLabel(t)} ${t.head_sha ?? ""}`.toLowerCase().includes(q);
    });
  }, [tasks, filter, repo, query]);

  return (
    <div className="overflow-hidden rounded-card border border-border bg-surface">
      <div className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-2.5">
        <div className="flex flex-wrap gap-1">
          {FILTERS.map((f) => (
            <button
              type="button"
              key={f.value}
              onClick={() => setFilter(f.value)}
              className={cn(
                "rounded-full px-2.5 py-1 text-xs font-medium transition-colors",
                filter === f.value
                  ? "bg-foreground text-background"
                  : "text-muted-foreground hover:bg-muted",
              )}
            >
              {f.label}
            </button>
          ))}
        </div>

        <div className="ml-auto flex items-center gap-2">
          {repos.length > 1 && (
            <select
              value={repo}
              onChange={(e) => setRepo(e.target.value)}
              aria-label="Filter by repository"
              className="h-7 max-w-[12rem] rounded-md border border-border bg-background px-2 text-xs text-foreground outline-none focus:border-border-strong focus:ring-1 focus:ring-ring"
            >
              <option value="all">All repositories</option>
              {repos.map((r) => (
                <option key={r} value={r}>
                  {r}
                </option>
              ))}
            </select>
          )}

          <div className="relative">
            <Search className="pointer-events-none absolute left-2 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
            <input
              type="search"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search runs"
              className="h-7 w-44 rounded-md border border-border bg-background pl-7 pr-2 text-xs outline-none placeholder:text-muted-foreground focus:border-border-strong focus:ring-1 focus:ring-ring"
            />
          </div>

          {/* Timeline / table view toggle. */}
          <div className="flex items-center rounded-md border border-border p-0.5">
            <ViewButton
              active={view === "timeline"}
              onClick={() => setView("timeline")}
              label="Timeline"
            >
              <LayoutList className="size-3.5" />
            </ViewButton>
            <ViewButton active={view === "table"} onClick={() => setView("table")} label="Table">
              <Table2 className="size-3.5" />
            </ViewButton>
          </div>
        </div>
      </div>

      {filtered.length === 0 ? (
        <StatusLine>No runs match the current filters.</StatusLine>
      ) : view === "timeline" ? (
        <RunTimeline tasks={filtered} now={now} />
      ) : (
        <RunTable tasks={filtered} now={now} />
      )}
    </div>
  );
}

function ViewButton({
  active,
  onClick,
  label,
  children,
}: {
  active: boolean;
  onClick: () => void;
  label: string;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      aria-label={`${label} view`}
      title={`${label} view`}
      className={cn(
        "rounded p-1 transition-colors",
        active ? "bg-muted text-foreground" : "text-muted-foreground hover:text-foreground",
      )}
    >
      {children}
    </button>
  );
}
