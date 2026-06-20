# ADR-0022 — Review write-back is validated and posted by the control plane

| Field      | Value |
|------------|-------|
| Status     | Accepted |
| Date       | 2026-06-19 |
| Deciders   | @ssegning |
| Epic       | #5 (indexer + agent, slice 6) |
| Builds on  | [ADR-0002](0002-rust-control-plane-trust-boundary.md), [ADR-0021](0021-opencode-headless-review-agent.md) |

## Context

Slice 5 produces a structured `ReviewResult` (summary + findings). Slice 6 turns it into an actual
PR review. Two questions:

1. **Who posts to GitHub?** The runner is an untrusted per-task Job and holds no GitHub App key
   (ADR-0002, ADR-0017). So the runner submits the review to the control plane, which mints the
   installation token and posts.
2. **How do findings become inline comments?** GitHub's "create review" endpoint rejects the *whole*
   review if any inline comment targets a line that isn't part of the PR diff. The agent can cite
   any line, so findings must be validated against the diff first.

## Decision

**The runner submits the review to `POST /internal/tasks/{id}/review`; the control plane validates
against the PR diff and posts a single PR review.**

- The control plane resolves the PR (owner/repo/number) and installation from the task, mints the
  token, and fetches the PR's changed files (`GET …/pulls/{n}/files`).
- It parses each file's unified-diff `patch` to compute the **commentable** RIGHT-side lines (added +
  context). A finding anchors to an **inline** comment only if its `(file, line)` is commentable;
  findings that don't (wrong file, line outside the diff) are **deferred into the review body** so
  nothing the agent found is lost. Findings are deduped by `(file, line, title)`.
- One PR review is posted with `event: COMMENT` — a body (summary + deferred findings) plus the
  validated inline comments.
- Submission is **non-fatal** in the runner: indexing and the review already succeeded, so a
  write-back hiccup is logged, not a task failure.

## Consequences

**Good**
- GitHub write credentials stay in the control plane (trust boundary), like every other datastore
  and external write.
- Diff-validation means a single bad line ref can't sink the whole review; out-of-diff findings
  still surface in the body.
- The patch parser + validation/dedup are pure functions, unit-tested without network.

**Trade-offs**
- `list_pr_files` reads only the first page (≤100 files) — fine for typical PRs; pagination is a
  follow-up.
- **No cross-run dedup yet**: re-running a task posts another review. Acceptable for now (tasks are
  idempotent at creation; re-runs are rare); a "skip if we already posted" guard is a follow-up.
- Inline comments use `line` + `side: RIGHT` (the new file). Comments on deleted lines aren't
  supported (rare for review findings).
- The GitHub calls aren't exercised in CI (no live App); the validation/shaping logic is.

## Alternatives rejected

- **Runner posts to GitHub directly** — would put a GitHub write credential in the untrusted Job.
- **Post every finding inline, no validation** — one out-of-diff line ref fails the entire review.
- **Body-only (no inline comments)** — loses the precise line anchoring that makes a review useful.

## Amendment (2026-06-20): scope to the diff, structured body, GitHub suggestions

The original shaping **deferred** every non-anchorable finding into the body — including findings on
files the PR never touched, which read as a whole-repo audit. `validate` now scopes to the PR:

- finding on a changed line → **inline** comment, and if it carries a `suggestion` (ADR-0021
  amendment), the comment includes a committable ```suggestion block;
- finding on a changed *file* but an unpinnable line → **body** (`### Notes on changed files`);
- finding on a file **not** in the PR's changed set → **out of scope**, dropped and counted (a
  transparency line in the body says how many were omitted).
- Safety valve: if the changed-file set can't be determined (empty `commentable`), defer rather than
  drop, so a transient failure never silently empties the review.
- The body carries the working-agreement disclosure: AI-generated, untrusted, a human owns the
  decision.

## Follow-ups

- Cross-run dedup (don't repost on re-run).
- `list_pr_files` pagination for very large PRs.
- PR overlay / incremental indexing (index only changed files for a PR) — the remaining item from
  the epic-#5 acceptance criteria, an optimization separate from write-back.
- Multi-line suggestions (the current ```suggestion block replaces a single anchored line).
