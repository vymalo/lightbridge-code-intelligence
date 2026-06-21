import { ExternalLink } from "lucide-react";
import type { Review, ReviewFinding } from "@/lib/tasks";

/** Severity → chip tone. Unknown severities fall back to a neutral chip. */
function severityClass(severity: string): string {
  switch (severity.toLowerCase()) {
    case "critical":
    case "error":
      return "bg-[var(--status-error)]/15 text-[var(--status-error)]";
    case "high":
    case "warning":
    case "warn":
      return "bg-amber-500/15 text-amber-600 dark:text-amber-400";
    case "medium":
    case "low":
    case "info":
      return "bg-blue-500/15 text-blue-600 dark:text-blue-400";
    default:
      return "bg-muted text-muted-foreground";
  }
}

/** The agent's persisted review for a run (Epic #75, Milestone C): summary, a count line, and each
 * finding with its severity, location, body, and optional suggested replacement. */
export function ReviewOutput({ review }: { review: Review }) {
  const counts = [
    `${review.inline_count} inline`,
    `${review.deferred_count} deferred`,
    review.out_of_scope_count > 0 ? `${review.out_of_scope_count} out of scope` : null,
  ].filter(Boolean);

  return (
    <div className="flex flex-col gap-4">
      {review.summary && <p className="text-sm leading-relaxed">{review.summary}</p>}
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-xs text-muted-foreground">{counts.join(" · ")}</p>
        {review.review_url && (
          <a
            href={review.review_url}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-1 text-xs text-accent transition-colors hover:underline"
          >
            View on GitHub
            <ExternalLink className="size-3 shrink-0" />
          </a>
        )}
      </div>
      {review.findings.length > 0 && (
        <ul className="flex flex-col gap-3">
          {review.findings.map((f, index) => (
            // Static, server-rendered list that never reorders — the index keeps the key unique even
            // when two raw findings are identical (file/line/severity/title), which is possible.
            // biome-ignore lint/suspicious/noArrayIndexKey: stable order, never reordered
            <FindingItem key={`${f.file}:${f.line}:${index}`} finding={f} />
          ))}
        </ul>
      )}
    </div>
  );
}

function FindingItem({ finding }: { finding: ReviewFinding }) {
  return (
    <li className="rounded-md border border-border p-3">
      <div className="flex flex-wrap items-center gap-2">
        <span
          className={`rounded px-1.5 py-0.5 text-[11px] font-medium uppercase tracking-wide ${severityClass(finding.severity)}`}
        >
          {finding.severity}
        </span>
        <span className="font-mono text-xs text-muted-foreground">
          {finding.file}:{finding.line}
        </span>
        <span className="text-sm font-medium">{finding.title}</span>
      </div>
      {finding.body && (
        <p className="mt-1.5 whitespace-pre-wrap text-sm text-muted-foreground">{finding.body}</p>
      )}
      {finding.suggestion && (
        <pre className="mt-2 overflow-x-auto rounded bg-muted p-2 font-mono text-xs">
          {finding.suggestion}
        </pre>
      )}
    </li>
  );
}
