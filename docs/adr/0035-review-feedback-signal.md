# ADR-0035: Capture 👍/👎 on posted reviews as a feedback signal

- **Status:** Accepted (capture mechanism **corrected** below: GitHub does not webhook reactions — poll the REST API instead)
- **Date:** 2026-06-22

## Context and Problem Statement

When a human reacts 👍/👎 to a comment the bot posted, that is a free, high-value judgement of review
quality. Today we capture none of it. The bot posts **outbound** reactions (👀/🎉/😕) but the webhook
does not subscribe to **inbound** reaction events, there is no table to store feedback, and — the
blocker — we **don't persist the GitHub review/comment IDs we create**: write-back keeps only
`review_url` (`migrations/0010_review_url.sql`), discarding the review `id` and per-inline-comment ids
from the create-review response (`control-plane/src/integrations/github.rs`). So even if a 👎 webhook
arrived, we couldn't map it back to a finding.

We want this signal **displayed in the dashboard** and retained as **training/evaluation data** to
improve the system.

## Decision Drivers

- **Don't lose a free quality signal** tied to specific findings.
- **Correlatable:** a reaction must map back to the exact finding/review it rates.
- **Surface + retain:** show it in the dashboard; keep it as a dataset for later evaluation.

## Decision Outcome

Two parts; the first is a prerequisite and lands first regardless of the rest:

1. **Persist created IDs (prerequisite).** On write-back, capture and store `github_review_id` and each
   inline comment's `github_comment_id`, correlated to the finding that produced it (extend the
   `reviews`/findings rows or add a `review_comments` table). Cheap, independently useful, and unblocks
   everything below — **do this now even before the feature ships.** Note: GitHub's create-review
   response is a single review object (id + `html_url`); the per-inline-comment ids are **not** in it,
   so capturing them needs a follow-up `GET /pulls/{pr}/reviews/{id}/comments` correlated back to
   findings by `(path, line)`.
2. **Capture reactions — by polling, not a webhook (CORRECTED).** GitHub does **not** emit webhook
   events for reactions (verified against the webhook-events docs: there is no `reaction` event, and
   `issue_comment`/`pull_request_review_comment` fire on comment create/edit/delete, never on a
   reaction). So the original "subscribe to reaction events" plan is infeasible. Instead, a **periodic
   job polls the reactions REST API** for the comment/review ids we own
   (`GET …/comments/{id}/reactions`) and **reconciles** the result into **`review_feedback`**
   `(finding ref, github_comment_id, reactor, reaction, created_at)`: new reactions are upserted, and a
   reaction that has disappeared is deleted/tombstoned — reconciliation gives us the "un-react"
   ("deleted") behaviour for free without a webhook. Aggregate per finding and per run; **expose in the
   dashboard run page** ([ADR-0016](0016-dashboard-information-architecture.md)) as 👍/👎 counts on each
   finding and a run-level summary. The dataset (finding text + transcript
   [ADR-0034](0034-agent-run-transcript-and-observability.md) + verdict) becomes the seed for offline
   evaluation/tuning. The poller fits the existing control-plane background loops (dispatcher/reaper) or
   the jobs sidecar ([ADR-0028](0028-agent-job-control-sidecar.md)).

### Consequences

- Good: a real quality signal per finding, shown to operators and retained for improvement; reuses the
  existing GitHub App webhook + reaction plumbing.
- Bad: net-new inbound event handling, a feedback table, and dashboard work; reaction→finding mapping
  needs the IDs from part 1; GitHub only exposes a small reaction vocabulary.
- Neutral: "use it to improve the system" (training/eval) is a **future** consumer — this ADR commits to
  *capturing and displaying*; the learning loop is a follow-up.

## Pros and Cons of the Options

### Persist IDs + capture reactions (chosen)
- Good: correlatable, displayable, retainable; incremental (IDs first).
- Bad: webhook + schema + UI work; mapping depends on stored ids.

### Link out to GitHub only (no capture)
- Good: zero backend work — `review_url` already lets you click through.
- Bad: no aggregation, no dataset, no per-finding signal in our UI; the value evaporates.

## More Information

- Prerequisite touches `control-plane/src/integrations/github.rs` (`ReviewResponse` — keep `id` +
  comment ids) and the `reviews` schema. Inbound handling in `control-plane/src/http/webhook.rs`.
  UI in `apps/web/components/runs/review-output.tsx`. Builds on
  [ADR-0022](0022-review-writeback-control-plane.md) (write-back) and pairs with
  [ADR-0034](0034-agent-run-transcript-and-observability.md) (transcript) for the eval dataset.
