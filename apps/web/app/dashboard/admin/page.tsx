import { Check, X } from "lucide-react";
import { ApiErrorLine, StatusLine } from "@/components/states";
import { Card, CardBody, CardHeader, CardTitle } from "@/components/ui/card";
import { isAdmin, listPendingRepos } from "@/lib/admin";
import type { Repository } from "@/lib/repos";
import { repoSlug } from "@/lib/repos";
import { currentClaims } from "@/lib/session";
import { approveRepoAction, denyRepoAction } from "./actions";

export const dynamic = "force-dynamic";

export default async function AdminPage() {
  const claims = await currentClaims();
  if (!isAdmin(claims)) {
    return (
      <div className="flex flex-col gap-5">
        <h1 className="text-lg font-medium tracking-tight">Repository approvals</h1>
        <Card>
          <StatusLine>
            You need the admin role to manage repository approvals. Ask an administrator to grant
            it.
          </StatusLine>
        </Card>
      </div>
    );
  }

  const result = await listPendingRepos();

  return (
    <div className="flex flex-col gap-5">
      <div>
        <h1 className="text-lg font-medium tracking-tight">Repository approvals</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Newly added repositories stay pending until approved — only then are they indexed or
          reviewed. Approving triggers an initial index; denying keeps the repo out of scope.
        </p>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Pending</CardTitle>
        </CardHeader>
        {!result.ok ? (
          <CardBody>
            <ApiErrorLine result={result} />
          </CardBody>
        ) : result.data.length === 0 ? (
          <StatusLine>No repositories are awaiting approval.</StatusLine>
        ) : (
          <ul className="divide-y divide-border">
            {result.data.map((repo) => (
              <PendingRow key={repo.id} repo={repo} />
            ))}
          </ul>
        )}
      </Card>
    </div>
  );
}

function PendingRow({ repo }: { repo: Repository }) {
  return (
    <li className="flex flex-wrap items-center justify-between gap-3 px-4 py-3">
      <div className="min-w-0">
        <div className="truncate text-sm font-medium">{repoSlug(repo)}</div>
        <div className="mt-0.5 font-mono text-xs text-muted-foreground">
          github id {repo.github_repo_id}
        </div>
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <form action={approveRepoAction}>
          <input type="hidden" name="id" value={repo.id} />
          <button
            type="submit"
            className="inline-flex items-center gap-1.5 rounded-md bg-accent px-2.5 py-1 text-xs font-medium text-accent-foreground transition-opacity hover:opacity-90"
          >
            <Check className="size-3.5" />
            Approve
          </button>
        </form>
        <form action={denyRepoAction}>
          <input type="hidden" name="id" value={repo.id} />
          <button
            type="submit"
            className="inline-flex items-center gap-1.5 rounded-md border border-border px-2.5 py-1 text-xs text-foreground transition-colors hover:bg-muted"
          >
            <X className="size-3.5" />
            Deny
          </button>
        </form>
      </div>
    </li>
  );
}
