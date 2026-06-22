# ADR-0037: The agent acts via mediated tools; the run kind is emergent

- **Status:** Proposed
- **Date:** 2026-06-22
- **Deciders:** @stephane-segning

## Context and Problem Statement

The native review agent ([ADR-0026](0026-native-review-agent.md)) ends a run by calling **one
terminal tool** — `submit_findings` — that returns a single structured payload; the control plane
then validates the whole batch against the PR diff and posts it as one review
([ADR-0022](0022-review-writeback-control-plane.md)). To add a conversational answer,
[ADR-0033](0033-inbound-command-parsing-and-run-kinds.md) introduced a **second** terminal tool
(`submit_answer`) and a **second** behaviour selected by an **up-front keyword classifier** that
resolves a `kind` (review vs ask) *before* the agent runs.

This shape has three problems. (1) It hard-codes **one output per run** — the agent can either file
findings or answer, never both, and "more in the future" (post a label, open a follow-up, cite a
fetched URL) means yet another terminal tool + branch each time. (2) The **keyword classifier is
brittle**: it guesses intent from words like "review"/"audit" and the model — a far better intent
reader — is reduced to a fallback. (3) Findings are validated **in a batch at the end**, so an
out-of-scope finding is discovered only after the fact and bucketed/surfaced rather than corrected
(the silent-drop class [ADR-0033](0033-inbound-command-parsing-and-run-kinds.md) fought).

Should the agent instead **act through mediated, side-effecting tools it calls *during* the loop** —
"add a review comment", "add a comment", "search" — with the run kind **emerging** from which tools
it used, rather than declared up front?

## Decision Drivers

- **Generality:** one loop should cover review, ask, and future actions without a new branch each time.
- **Let the model decide intent** from the user's message, not a keyword list.
- **No silent drops:** an off-diff finding should be a recoverable, per-call error the agent sees and
  fixes — not a post-hoc bucket.
- **Trust boundary is non-negotiable** ([ADR-0002](0002-rust-control-plane-trust-boundary.md)): the
  untrusted per-task Job must never hold GitHub credentials; the control plane owns all writes.
- **Preserve the review UX:** inline comments grouped into **one** PR review (one notification, one
  summary), and **nothing posted** if a run dies mid-way.
- **Idempotency:** a retried task must not double-post.
- **Auditability + feedback:** the posted artifacts must be recorded with their GitHub IDs for the
  feedback signal ([ADR-0035](0035-review-feedback-signal.md), #144).
- **Stay in scope** ([ADR-0029](0029-focused-review-not-generic-runner.md)): tools that *review and
  answer about code*, not a generic step runner.

## Considered Options

- **Option A — Status quo:** terminal payload per kind (`submit_findings` / `submit_answer`) + the
  up-front classifier.
- **Option B — Mediated action tools; kind emergent; buffer + flush one grouped review** at clean
  completion.
- **Option C — Mediated action tools, but post each comment immediately (streaming).**

## Decision Outcome

Chosen option: **Option B**, because it satisfies every driver at once — one loop, model-decided
intent, per-call validation, and the trust boundary — while preserving today's grouped-review UX and
crash-safety that pure streaming (Option C) would sacrifice.

The agent is given the user's message and a **toolbox**, and it *acts*:

1. **Read tools** (unchanged): `lightbridge_vector_semantic_search`, `graph_find_symbol`,
   `graph_get_callers`, and future read-only tools (e.g. fetch a URL).
2. **Write actions** — the structure that was `submit_findings`'s payload becomes the **arguments** of
   tools the agent calls as it goes:
   - `add_review_comment(file, line, body, priority, category, suggestion?)` — an inline finding.
   - `add_comment(body)` — a plain reply on the thread.
   - `set_summary(body)` — the run's summary/verdict (drives outcome labels).
   - …extensible (a label, a follow-up) without a new loop.
3. **Mediation + the trust boundary:** every write action is dispatched **runner → control plane**
   (the same callback path the read tools already use), so the Job still holds no GitHub key. The
   control plane is the **policy point**: it validates each `add_review_comment` against the PR diff
   **at call time** and returns a recoverable error to the agent when the line isn't in the diff
   ("pick a changed line, or use `add_comment`") — the agent corrects itself; nothing is silently
   dropped.
4. **Buffer, then flush once:** the control plane **accumulates everything** for the task — inline
   findings, the summary, *and* any `add_comment` bodies — and posts **nothing** until **clean loop
   completion**. On completion it flushes the inline findings + summary as **one grouped PR review**,
   and consolidates the buffered `add_comment` bodies into **a single thread reply** (so multiple
   `add_comment` calls never fan out into a notification storm). A run that aborts or dies mid-way
   flushes nothing (crash-safe).
5. **Idempotency on retry:** the accumulation buffer is keyed by `(task, run_epoch)` and is **cleared
   when the runner (re)starts** that task, so a retry begins from empty rather than appending to a
   partial buffer. Within a run, accumulated inline findings are deduplicated by **`(file, line)` —
   last write wins**, *not* a content hash: an LLM re-run is non-deterministic and will reword the same
   finding, so a content hash would let near-duplicates through; keying on position keeps the latest,
   most-refined finding per line.
6. **Empty runs always produce feedback:** the agent is instructed to call `set_summary` exactly once
   as its final verdict (the analogue of the old always-call-`submit_findings`). If a run nonetheless
   completes cleanly having called **no** write tool, the control plane posts a **default "no issues
   found" review** so an `@mention`-triggered run never looks like a silent hang. (An `ask` that
   produced no answer is likewise summarized rather than silent.)
