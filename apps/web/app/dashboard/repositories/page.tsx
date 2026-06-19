import { GitBranch } from "lucide-react";
import { ApiErrorLine, EmptyState, StatusLine } from "@/components/states";
import { Card } from "@/components/ui/card";
import { listRepositories } from "@/lib/api";
import { githubAppInstallUrl } from "@/lib/config";
import { repoSlug } from "@/lib/repos";
import { relativeTime } from "@/lib/tasks";

export const dynamic = "force-dynamic";

export default async function Repositories() {
  const result = await listRepositories();
  const now = Date.now();

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-lg font-medium tracking-tight">Repositories</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Repositories the GitHub App is connected to, with their run activity.
        </p>
      </div>

      {!result.ok ? (
        <Card>
          <ApiErrorLine result={result} />
        </Card>
      ) : result.data.length === 0 ? (
        <EmptyState
          title="No repositories yet"
          action={
            <a
              className="inline-flex items-center rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-accent-foreground transition-opacity hover:opacity-90"
              href={githubAppInstallUrl()}
              target="_blank"
              rel="noreferrer"
            >
              Install the GitHub App
            </a>
          }
        >
          A repository appears here once the GitHub App processes an event on it (e.g. opening a
          pull request).
        </EmptyState>
      ) : (
        <Card className="overflow-hidden">
          <div className="divide-y divide-border">
            {result.data.map((repo) => (
              <div key={repo.id} className="flex items-center gap-3 px-4 py-3">
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-medium">{repoSlug(repo)}</div>
                  <div className="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs text-muted-foreground">
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
                {/* Index status is honest: the indexer that fills repo_index is a later step. */}
                <span className="status-pill status-pending shrink-0">Not indexed</span>
              </div>
            ))}
          </div>
          <StatusLine>
            Index health (graph + vector freshness, ADR-0016) appears here once the indexer lands.
          </StatusLine>
        </Card>
      )}
    </div>
  );
}
