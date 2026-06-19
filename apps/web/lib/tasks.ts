/**
 * Task-run domain types + presentation logic for the dashboard (ADR-0016). Mirrors the control
 * plane's `/tasks` payload (`TaskRow` in `services/control-plane/src/db.rs`). Pure + Edge-safe.
 */

/** One task run, as returned by `GET /tasks` and `GET /tasks/{id}`. */
export interface Task {
  id: string;
  repository_id: number;
  installation_id: number;
  github_delivery_id: string;
  target_type: string;
  target_id: number;
  command_text: string;
  base_sha: string | null;
  head_sha: string | null;
  status: string;
  priority: number;
  created_at: string;
  started_at: string | null;
  completed_at: string | null;
  repo_owner: string | null;
  repo_name: string | null;
  repo_default_branch: string | null;
}

/** The small visual set statuses collapse to (ADR-0015/0016 tokens). */
export type StatusVariant = "pending" | "active" | "success" | "error" | "muted";

/** Map a raw `TaskStatus` string (snake_case from the DB) to its visual variant + label. */
export function statusVisual(status: string): { variant: StatusVariant; label: string } {
  switch (status) {
    case "received":
      return { variant: "pending", label: "Received" };
    case "waiting_for_index":
      return { variant: "pending", label: "Waiting for index" };
    case "queued":
      return { variant: "pending", label: "Queued" };
    case "running":
      return { variant: "active", label: "Running" };
    case "posting_result":
      return { variant: "active", label: "Posting result" };
    case "succeeded":
      return { variant: "success", label: "Succeeded" };
    case "failed":
      return { variant: "error", label: "Failed" };
    case "timed_out":
      return { variant: "error", label: "Timed out" };
    case "cancelled":
      return { variant: "muted", label: "Cancelled" };
    default:
      // Unknown future status: show it verbatim rather than hiding it.
      return { variant: "pending", label: status };
  }
}

/** Human repo slug (`owner/name`), or a stable fallback when the join came back empty. */
export function repoLabel(task: Task): string {
  if (task.repo_owner && task.repo_name) return `${task.repo_owner}/${task.repo_name}`;
  return `repo #${task.repository_id}`;
}

/** What triggered the run, e.g. `review · PR #123`. */
export function triggerLabel(task: Task): string {
  const target =
    task.target_type === "pull_request"
      ? `PR #${task.target_id}`
      : `${task.target_type} #${task.target_id}`;
  return `${task.command_text} · ${target}`;
}

/** First 7 chars of a SHA (git short form), or null. */
export function shortSha(sha: string | null): string | null {
  return sha ? sha.slice(0, 7) : null;
}

const RELATIVE_UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
  ["year", 31_536_000],
  ["month", 2_592_000],
  ["week", 604_800],
  ["day", 86_400],
  ["hour", 3600],
  ["minute", 60],
  ["second", 1],
];

/** "3 minutes ago" from an ISO timestamp, relative to `now` (defaults to current time). */
export function relativeTime(iso: string, now: number = Date.now()): string {
  const seconds = Math.round((new Date(iso).getTime() - now) / 1000);
  const abs = Math.abs(seconds);
  const fmt = new Intl.RelativeTimeFormat("en", { numeric: "auto" });
  for (const [unit, secondsInUnit] of RELATIVE_UNITS) {
    if (abs >= secondsInUnit || unit === "second") {
      return fmt.format(Math.round(seconds / secondsInUnit), unit);
    }
  }
  return fmt.format(0, "second");
}

/** Run duration `started→completed` (or `started→now` if still running), formatted compactly. */
export function duration(task: Task, now: number = Date.now()): string | null {
  if (!task.started_at) return null;
  const start = new Date(task.started_at).getTime();
  const end = task.completed_at ? new Date(task.completed_at).getTime() : now;
  const seconds = Math.max(0, Math.round((end - start) / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const rem = seconds % 60;
  if (minutes < 60) return rem ? `${minutes}m ${rem}s` : `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${minutes % 60}m`;
}

/** Absolute timestamp for tooltips / detail rows. */
export function absoluteTime(iso: string): string {
  return new Date(iso).toLocaleString("en", {
    dateStyle: "medium",
    timeStyle: "short",
  });
}
