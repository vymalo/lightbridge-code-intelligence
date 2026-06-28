# ADR-0050: Reviews reuse the latest indexed snapshot (no per-PR re-index)

- **Status:** Accepted
- **Date:** 2026-06-25
- **Deciders:** @stephane-segning

## Context and Problem Statement

A review reuses a base semantic index plus the PR diff ([ADR-0025](0025-review-reuses-base-index.md));
retrieval (`search_code_chunks`, graph) is pinned to a `(repository_id, commit_sha)` scope, and the
runner skips the full re-index when the control plane reports `repo_indexed = true`.

[ADR-0048-era commit fix #188] made that skip-decision **commit-scoped** to kill a *hollow index*: the
old check was repo-level ("any chunks?"), so an index built at commit *A* answered "indexed" while a
search pinned to commit *B* returned **zero** hits (dogfood run `7c15f9bb`). #188 keyed both the
skip-check and retrieval on the same commit — `head_sha.unwrap_or(default_branch)`.

That fixed the hollow index but exposed a cost. The index is maintained on the **default branch** by
re-index-on-push ([#183](https://github.com/adorsys-gis/lightbridge-code-intelligence/pull/183)), so it
has chunks at the *default-branch* commit — never at a PR's head. With retrieval pinned to the PR head,
`repo_has_index(head_sha)` is always false, so **every PR review (and every push to a PR) runs a full
re-index** before reviewing. Observed live on run `590af322` (review of PR #191): ~1170 chunks + graph
re-indexed, ~3.5 min, on a repo that was already indexed. The two prior behaviours were the only
options on offer: *hollow* (repo-level check) or *re-index-every-PR* (head-scoped check).

## Decision

Pin review reuse and all retrieval to the repository's **latest indexed snapshot**, not the PR head.

- New `latest_indexed_commit(repository_id)` returns the `commit_sha` of the most recently written
  `code_chunks` row (`ORDER BY created_at DESC LIMIT 1`), or `None` if never indexed.
- **Skip decision** (`get_context`): `repo_indexed = latest_indexed_commit(...).is_some()`.
- **Retrieval scope** (`task_scope`): pin to `latest_indexed_commit(...)`, falling back to the head /
  default branch only when the repo has no index at all.

Both reference the *same* `latest_indexed_commit`, so the skip-decision and the search scope can never
disagree — the search always queries a commit that provably has chunks. This **keeps #188's core
insight** (skip-check and retrieval must reference the same indexed commit) while changing *which*
commit that is, from "the PR head" to "the latest snapshot we actually have."

This works because a review's retrieval is for **base/repo context** (callers, related code); the PR's
own changes are already in the prompt as the diff ([ADR-0025](0025-review-reuses-base-index.md)). So
retrieving against the default-branch snapshot — kept fresh by re-index-on-push — is correct, and there
is no need to re-index the PR head. The runner is unchanged: it already skips when `repo_indexed` is
true. This is a control-plane-only change.

## Consequences

- **Good:** PR reviews on an indexed repo **skip the full re-index** (the ~3.5-min cost on run
  `590af322` disappears) and still get real, non-hollow search hits.
- **Good:** supersedes #188's mechanism without regressing it — same "one commit for both" guarantee,
  better commit choice. `repo_has_index(repo, commit)` is removed (it was only the skip-check).
- **Cost / limits:** retrieval context is the *latest indexed* snapshot, normally the default branch,
  not the PR head — intended (the diff carries the PR's own changes). A cold repo's first review still
  indexes (at the head it checked out); re-index-on-push then converges the latest snapshot to the
  default branch. A stale snapshot (if re-index-on-push lags) is still real context — far better than a
  per-PR re-index or a hollow index.
- **Follow-up:** incremental indexing ([#63](https://github.com/adorsys-gis/lightbridge-code-intelligence/issues/63))
  would let a review layer only the diff's files onto the base snapshot — the natural next step once
  reuse is the default.

## References

- Epic [#177](https://github.com/adorsys-gis/lightbridge-code-intelligence/issues/177).
- [ADR-0025](0025-review-reuses-base-index.md) — reviews reuse the base index + the PR diff.
- [ADR-0037](0037-agent-acts-via-mediated-tools.md) — the mediated retrieval tools this scopes.
- #188 — the commit-scoped `repo_has_index` this supersedes (kept its same-commit insight).
- #183 — re-index-on-push, which keeps the default-branch snapshot fresh for reuse.
- Run `590af322` (review of PR #191) — the per-PR full re-index this removes.
