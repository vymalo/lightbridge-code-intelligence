import Link from "next/link";
import { Insights } from "@/components/overview/insights";
import { RunRow } from "@/components/runs/run-row";
import { buttonClass } from "@/components/ui/button";
import { Card, CardHeader, CardTitle } from "@/components/ui/card";
import { ApiErrorLine, EmptyState } from "@/components/ui/states";
import { listTasks } from "@/lib/server/api";
import { githubAppInstallUrl } from "@/lib/utils/config";

// Task state changes server-side; always render fresh.
export const dynamic = "force-dynamic";

export default async function Overview() {
  const result = await listTasks();
  const now = Date.now();

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-lg font-medium tracking-tight">Overview</h1>
        <p className="mt-1 text-sm text-base-content/60">
          Task runs across your connected repositories.
        </p>
      </div>

      {!result.ok ? (
        <Card>
          <ApiErrorLine result={result} />
        </Card>
      ) : result.data.length === 0 ? (
        <EmptyState
          title="No task runs yet"
          action={
            <a
              className={buttonClass("primary")}
              href={githubAppInstallUrl()}
              target="_blank"
              rel="noreferrer"
            >
              Install the GitHub App
            </a>
          }
        >
          Once the Lightbridge GitHub App is installed on a repository, opening or updating a pull
          request will create a review run here.
        </EmptyState>
      ) : (
        <>
          <Insights tasks={result.data} now={now} />

          <Card>
            <CardHeader className="flex items-center justify-between">
              <CardTitle>Recent runs</CardTitle>
              <Link
                href="/dashboard/runs"
                className="text-xs text-base-content/60 underline-offset-2 hover:underline"
              >
                View all
              </Link>
            </CardHeader>
            <div className="divide-y divide-base-content/15">
              {result.data.slice(0, 8).map((task) => (
                <RunRow key={task.id} task={task} now={now} />
              ))}
            </div>
          </Card>
        </>
      )}
    </div>
  );
}
