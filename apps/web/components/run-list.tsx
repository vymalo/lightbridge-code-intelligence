"use client";

import { Search } from "lucide-react";
import { useMemo, useState } from "react";
import { RunRow } from "@/components/run-row";
import { StatusLine } from "@/components/states";
import { repoLabel, type StatusVariant, statusVisual, type Task, triggerLabel } from "@/lib/tasks";

const FILTERS: { value: StatusVariant | "all"; label: string }[] = [
  { value: "all", label: "All" },
  { value: "active", label: "Running" },
  { value: "pending", label: "Pending" },
  { value: "success", label: "Succeeded" },
  { value: "error", label: "Failed" },
  { value: "muted", label: "Cancelled" },
];

/** Run list with a status filter + text search (client-side over the fetched page). `now` is passed
 *  from the server so relative times don't cause hydration drift. */
export function RunList({ tasks, now }: { tasks: Task[]; now: number }) {
  const [filter, setFilter] = useState<StatusVariant | "all">("all");
  const [query, setQuery] = useState("");

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return tasks.filter((t) => {
      if (filter !== "all" && statusVisual(t.status).variant !== filter) return false;
      if (!q) return true;
      return `${repoLabel(t)} ${triggerLabel(t)} ${t.head_sha ?? ""}`.toLowerCase().includes(q);
    });
  }, [tasks, filter, query]);

  return (
    <div className="overflow-hidden rounded-card border border-border bg-surface">
      <div className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-2.5">
        <div className="flex flex-wrap gap-1">
          {FILTERS.map((f) => (
            <button
              type="button"
              key={f.value}
              onClick={() => setFilter(f.value)}
              className={`rounded-full px-2.5 py-1 text-xs font-medium transition-colors ${
                filter === f.value
                  ? "bg-foreground text-background"
                  : "text-muted-foreground hover:bg-muted"
              }`}
            >
              {f.label}
            </button>
          ))}
        </div>
        <div className="relative ml-auto">
          <Search className="pointer-events-none absolute left-2 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <input
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search runs"
            className="h-7 w-44 rounded-md border border-border bg-background pl-7 pr-2 text-xs outline-none placeholder:text-muted-foreground focus:border-border-strong focus:ring-1 focus:ring-ring"
          />
        </div>
      </div>

      {filtered.length === 0 ? (
        <StatusLine>No runs match the current filter.</StatusLine>
      ) : (
        <div className="divide-y divide-border">
          {filtered.map((task) => (
            <RunRow key={task.id} task={task} now={now} />
          ))}
        </div>
      )}
    </div>
  );
}
