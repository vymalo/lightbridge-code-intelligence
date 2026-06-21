"use client";

import { LayoutList, Table2 } from "lucide-react";
import { parseAsInteger, parseAsStringLiteral, useQueryState } from "nuqs";
import { useMemo } from "react";
import { RunTable } from "@/components/run-table";
import { RunTimeline } from "@/components/run-timeline";
import { StatusLine } from "@/components/states";
import { SearchInput } from "@/components/ui/search-input";
import { cn } from "@/lib/cn";
import { repoLabel, statusVisual, type Task, triggerLabel } from "@/lib/tasks";

const FILTER_VALUES = ["all", "active", "pending", "success", "error", "muted"] as const;
const FILTERS: { value: (typeof FILTER_VALUES)[number]; label: string }[] = [
  { value: "all", label: "All" },
  { value: "active", label: "Running" },
  { value: "pending", label: "Pending" },
  { value: "success", label: "Succeeded" },
  { value: "error", label: "Failed" },
  { value: "muted", label: "Cancelled" },
];

const VIEW_VALUES = ["timeline", "table"] as const;

/** Run list with status + repo filters, text search, and a timeline/table view toggle (ADR-0024,
 * daisyUI in ADR-0027). All state lives in the URL via nuqs (shareable/bookmarkable); `now` is
 * server-passed so relative times don't drift on hydration. Filtering is client-side over the page. */
export function RunList({ tasks, now }: { tasks: Task[]; now: number }) {
  const [filter, setFilter] = useQueryState(
    "status",
    parseAsStringLiteral(FILTER_VALUES).withDefault("all"),
  );
  const [repo, setRepo] = useQueryState("repo", { defaultValue: "all", clearOnDefault: true });
  const [query, setQuery] = useQueryState("q", { defaultValue: "", clearOnDefault: true });
  const [view, setView] = useQueryState(
    "view",
    parseAsStringLiteral(VIEW_VALUES).withDefault("timeline"),
  );
  const [page, setPage] = useQueryState("page", parseAsInteger.withDefault(0));

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

  // Any filter change invalidates the current page offset, so reset to the first page.
  const resetPage = () => setPage(null);

  return (
    <div className="overflow-hidden rounded-box border border-border bg-base-200">
      <div className="flex flex-wrap items-center gap-2 border-b border-border px-3 py-2.5">
        <div className="join">
          {FILTERS.map((f) => (
            <button
              type="button"
              key={f.value}
              onClick={() => {
                setFilter(f.value);
                resetPage();
              }}
              className={cn("btn btn-xs join-item", filter === f.value && "btn-active btn-primary")}
            >
              {f.label}
            </button>
          ))}
        </div>

        <div className="ml-auto flex items-center gap-2">
          {repos.length > 1 && (
            <select
              value={repo}
              onChange={(e) => {
                setRepo(e.target.value);
                resetPage();
              }}
              aria-label="Filter by repository"
              className="select select-sm max-w-[12rem]"
            >
              <option value="all">All repositories</option>
              {repos.map((r) => (
                <option key={r} value={r}>
                  {r}
                </option>
              ))}
            </select>
          )}

          <SearchInput
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              resetPage();
            }}
            placeholder="Search runs"
            aria-label="Search runs"
            className="w-44"
          />

          {/* Timeline / table view toggle. */}
          <div className="join">
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
        <RunTable tasks={filtered} now={now} page={page} onPageChange={setPage} />
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
      className={cn("btn btn-xs btn-square join-item", active && "btn-active")}
    >
      {children}
    </button>
  );
}
