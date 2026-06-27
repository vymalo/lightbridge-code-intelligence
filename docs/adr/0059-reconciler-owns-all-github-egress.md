# ADR-0059: The reconciler owns all GitHub egress (transactional outbox)

- **Status:** Accepted
- **Date:** 2026-06-27
- **Deciders:** @stephane-segning

## Context and Problem Statement

Outbound writes to GitHub are scattered across two roles:

- **serve** posts the grouped review + inline comments and any reply on `finalize`, and reacts 👀
  ("seen") on webhook receipt / 😕 on a reported failure — all **synchronously, in the request path**.
- **reconciler** (née `poller`, [ADR-0058](0058-rename-poller-role-to-reconciler.md)) posts the
  failure notice for an uncatchable kill ([ADR-0057](0057-poller-posts-failure-notice-on-uncatchable-kill.md))
  and reads reactions inbound.

Two independent writers is the root of several problems:

1. **It forced the [ADR-0057](0057-poller-posts-failure-notice-on-uncatchable-kill.md) settle buffer.**
   serve and the reconciler both post failure notices, so a 2-minute buffer + a `has_posted_to_github`
   gate exist purely to stop them racing a double-post. That's accidental complexity from having two
   posters, not essential.
2. **GitHub coupling on the hot path.** Because `finalize` posts the review inline, a GitHub outage or
   secondary-rate-limit stall **blocks the runner's `finalize` call** — an ingest-path request waiting
   on a third party.
3. **Split rate-limit budget + duplicated policy.** Retry, back-off, dedup, and ordering logic have to
   exist in two places, against one shared GitHub rate-limit budget that neither owner can see whole.

## Decision

**Funnel every outbound GitHub *content* write through the reconciler via a transactional outbox.** There
is exactly one writer to GitHub.

### The outbox

A `github_outbox` table of **intent** rows:

```
id (BIGSERIAL — the monotonic per-row key the drain orders on) · task_id
kind (review | reply | reaction | label | failure_notice)
payload (jsonb — the FULLY-SHAPED content to post, pre-rendered by the producer) · dedup_key
status (pending | posted | failed) · attempts · last_error
created_at · posted_at · github_id (the posted review/comment id)
```

- **Producers write intent — fully shaped — in the same transaction as the domain change.** They
  `INSERT` the outbox row and `pg_notify('github_outbox')` inside that txn, so the post can't be lost if
  the producer crashes after commit and can't fire if it rolls back (at-least-once, tied to the domain
  write). Crucially, **all the shaping happens at produce time**: the PR-diff fetch (`list_pr_files`) and
  the inline / deferred / out-of-scope validation + body rendering that `finalize_review` does today run
  in the producer and are baked into the `payload`. Those are GitHub *reads*, not writes — so "serve
  stops calling the GitHub write API" still holds — and it keeps the reconciler a **dumb poster** (it
  ships bytes; it never parses a diff). The producers are:
  - **serve / finalize** — the grouped review (always enqueued, *including the empty-buffer clean-review
    backstop*: an empty successful run still enqueues a review intent, so an @mention review never goes
    silent), the reply, the 👀/😕 reactions, and a **`label`** intent (the `add_review_labels` outcome
    labels — `reviewed`/`findings`/`error` — must ride the outbox too, or they stay a second serve-side
    writer and defeat single-egress).
  - **the reaper** — for an *uncatchable* kill (no `finalize`, no status report ever reaches serve), the
    reaper enqueues the `failure_notice` intent when it marks the task `failed`. The keyless dispatcher
    **can** write an intent *row* (it has DB access) even though it can't *post* — which is exactly what
    lets the outbox close the [ADR-0057](0057-poller-posts-failure-notice-on-uncatchable-kill.md) gap
    cleanly, with no separate sweep.
- **The reconciler is the sole consumer — a single-replica role** (the invariant inherited from the
  poller, now load-bearing: a second replica would break the per-task ordering below). It `LISTEN`s on
  `github_outbox` (timer fallback, as the dispatcher already runs for `task_queued`), claims `pending`
  rows **`ORDER BY created_at, id FOR UPDATE SKIP LOCKED`** — `SKIP LOCKED` serializes *claiming*, not
  order, so the ordering is explicit and `id` breaks the `created_at` ties — posts via the App token, and
  marks `posted` with the returned `github_id` (feeding the same `review_comments`/`reviews` rows the
  feedback poll joins on). A failed post → `attempts++`, `last_error`, retried with back-off.

### What moves, what stays

- **serve stops calling the GitHub write API entirely.** The agent's `pending_review_actions` buffer
  ([ADR-0037](0037-agent-acts-via-mediated-tools.md)) is *already* an outbox; the review **flush moves
  from serve's synchronous `finalize` to the reconciler's drain** — `finalize` shapes the review (above)
  and enqueues the intent, then returns. Replies, the 👀/😕 reactions, the labels, and the failure notice
  all become outbox `kind`s.
