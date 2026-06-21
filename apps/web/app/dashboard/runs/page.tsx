import { RunList } from "@/components/run-list";
import { ApiErrorLine, EmptyState } from "@/components/states";
import { Card } from "@/components/ui/card";
import { listTasks } from "@/lib/api";

export const dynamic = "force-dynamic";

export default async function Runs({
  searchParams,
}: {
  searchParams: Promise<{ status?: string }>;
}) {
  const { status } = await searchParams;
  const result = await listTasks();
  const now = Date.now();

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-lg font-medium tracking-tight">Runs</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Every task run, most recent first. Select a run to see its output and logs.
        </p>
      </div>

      {!result.ok ? (
        <Card>
          <ApiErrorLine result={result} />
        </Card>
      ) : result.data.length === 0 ? (
        <EmptyState title="No task runs yet">
          Runs appear here when the GitHub App processes a pull request or comment command.
        </EmptyState>
      ) : (
        <RunList tasks={result.data} now={now} initialStatus={status} />
      )}
    </div>
  );
}
