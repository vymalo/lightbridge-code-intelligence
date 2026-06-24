# ADR-0044: Feedback memory (M1) — don't re-raise findings a human rejected

- **Status:** Proposed
- **Date:** 2026-06-24
- **Deciders:** @stephane-segning

## Context and Problem Statement

The review agent has no memory across runs, so it re-raises findings a human already rejected — the
same false positive, review after review. We capture 👍/👎 reactions on posted findings
([ADR-0035](0035-review-feedback-signal.md)) but never feed them back into future reviews.

A general agent-writable "memory tool" was considered and **deliberately deferred** (M2). Two reasons:
(1) dogfood run `7c15f9bb` showed the agent abusing a record tool as a scratchpad and looping — a
free-form memory it writes to itself would let it persist *wrong* learnings; (2) at the time the
primary model couldn't reliably call tools at all (leaked tool-call tokens). A memory the agent must
*call a tool to use* is worthless on a shaky tool-calling foundation.

So M1 is the **safe, read-only** form: memory **derived from verified human feedback**, injected as
context — no agent write tool, works even if tool-calling is flaky.

## Decision (M1)

On a `review`-kind task, the control plane looks up findings on this **repo** that received a 👎 (`-1`
reaction): join `review_feedback` (the reaction) → `review_comments` (the `(file, line)`) → that run's
`reviews.findings` (to recover the title), bounded and deduped. It formats them into a `repo_memory`
context block — *"a human rejected these; don't raise them again unless the code materially changed"* —
returned as a new optional field on `TaskContextResponse`. The runner injects it into the prompt next to
the prior-review block ([ADR-0040](0040-re-review-reads-prior-findings.md)), and the operator prompt
points the agent at it.

Properties (mirrors ADR-0040's safe-context pattern):
- **Read-only, untrusted context** — not a command; the tool-protocol stays authoritative. No agent
  write tool, so no scratchpad/poisoning path and no dependence on tool-calling.
- **Derived from verified outcomes** — only human 👎 reactions, not the agent's self-assessment.
- **`review` kind only; best-effort** — a lookup error degrades to no memory, never a failed task.
- **Bounded** (cap 30) so the prompt stays small.

## Consequences

- **Good:** the agent stops re-raising known false positives; reuses ADR-0035 feedback + ADR-0040
  injection; safe by construction (read-only, feedback-derived); independent of the model's
  tool-calling reliability.
- **Limitation:** matches by `(file, line)` against the stored finding — a path-normalization mismatch
  or a moved line silently misses a row (best-effort). It remembers *rejections*, not repo conventions
  or 👍-reinforced patterns — those are easy extensions once this proves out.
- **M2 (deferred):** an agent-writable, scoped `remember`/`recall` tool (repo facts, write-gated at
  `finish`, semantic recall) — only after the tool-calling foundation is solid (the model switch to
  `adorsys-reviewer` addresses that) and M1 shows the value.

## References

- Epic [#177](https://github.com/adorsys-gis/lightbridge-code-intelligence/issues/177).
- [ADR-0035](0035-review-feedback-signal.md) — the 👍/👎 feedback signal this consumes.
- [ADR-0040](0040-re-review-reads-prior-findings.md) — the prior-review context this mirrors.
- Dogfood run `7c15f9bb` — the scratchpad-loop that argued for read-only-memory-first.