- **serve keeps the App key — for non-content GitHub work, not posting.** What it loses is *content
  writes*; *reads* and *token-mints* stay, because the keyless dispatcher can't do them and they're
  synchronous: minting the runner's clone token at Job bootstrap, and the admin/install path's read to
  resolve a repo's default branch before the first index
  (`admin.rs` — [ADR-0017](0017-agent-runner-control-plane-bootstrap.md) bootstrap). This is a *role*
  distinction within the one binary, so [ADR-0002](0002-rust-control-plane-trust-boundary.md)'s
  key-placement is unchanged; what changes is that serve no longer *posts content*.
- **The 👀 "seen" reaction also goes through the outbox** (near-instant via the NOTIFY). Chosen over
  keeping one synchronous exception: uniform egress is more future-proof than a special case, and the
  NOTIFY round-trip keeps "seen" effectively immediate.

## Consequences

- **Good — the dual-poster and its settle buffer are retired.** With one writer there is no race;
  [ADR-0057](0057-poller-posts-failure-notice-on-uncatchable-kill.md)'s `> 2 minutes` buffer and the
  reaper-can't-post problem both disappear — the notice is just another outbox `kind`, drained like the
  rest. The keyless-dispatcher constraint stops mattering because nothing but the reconciler posts.
- **Good — ingest no longer blocks on GitHub.** `finalize` returns once the review is *durably queued*;
  a GitHub outage drains later instead of failing the runner. One writer also means one rate-limit
  budget and one retry/dedup/ordering policy.
- **Good — testability.** Egress collapses to "intent row in → GitHub call out," unit-testable without
  driving a webhook or a finalize through the whole stack.
- **Cost — eventual, not synchronous, posting.** A review/reaction posts on the NOTIFY (sub-second
  typical) or the timer fallback, not in the producing handler. The finalize HTTP response no longer
  means "it's on GitHub" — it means "it will be." Acceptable: nothing downstream needs the post to have
  already landed.
- **Cost — ordering within a task.** A task can enqueue several items (review, then a reaction) whose
  order matters; the reconciler drains a task's rows in **`(created_at, id)`** order. The `id` tie-breaker
  is load-bearing: rows enqueued in one transaction share `created_at` (Postgres `now()` is
  transaction-stable), so `created_at` alone is not a total order. Ordering also assumes the
  **single-replica** invariant above.
- **Risk — double-delivery is possible, and `dedup_key` does not fully close it.** At-least-once means a
  crash *between* the GitHub POST and the `posted` mark can repost on retry — and a *local* check can't
  prevent it, because GitHub accepted the write while Postgres never recorded the id, and GitHub exposes
  no idempotency key for reviews/comments. So `dedup_key` prevents a double-*enqueue*, and a pre-post
  `has_posted_to_github`-style check catches the common retry-before-any-post case, but neither can see a
  write that landed-then-lost-its-ack. We **accept rare duplicates** (at-least-once beats at-most-once —
  a duplicate comment is recoverable; a silently-lost review is not) and **minimize the window** by
  marking `posted` on the same connection immediately after the API returns. Where a `kind` can check
  remotely and cheaply (e.g. `failure_notice` via the existing `has_posted_to_github`), it does; the rest
  tolerate the rare dup rather than claim an idempotency we don't have.
- **Migration — none up front.** The outbox carries only *new* writes; in-flight work drains via the old
  path during rollout. Staged so each step is shippable and shrinks serve's egress surface:
  **(1)** rename ([ADR-0058](0058-rename-poller-role-to-reconciler.md)) → **(2)** outbox + move the
  failure notice (retires the settle buffer) → **(3)** reactions → **(4)** the review/reply flush. After
  (4), serve posts nothing.

## References

- [ADR-0058](0058-rename-poller-role-to-reconciler.md) — the role rename (stage 1, prerequisite).
- [ADR-0037](0037-agent-acts-via-mediated-tools.md) — the mediated-tool buffer that *is* the review
  outbox; this ADR moves its flush to the reconciler's drain.
- [ADR-0056](0056-control-plane-owns-the-posted-output.md) /
  [ADR-0057](0057-poller-posts-failure-notice-on-uncatchable-kill.md) — the failure-notice + dual-poster
  this supersedes (the notice becomes an outbox `kind`; the settle buffer is removed).
- [ADR-0002](0002-rust-control-plane-trust-boundary.md) — trust boundary / App-key placement (unchanged;
  egress is consolidated within it).
- [ADR-0017](0017-agent-runner-control-plane-bootstrap.md) — serve's retained non-content key use (clone
  token at Job bootstrap; default-branch resolution).
- [ADR-0035](0035-review-feedback-signal.md) — the inbound reaction read the reconciler keeps doing.
- The existing `LISTEN/NOTIFY` precedent: `dispatcher` on `task_queued` (`PgListener` + timer fallback).
