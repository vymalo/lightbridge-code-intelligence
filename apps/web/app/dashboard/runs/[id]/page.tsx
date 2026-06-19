import { ArrowLeft } from "lucide-react";
import Link from "next/link";
import { notFound } from "next/navigation";
import { ApiErrorLine, StatusLine } from "@/components/states";
import { Card, CardBody, CardHeader, CardTitle } from "@/components/ui/card";
import { StatusPill } from "@/components/ui/status-pill";
import { getTask } from "@/lib/api";
import {
  absoluteTime,
  duration,
  repoLabel,
  shortSha,
  statusVisual,
  triggerLabel,
} from "@/lib/tasks";

export const dynamic = "force-dynamic";

export default async function RunDetail({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  const result = await getTask(id);

  if (!result.ok) {
    return (
      <Shell>
        <Card>
          <ApiErrorLine result={result} />
        </Card>
      </Shell>
    );
  }
  if (!result.data) notFound();

  const task = result.data;
  const now = Date.now();
  const isError = statusVisual(task.status).variant === "error";

  return (
    <Shell>
      <div className="flex flex-wrap items-center gap-3">
        <StatusPill status={task.status} />
        <h1 className="text-lg font-medium tracking-tight">{triggerLabel(task)}</h1>
      </div>

      {isError && (
        <Card className="border-[var(--status-error)]">
          <CardBody>
            <StatusLine tone="error">
              This run ended in a {statusVisual(task.status).label.toLowerCase()} state. Detailed
              error output will appear here once the agent runner reports results.
            </StatusLine>
          </CardBody>
        </Card>
      )}

      <Card>
        <CardHeader>
          <CardTitle>Overview</CardTitle>
        </CardHeader>
        <CardBody>
          <dl className="grid grid-cols-1 gap-x-8 gap-y-3 sm:grid-cols-2">
            <Field label="Repository" value={repoLabel(task)} />
            <Field label="Branch" value={task.repo_default_branch ?? "—"} />
            <Field label="Trigger" value={triggerLabel(task)} />
            <Field label="Delivery" value={task.github_delivery_id} mono />
            <Field label="Base SHA" value={shortSha(task.base_sha) ?? "—"} mono />
            <Field label="Head SHA" value={shortSha(task.head_sha) ?? "—"} mono />
            <Field label="Created" value={absoluteTime(task.created_at)} />
            <Field label="Started" value={task.started_at ? absoluteTime(task.started_at) : "—"} />
            <Field
              label="Completed"
              value={task.completed_at ? absoluteTime(task.completed_at) : "—"}
            />
            <Field label="Duration" value={duration(task, now) ?? "—"} mono />
          </dl>
        </CardBody>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Review output</CardTitle>
        </CardHeader>
        <StatusLine>
          Structured review findings will appear here once the indexer and agent runner are wired
          (Code product epic). This run is currently {statusVisual(task.status).label.toLowerCase()}
          .
        </StatusLine>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Logs</CardTitle>
        </CardHeader>
        <StatusLine>Run logs will stream here once the dispatcher executes the task.</StatusLine>
      </Card>
    </Shell>
  );
}

function Shell({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-5">
      <Link
        href="/dashboard/runs"
        className="inline-flex w-fit items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground"
      >
        <ArrowLeft className="size-4" />
        Runs
      </Link>
      {children}
    </div>
  );
}

function Field({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div>
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className={`mt-0.5 break-all text-sm ${mono ? "font-mono" : ""}`}>{value}</dd>
    </div>
  );
}
