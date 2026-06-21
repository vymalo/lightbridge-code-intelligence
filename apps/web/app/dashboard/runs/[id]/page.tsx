import { ArrowLeft, ExternalLink } from "lucide-react";
import Link from "next/link";
import { notFound } from "next/navigation";
import { CommandSnippet } from "@/components/command-snippet";
import { ReviewOutput } from "@/components/review-output";
import { RunLogs } from "@/components/run-logs";
import { ApiErrorLine, StatusLine } from "@/components/states";
import { Card, CardBody, CardHeader, CardTitle } from "@/components/ui/card";
import { StatusPill } from "@/components/ui/status-pill";
import { getReview, getTask } from "@/lib/api";
import {
  absoluteTime,
  duration,
  repoLabel,
  repoUrl,
  shortSha,
  statusVisual,
  targetUrl,
  triggerLabel,
} from "@/lib/tasks";

export const dynamic = "force-dynamic";

// Task ids are UUIDs; reject malformed paths up front so we don't round-trip to the control plane.
const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** Agents namespace the runner Jobs live in (mirrors the control plane's AGENT_NAMESPACE). */
function agentsNamespace(): string {
  return process.env.AGENT_NAMESPACE?.trim() || "lightbridge-agents";
}

export default async function RunDetail({ params }: { params: Promise<{ id: string }> }) {
  const { id } = await params;
  if (!UUID_RE.test(id)) notFound();
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
  // The persisted review (if any). 404 → null (older run / index task / never posted).
  const reviewResult = await getReview(id);
  const review = reviewResult.ok ? reviewResult.data : null;

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
            <Field label="Repository" value={repoLabel(task)} href={repoUrl(task)} />
            <Field label="Branch" value={task.repo_default_branch ?? "—"} />
            <Field label="Trigger" value={triggerLabel(task)} href={targetUrl(task)} />
            <Field label="Delivery" value={task.github_delivery_id ?? "—"} mono />
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
        {!reviewResult.ok ? (
          <CardBody>
            <ApiErrorLine result={reviewResult} />
          </CardBody>
        ) : review ? (
          <CardBody>
            <ReviewOutput review={review} />
          </CardBody>
        ) : (
          <StatusLine>
            No review was recorded for this run — it may be an indexing run, a run that hasn't
            posted yet, or one from before review history was captured.
          </StatusLine>
        )}
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Logs</CardTitle>
        </CardHeader>
        {task.job_name ? (
          <CardBody className="flex flex-col gap-3">
            <RunLogs taskId={task.id} />
            <CommandSnippet
              label="Or stream from your terminal:"
              command={`kubectl -n ${agentsNamespace()} logs -f -l batch.kubernetes.io/job-name=${task.job_name}`}
            />
          </CardBody>
        ) : (
          <StatusLine>Run logs will stream here once the dispatcher executes the task.</StatusLine>
        )}
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

function Field({
  label,
  value,
  mono,
  href,
}: {
  label: string;
  value: string;
  mono?: boolean;
  href?: string | null;
}) {
  return (
    <div>
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className={`mt-0.5 break-all text-sm ${mono ? "font-mono" : ""}`}>
        {href ? (
          <a
            href={href}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-1 text-accent transition-colors hover:underline"
          >
            {value}
            <ExternalLink className="size-3 shrink-0" />
          </a>
        ) : (
          value
        )}
      </dd>
    </div>
  );
}
