import { ChevronRight, ExternalLink } from "lucide-react";
import { cn } from "@/lib/cn";
import type { Review, ReviewFinding } from "@/lib/tasks";

/** Severity → daisyUI badge color. Unknown severities fall back to a neutral (ghost) chip. */
function severityBadge(severity: string): string {
  switch (severity.toLowerCase()) {
    case "critical":
    case "error":
      return "badge-error";
    case "high":
    case "warning":
    case "warn":
      return "badge-warning";
    case "medium":
    case "low":
    case "info":
      return "badge-info";
    default:
      return "badge-ghost";
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
        <p className="text-xs text-base-content/60">{counts.join(" · ")}</p>
        {review.review_url && (
          <a
            href={review.review_url}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-1 text-xs text-primary transition-colors hover:underline"
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

/** A finding as a disclosure row (ADR-0024, Lovable pattern): the #103 format collapsed to
 * `severity · title · file:line`, expanding to the body, suggestion, and resources. Native
 * `<details>` so it works without client JS; high-severity findings open by default. */
function FindingItem({ finding }: { finding: ReviewFinding }) {
  const hasDetail = Boolean(finding.body || finding.suggestion || finding.resources?.length);
  const defaultOpen = ["critical", "error"].includes(finding.severity.toLowerCase());

  // No body/suggestion/resources → a static row, not a `<details>` (an expandable-but-empty box
  // reads as broken).
  if (!hasDetail) {
    return (
      <li>
        <div className="flex flex-wrap items-center gap-2 rounded-md border border-base-content/15 p-3">
          <FindingHeader finding={finding} />
        </div>
      </li>
    );
  }

  return (
    <li>
      <details open={defaultOpen} className="group rounded-md border border-base-content/15">
        <summary className="flex cursor-pointer list-none flex-wrap items-center gap-2 p-3 [&::-webkit-details-marker]:hidden">
          <ChevronRight className="size-3.5 shrink-0 text-base-content/60 transition-transform group-open:rotate-90" />
          <FindingHeader finding={finding} />
        </summary>
        <div className="border-t border-base-content/15 px-3 pb-3 pt-2.5">
          {finding.body && (
            <p className="whitespace-pre-wrap text-sm text-base-content/60">{finding.body}</p>
          )}
          {finding.suggestion && (
            <pre className="mt-2 overflow-x-auto rounded bg-base-300 p-2 font-mono text-xs">
              {finding.suggestion}
            </pre>
          )}
          {finding.resources && finding.resources.length > 0 && (
            <div className="mt-2">
              <span className="text-[11px] font-medium uppercase tracking-wide text-base-content/60">
                Resources
              </span>
              <ul className="mt-1 flex flex-col gap-0.5">
                {finding.resources.map((url, index) => (
                  // Static list that never reorders; index keeps the key unique when a URL repeats.
                  // biome-ignore lint/suspicious/noArrayIndexKey: stable order, never reordered
                  <li key={index}>
                    <a
                      href={url}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="break-all text-xs text-primary transition-colors hover:underline"
                    >
                      {url}
                    </a>
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>
      </details>
    </li>
  );
}

/** The shared collapsed header for a finding: severity chip · title · file:line. */
function FindingHeader({ finding }: { finding: ReviewFinding }) {
  return (
    <>
      <span
        className={cn(
          "badge badge-soft badge-xs font-medium uppercase tracking-wide",
          severityBadge(finding.severity),
        )}
      >
        {finding.severity}
      </span>
      <span className="text-sm font-medium">{finding.title}</span>
      <span className="ml-auto font-mono text-xs text-base-content/60">
        {finding.file}:{finding.line}
      </span>
    </>
  );
}
