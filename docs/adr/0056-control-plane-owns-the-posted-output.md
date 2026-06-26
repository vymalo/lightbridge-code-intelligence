# ADR-0056: The control plane owns what gets posted (PR review-only channel + failure notice)

- **Status:** Accepted
- **Date:** 2026-06-26
- **Deciders:** @stephane-segning

## Context and Problem Statement

The agent never posts to GitHub directly — it buffers mediated actions (`add_review_comment` /
`add_comment` / `finish`) in `pending_review_actions`, and the control plane **flushes them as one
grouped review on `finalize`** ([ADR-0037](0037-agent-acts-via-mediated-tools.md)). But the flush is a
**faithful dump**, not a policy gate, and two gaps showed up live (2026-06-26, ai-helm#501):

1. **Duplicate / junk channel on a PR.** The agent buffered three `add_comment` "answers" that were
   just progress narration — *"Still reviewing the remaining changed files… Continuing review…"* —
   and the control plane dutifully posted them as a **separate issue comment** alongside the real PR
   review. The author got two messages: a good review **and** a useless "still reviewing" note. The
   agent had mis-used `add_comment` (the *answer* channel, meant for issues / @mention questions) as a
   progress channel, and nothing stopped it from reaching the PR.
2. **Silence on failure.** When a review task fails terminally **without** finalizing (the agent hit an
   error, aborted, or exhausted retries), the control plane sets `tasks.error_detail` **for the web
   console only** — it posts **nothing** to GitHub. The author who triggered the review just sees
   silence and can't tell it failed.

## Decision

**Make the control plane the policy owner of what reaches GitHub** — not a passthrough.

1. **PR is a review-only channel.** In `finalize_review`, the buffered `add_comment` replies are
   flushed **only on a non-PR target** (an issue / @mention question, where the answer *is* a comment).
   On a `pull_request` target the verdict belongs solely in the grouped review, so the buffered replies
   are **dropped** (logged, not posted). This kills the junk-comment class **structurally** — the agent
   can buffer all the "still reviewing" narration it wants and none of it can reach a PR.
2. **Failure-fallback notice.** When a review task on a PR goes terminally `failed`/`timed_out`, the
   control plane (serve role, which already holds the App token) posts a brief *"something went wrong —
   re-mention me to retry"* comment — **only if nothing was already posted** for the task
   (`has_posted_to_github`: no `reviews` row and no `review_comments` row). That check makes it safe
   against a finalize-then-crash double-post and idempotent across retries (the notice records itself as
   a `review_comments` row).

## Consequences

- **Good:** the duplicate/junk-comment failure mode is gone by construction (no reliance on the model
  behaving). A failed review is no longer silent to the author. Both changes live where the trust
  boundary already is — the control plane owns the post ([ADR-0002](0002-rust-control-plane-trust-boundary.md),
  [ADR-0022](0022-review-writeback-control-plane.md)).
- **Limit — the uncatchable-kill gap (deliberately scoped).** The failure notice fires on the
  **runner-reported** path (`POST /internal/tasks/{id}/status` → serve role). An **uncatchable** kill
  (OOM / SIGKILL / node eviction) never reports; it is detected by the **reaper** in the **dispatcher**
  role, which holds **no GitHub App key** (ADR-0002) and so cannot post. Covering that needs a
  serve/poller-side component (the poller already loops with the App key, or a `NOTIFY` from
  `set_task_status` to a serve listener) — deferred as a follow-up. The common case (the agent hits an
  error and reports it) is covered now.
- **Note:** `add_comment` is **not** removed — it remains the correct channel for an issue or an
  @mention question (no diff → the answer is a comment). The gate is by `target_type`, not a deletion.

## References

- [ADR-0037](0037-agent-acts-via-mediated-tools.md) — mediated tools + buffer/flush-on-finalize.
- [ADR-0002](0002-rust-control-plane-trust-boundary.md) — the control plane owns the trust boundary
  (and the App key lives on serve + poller, not the dispatcher).
- Dogfood evidence: ai-helm#501 (run `95e6bb76`) — the junk "still reviewing" comment + a 25-min review.
