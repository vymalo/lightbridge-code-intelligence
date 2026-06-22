import { GitBranch, GitCommitHorizontal } from "lucide-react";
import Link from "next/link";
import { StatusPill } from "@/components/ui/status-pill";
import {
  duration,
  relativeTime,
  repoLabel,
  shortSha,
  type Task,
  triggerLabel,
} from "@/lib/domain/tasks";

/** One row in the run list — status · trigger · repo · branch · sha · relative time · duration. */
export function RunRow({ task, now }: { task: Task; now: number }) {
  const sha = shortSha(task.head_sha);
  const dur = duration(task, now);
  const branch = task.repo_default_branch;
  return (
    <Link
      href={`/dashboard/runs/${task.id}`}
      className="flex items-center gap-3 px-4 py-3 transition-colors hover:bg-base-300/60"
    >
      <StatusPill status={task.status} className="w-32 shrink-0 justify-start" />
      <div className="min-w-0 flex-1">
        <div className="truncate text-sm font-medium">{triggerLabel(task)}</div>
        <div className="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs text-base-content/60">
          <span className="truncate">{repoLabel(task)}</span>
          {branch && (
            <span className="inline-flex items-center gap-1">
              <GitBranch className="size-3" />
              {branch}
            </span>
          )}
          {sha && (
            <span className="inline-flex items-center gap-1 font-mono">
              <GitCommitHorizontal className="size-3" />
              {sha}
            </span>
          )}
        </div>
      </div>
      <div className="hidden shrink-0 text-right text-xs text-base-content/60 sm:block">
        <div title={task.created_at}>{relativeTime(task.created_at, now)}</div>
        {dur && <div className="mt-0.5 font-mono">{dur}</div>}
      </div>
    </Link>
  );
}
