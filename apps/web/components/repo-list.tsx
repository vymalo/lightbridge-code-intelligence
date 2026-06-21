"use client";

import { ExternalLink, GitBranch, Search } from "lucide-react";
import { parseAsInteger, useQueryState } from "nuqs";
import { useMemo } from "react";
import { Card } from "@/components/ui/card";
import { approvalVisual, type Repository, repoSlug, repoUrl } from "@/lib/repos";
import { relativeTime } from "@/lib/tasks";

const PAGE_SIZE = 12;

/** Connected repositories as cards with a search box + pagination (ADR-0024). Search + page live in
 * the URL via nuqs; filtering/paging is client-side over the fetched list. `now` is server-passed so
 * relative times don't drift on hydration. */
export function RepoList({ repos, now }: { repos: Repository[]; now: number }) {
  const [query, setQuery] = useQueryState("q", { defaultValue: "", clearOnDefault: true });
  const [page, setPage] = useQueryState("page", parseAsInteger.withDefault(0));

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return q ? repos.filter((r) => repoSlug(r).toLowerCase().includes(q)) : repos;
  }, [repos, query]);

  const pageCount = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
  const current = Math.min(page, pageCount - 1);
  const start = current * PAGE_SIZE;
  const shown = filtered.slice(start, start + PAGE_SIZE);

  return (
    <div className="flex flex-col gap-3">
      <div className="relative w-full sm:w-72">
        <Search className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
        <input
          type="search"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setPage(null);
          }}
          placeholder="Search repositories"
          className="h-9 w-full rounded-md border border-border bg-background pl-8 pr-3 text-sm outline-none placeholder:text-muted-foreground focus:border-border-strong focus:ring-1 focus:ring-ring"
        />
      </div>

      {shown.length === 0 ? (
        <p className="px-1 py-6 text-sm text-muted-foreground">No repositories match “{query}”.</p>
      ) : (
        <div className="grid gap-3 sm:grid-cols-2">
          {shown.map((repo) => (
            <RepoCard key={repo.id} repo={repo} now={now} />
          ))}
        </div>
      )}

      {filtered.length > PAGE_SIZE && (
        <div className="flex items-center justify-between gap-3 text-xs text-muted-foreground">
          <span>
            {start + 1}–{Math.min(start + PAGE_SIZE, filtered.length)} of {filtered.length}
          </span>
          <div className="flex items-center gap-1">
            <PageButton disabled={current <= 0} onClick={() => setPage(current - 1 || null)}>
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
      )}
    </div>
  );
}

function RepoCard({ repo, now }: { repo: Repository; now: number }) {
  const approval = approvalVisual(repo);
  return (
    <Card className="flex flex-col">
      <div className="flex items-start justify-between gap-3 px-4 py-3">
        <div className="min-w-0">
          <div className="truncate text-sm font-medium">{repoSlug(repo)}</div>
          <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs text-muted-foreground">
            <span className="inline-flex items-center gap-1">
              <GitBranch className="size-3" />
              {repo.default_branch}
            </span>
            <span>
              {repo.task_count} {repo.task_count === 1 ? "run" : "runs"}
            </span>
            {repo.last_task_at && <span>last {relativeTime(repo.last_task_at, now)}</span>}
          </div>
        </div>
        <span className={`status-pill status-${approval.variant} shrink-0`}>{approval.label}</span>
      </div>
      <div className="flex items-center justify-between gap-3 border-t border-border px-4 py-2 text-xs">
        {/* Index health (graph + vector freshness, ADR-0016) lands with the indexer — honest for now. */}
        <span className="text-muted-foreground">Not indexed yet</span>
        <a
          href={repoUrl(repo)}
          target="_blank"
          rel="noopener noreferrer"
          className="inline-flex items-center gap-1 text-accent transition-colors hover:underline"
        >
          View on GitHub
          <ExternalLink className="size-3 shrink-0" />
        </a>
      </div>
    </Card>
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
