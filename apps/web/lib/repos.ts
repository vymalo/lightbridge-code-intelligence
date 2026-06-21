/**
 * Repository domain types for the Repositories view (ADR-0016). Mirrors the control plane's
 * `/repositories` payload (`RepositoryRow` in `services/control-plane/src/db.rs`). Edge-safe.
 */

/** A connected repository plus its run-activity summary. */
export interface Repository {
  id: number;
  github_repo_id: number;
  owner: string;
  name: string;
  default_branch: string;
  /** Approval gate (Epic #75): `pending` | `approved` | `disabled`. `active` mirrors `approved`. */
  status: string;
  active: boolean;
  approved_at: string | null;
  approved_by: string | null;
  task_count: number;
  /** ISO timestamp of the most recent run, or null if none yet. */
  last_task_at: string | null;
}

/** `owner/name` slug. */
export function repoSlug(repo: Repository): string {
  return `${repo.owner}/${repo.name}`;
}