7. **The run kind is emergent:** it is *derived* from which tools fired (inline findings ⇒ "review";
   only a reply ⇒ "ask"; both ⇒ both; neither ⇒ "review" with the default clean summary) and recorded
   for analytics/observability — it is **no longer an input** and there is **no up-front classifier**.
   `classify_kind` and the `submit_findings` / `submit_answer` terminal split are retired.

### Consequences

- **Good:** one agent loop and one growing toolbox cover review, ask, and future actions — no new
  branch per capability. The brittle keyword classifier is gone; the model decides from the message.
  Per-call diff validation **eliminates the silent-drop class** instead of mitigating it. The trust
  boundary is unchanged (writes stay control-plane-mediated). Review UX (one grouped review) and
  crash-safety are preserved. Feedback ([ADR-0035](0035-review-feedback-signal.md)) gains per-comment
  granularity for free.
- **Bad / accepted trade-off:** more internal write endpoints and **server-side accumulation state**
  per in-flight task (flushed or discarded at end), plus a **clear-on-restart + `(file, line)`
  last-write-wins** dedup so non-deterministic retries don't double-post. The summary/verdict and
  outcome labels move from fixed payload fields to a `set_summary` tool + derivation, so the agent must
  be steered to always call it — and the control plane backstops with a default clean review when it
  doesn't, so feedback is never silent.
- **Neutral / to watch:** [ADR-0022](0022-review-writeback-control-plane.md)'s "validate the whole
  batch, then post" becomes "validate per call, buffer, flush once at completion" — same guarantees,
  different timing. The transcript ([ADR-0034](0034-agent-run-transcript-and-observability.md))
  becomes a natural log of the tool actions taken. A single combined system prompt must still keep the
  **review discipline sharp** (scope rule, priority/category) even though the same agent can also
  answer — prompt quality is the thing to monitor, given review quality is the live concern.

## Pros and Cons of the Options

### Option A — terminal payload per kind + classifier (status quo)

- Good: simplest write path (one batch, one validate, one post); atomic by construction.
- Good: the review prompt is single-purpose and undiluted.
- Bad: one output per run; every new behaviour is a new terminal tool + branch.
- Bad: brittle keyword classifier; the model is only a fallback.
- Bad: findings validated post-hoc → the silent-drop class persists as a bucket, not a fix.

### Option B — mediated action tools; emergent kind; buffer + flush (chosen)

- Good: one loop, model-decided intent, per-call validation, trust boundary intact, grouped UX kept.
- Good: extensible to future actions without structural change.
- Bad: accumulation state + idempotency to build; summary/labels become a tool/derived.

### Option C — mediated action tools, post immediately (streaming)

- Good: live feedback; the simplest server state (no buffering).
- Bad: N separate comments and notifications instead of one grouped review; a mid-run crash leaves a
  **partial** review with no summary; idempotency is harder. The UX/atomicity cost isn't worth the
  streaming, given reviews complete in seconds.

## More Information

- **Supersedes the run-kind mechanism of**
  [ADR-0033](0033-inbound-command-parsing-and-run-kinds.md): the kind is now emergent (no input, no
  classifier). The rest of ADR-0033 carries forward — honouring the user's words (#138), surfacing
  rather than dropping out-of-scope findings, and non-PR targets (its slice 3) — re-expressed in terms
  of tools (off-diff ⇒ recoverable error or `add_comment`).
- **Evolves the output mechanism of** [ADR-0026](0026-native-review-agent.md) (terminal
  `submit_findings` → incremental `add_*` action tools; the in-process native loop and structured tool
  calls are kept) and the **write-back timing of**
  [ADR-0022](0022-review-writeback-control-plane.md) (batch validate-then-post → per-call validate +
  buffered single flush; the control-plane-owns-writes invariant is kept).
- Builds on [ADR-0026](0026-native-review-agent.md) (the loop + control tools) and
  [ADR-0017](0017-agent-runner-control-plane-bootstrap.md) (runner ↔ control-plane callback).
- Source of truth: #142 (run kinds); incident
  [ai-helm#436](https://github.com/ADORSYS-GIS/ai-helm/pull/436).
- Interim: PR #154 ships the ask path under the ADR-0033 mechanism (an explicit `kind` + the
  `submit_answer` terminal tool) to fix the live ai-helm#436 incident now; this ADR is the target the
  next slice refactors toward, at which point `classify_kind` and the terminal split are removed.
