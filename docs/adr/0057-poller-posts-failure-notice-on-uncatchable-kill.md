# ADR-0057: The poller posts the failure notice for an uncatchable kill

- **Status:** Accepted
- **Date:** 2026-06-27
- **Deciders:** @stephane-segning

## Context and Problem Statement

[ADR-0056](0056-control-plane-owns-the-posted-output.md) gave a failed PR review a voice: instead of
silence, the control plane posts a brief *"something went wrong — re-mention me to retry"* notice. But
it shipped with one scoped gap, called out in that ADR's Consequences:

> The failure notice fires on the **runner-reported** path (`POST /internal/tasks/{id}/status` → serve
> role). An **uncatchable** kill (OOM / SIGKILL / node eviction) never reports; it is detected by the
> **reaper** in the **dispatcher** role, which holds **no GitHub App key** (ADR-0002) and so cannot post.

So the worst failures — the ones the runner can't even report — are exactly the ones that stay silent.
A pod OOM-killed mid-review, or evicted off a drained node, leaves the author watching a PR that never
gets a review and never gets a "whoops" either. The reaper marks the task `failed`
([`reaper.rs`](../../services/control-plane/src/queue/reaper.rs)), but the dispatcher it runs in has no
way to reach GitHub.

## Decision

**The poller posts the notice the dispatcher can't.** The poller (ADR-0035) is already a single-replica
role that loops on a timer **and holds the GitHub App key** — the exact shape this needs. Each cycle,
after the feedback poll, it runs a **failure-notice sweep**:

1. `failed_pr_tasks_without_feedback(within_days)` selects PR review tasks that ended terminally
   (`failed`/`timed_out`) with **nothing posted** — no `reviews` row, no `review_comments` row — bounded
   to those completed within the last `within_days` (ancient failures are abandoned, not re-litigated)
   **and** more than a couple of minutes ago.
2. For each, it posts the same notice via the shared
   [`failure_notice::post_if_unposted`](../../services/control-plane/src/failure_notice.rs), which the
   serve path now also calls. That helper re-checks `has_posted_to_github` under the same dedup gate and
   records the notice as a `failure_notice` comment.

**Race-free with serve by a settle buffer, not a lock.** The `> 2 minutes` floor means a *reported*
failure — which serve handles **synchronously** the instant the runner posts its status — is never in
the sweep's set while serve still owns it. A reaper-marked kill (no serve handler at all) simply waits
out the short buffer. Past that, the shared dedup gate (`has_posted_to_github` + the recorded
`failure_notice` row) makes the whole thing idempotent: once a notice is posted, the task drops out of
the set on the next cycle, and the sweep quiesces to empty.

The 😕 **reaction** stays serve-only (it needs the review config the poller doesn't carry); the
reaper-path notice lands as the comment **without** the reaction. That's an accepted asymmetry — the
comment is what breaks the silence; the emoji is garnish.

## Consequences

- **Good:** the uncatchable-kill gap is closed — every terminally-failed PR review now gets either a
  real review or a notice, regardless of *how* it died. The sweep is also a **backstop for serve's own
  best-effort path**: if serve's synchronous post fails (token mint blip, GitHub 5xx), the next sweep
  picks the task up and retries, where before that failure was final.
- **Good (no new surface):** reuses the existing poller loop, App key, per-installation token cache, and
  the ADR-0056 dedup gate. No new role, deployment, `NOTIFY` listener, or credential placement — and the
  notice-posting logic now lives in **one** shared function instead of being duplicated.
- **Cost — latency.** A reaper-path notice is not instant: it lands on the next poller tick after the
  settle buffer (≈ `POLLER_INTERVAL_SECS`, default 300s, after the kill is reaped). Acceptable — the
  author has already been waiting minutes for a review that died; a few more before the apology is fine.
  A *reported* failure is still instant via serve.
- **Bound — `within_days`.** A failure older than the poller window never gets a notice. Deliberate: an
  abandoned task from days ago has no audience left, and an unbounded scan would walk all of history.

## References

- [ADR-0056](0056-control-plane-owns-the-posted-output.md) — the failure notice + the gap this closes.
- [ADR-0035](0035-review-feedback-signal.md) — the poller role (single replica, holds the App key).
- [ADR-0002](0002-rust-control-plane-trust-boundary.md) — the App key lives on serve + poller, **not**
  the dispatcher, which is why the reaper can't post and the poller must.
- Code: [`failure_notice.rs`](../../services/control-plane/src/failure_notice.rs) (shared post),
  [`queue/poller.rs`](../../services/control-plane/src/queue/poller.rs) (`sweep_failure_notices`),
  [`db.rs`](../../services/control-plane/src/db.rs) (`failed_pr_tasks_without_feedback`).
