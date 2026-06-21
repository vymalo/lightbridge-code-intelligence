"use client";

import { ChevronDown, ChevronUp } from "lucide-react";
import { useRouter } from "next/navigation";
import { useMemo, useState } from "react";
import { StatusPill } from "@/components/ui/status-pill";
import { cn } from "@/lib/cn";
import {
  duration,
  durationSeconds,
  relativeTime,
  repoLabel,
  shortSha,
  statusVisual,
  type Task,
  triggerLabel,
} from "@/lib/tasks";

type SortKey = "created" | "duration" | "status" | "repo" | "trigger";
type SortDir = "asc" | "desc";

const PAGE_SIZE = 25;

// Ascending comparators per column; the header toggles direction. `created` is the default (desc =
// newest first), matching the list's natural order.
function compare(key: SortKey, a: Task, b: Task, now: number): number {
  switch (key) {
    case "created":
      return new Date(a.created_at).getTime() - new Date(b.created_at).getTime();
    case "duration":
      // Unstarted runs (null) sort below started ones in ascending order.
      return (durationSeconds(a, now) ?? -1) - (durationSeconds(b, now) ?? -1);
    case "status":
      return statusVisual(a.status).label.localeCompare(statusVisual(b.status).label);
    case "repo":
      return repoLabel(a).localeCompare(repoLabel(b));
    case "trigger":
      return triggerLabel(a).localeCompare(triggerLabel(b));
  }
}

/** Dense, sortable, paginated table of runs (ADR-0024, Appwrite table pattern). Sort + page state is
 * local; `now` comes from the server so relative times don't drift on hydration. */
export function RunTable({ tasks, now }: { tasks: Task[]; now: number }) {
  const router = useRouter();
  const [sort, setSort] = useState<{ key: SortKey; dir: SortDir }>({ key: "created", dir: "desc" });
  const [page, setPage] = useState(0);

  const sorted = useMemo(() => {
    const out = [...tasks].sort((a, b) => compare(sort.key, a, b, now));
    return sort.dir === "desc" ? out.reverse() : out;
  }, [tasks, sort, now]);

  // Clamp the page when the filtered set shrinks beneath the current offset.
  const pageCount = Math.max(1, Math.ceil(sorted.length / PAGE_SIZE));
  const current = Math.min(page, pageCount - 1);
  const start = current * PAGE_SIZE;
  const rows = sorted.slice(start, start + PAGE_SIZE);

  const toggle = (key: SortKey) =>
    setSort((s) =>
      s.key === key ? { key, dir: s.dir === "asc" ? "desc" : "asc" } : { key, dir: "asc" },
    );

  return (
    <div>
      <div className="overflow-x-auto">
        <table className="w-full border-collapse text-sm">
          <thead>
            <tr className="border-b border-border text-left text-xs text-muted-foreground">
              <Th label="Status" sortKey="status" sort={sort} onSort={toggle} />
              <Th label="Trigger" sortKey="trigger" sort={sort} onSort={toggle} />
              <Th label="Repository" sortKey="repo" sort={sort} onSort={toggle} />
              <th className="px-4 py-2 font-medium">Branch</th>
              <Th label="Created" sortKey="created" sort={sort} onSort={toggle} align="right" />
              <Th label="Duration" sortKey="duration" sort={sort} onSort={toggle} align="right" />
            </tr>
          </thead>
          <tbody>
            {rows.map((task) => {
              const dur = duration(task, now);
              const sha = shortSha(task.head_sha);
              return (
                <tr
                  key={task.id}
                  onClick={() => router.push(`/dashboard/runs/${task.id}`)}
                  className="cursor-pointer border-b border-border transition-colors last:border-0 hover:bg-muted/60"
                >
                  <td className="px-4 py-2.5">
                    <StatusPill status={task.status} />
                  </td>
                  <td className="max-w-xs truncate px-4 py-2.5 font-medium">
                    {/* A real link keeps the row keyboard-accessible; the row onClick is mouse sugar. */}
                    <a
                      href={`/dashboard/runs/${task.id}`}
                      onClick={(e) => e.stopPropagation()}
                      className="hover:underline"
                    >
                      {triggerLabel(task)}
                    </a>
                  </td>
                  <td className="max-w-[12rem] truncate px-4 py-2.5 text-muted-foreground">
                    {repoLabel(task)}
                  </td>
                  <td className="px-4 py-2.5 text-muted-foreground">
                    {task.repo_default_branch ??
                      (sha ? <span className="font-mono">{sha}</span> : "—")}
                  </td>
                  <td
                    className="whitespace-nowrap px-4 py-2.5 text-right text-muted-foreground"
                    title={task.created_at}
                  >
                    {relativeTime(task.created_at, now)}
                  </td>
                  <td className="px-4 py-2.5 text-right font-mono text-muted-foreground">
                    {dur ?? "—"}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      <div className="flex items-center justify-between gap-3 border-t border-border px-4 py-2.5 text-xs text-muted-foreground">
        <span>
          {sorted.length === 0
            ? "No runs"
            : `${start + 1}–${Math.min(start + PAGE_SIZE, sorted.length)} of ${sorted.length}`}
        </span>
        <div className="flex items-center gap-1">
          <PageButton disabled={current <= 0} onClick={() => setPage(current - 1)}>
            Prev
          </PageButton>
          <span className="px-1 tabular-nums">
            {current + 1} / {pageCount}
          </span>
          <PageButton disabled={current >= pageCount - 1} onClick={() => setPage(current + 1)}>
            Next
          </PageButton>
        </div>
      </div>
    </div>
  );
}

function Th({
  label,
  sortKey,
  sort,
  onSort,
  align = "left",
}: {
  label: string;
  sortKey: SortKey;
  sort: { key: SortKey; dir: SortDir };
  onSort: (key: SortKey) => void;
  align?: "left" | "right";
}) {
  const active = sort.key === sortKey;
  return (
    <th className={cn("px-4 py-2 font-medium", align === "right" && "text-right")}>
      <button
        type="button"
        onClick={() => onSort(sortKey)}
        className={cn(
          "inline-flex items-center gap-1 transition-colors hover:text-foreground",
          align === "right" && "flex-row-reverse",
          active && "text-foreground",
        )}
      >
        {label}
        {active &&
          (sort.dir === "asc" ? (
            <ChevronUp className="size-3" />
          ) : (
            <ChevronDown className="size-3" />
          ))}
      </button>
    </th>
  );
}

function PageButton({
  disabled,
  onClick,
  children,
}: {
  disabled: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className="rounded-md border border-border px-2 py-1 transition-colors hover:bg-muted disabled:cursor-not-allowed disabled:opacity-40"
    >
      {children}
    </button>
  );
}
