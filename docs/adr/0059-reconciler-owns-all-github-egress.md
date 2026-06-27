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
id · task_id · kind (review | reply | reaction | failure_notice)
payload (jsonb — what to post) · dedup_key
status (pending | posted | failed) · attempts · last_error
created_at · posted_at · github_id (the posted comment/review id)
```

- **Producers (serve, the finalize path) only write intent.** In the *same transaction* that records the
  domain change, they `INSERT` the outbox row and `pg_notify('github_outbox')`. Transactional means the
  post can't be lost if the producer crashes after commit, and can't fire if the transaction rolls back
  — at-least-once delivery tied to the domain write.
- **The reconciler is the sole consumer.** It `LISTEN`s on `github_outbox` (with the timer fallback we
  already run for `task_queued`), claims `pending` rows (`FOR UPDATE SKIP LOCKED`), posts via the App
  token, and marks `posted` with the returned `github_id` (feeding the same `review_comments`/`reviews`
  rows the feedback poll already joins on). A failed post → `attempts++`, `last_error`, retried with
  back-off; `dedup_key` makes a redelivery idempotent.

### What moves, what stays

- **serve stops calling the GitHub write API entirely.** The agent's `pending_review_actions` buffer
  ([ADR-0037](0037-agent-acts-via-mediated-tools.md)) is *already* an outbox; the review **flush moves
  from serve's synchronous `finalize` to the reconciler's drain** — `finalize` just records the task as
  ready-to-post and notifies. Replies, the 👀/😕 reactions, and the failure notice all become outbox
  `kind`s.
- **serve keeps the App key** — but only to **mint the runner's clone token** at Job bootstrap
  (synchronous auth plumbing the keyless dispatcher can't do; not content egress). This is a *role*
  distinction within the one binary, so [ADR-0002](0002-rust-control-plane-trust-boundary.md)'s
  key-placement is unchanged; what changes is that serve no longer *posts*.
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
  order matters; the reconciler processes a task's rows in `created_at` order. An impl detail, but a real
  one.
- **Risk — double-delivery.** At-least-once means a crash between the GitHub post and the `posted` mark
  could repost. Mitigated by `dedup_key` (checked before posting) plus GitHub idempotency where
  available; the existing `has_posted_to_github` gate generalizes into this check.
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
- [ADR-0035](0035-review-feedback-signal.md) — the inbound reaction read the reconciler keeps doing.
- The existing `LISTEN/NOTIFY` precedent: `dispatcher` on `task_queued` (`PgListener` + timer fallback).
