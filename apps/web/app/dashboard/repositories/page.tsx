import { RepoList } from "@/components/repo-list";
import { ApiErrorLine, EmptyState } from "@/components/states";
import { Card } from "@/components/ui/card";
import { listRepositories } from "@/lib/api";
import { githubAppInstallUrl } from "@/lib/config";

export const dynamic = "force-dynamic";

export default async function Repositories() {
  const result = await listRepositories();
  const now = Date.now();

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-lg font-medium tracking-tight">Repositories</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Repositories the GitHub App is connected to, with their approval and run activity.
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
        <RepoList repos={result.data} now={now} />
      )}
    </div>
  );
}
