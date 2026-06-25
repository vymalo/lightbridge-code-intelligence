# ADR-0047: Reviewer prompt — grounding & uncertainty calibration (empty retrieval ≠ proof of absence)

- **Status:** Proposed
- **Date:** 2026-06-25
- **Deciders:** @stephane-segning

## Context and Problem Statement

The native review agent ([ADR-0037](0037-agent-acts-via-mediated-tools.md)) investigates a PR with
read-only retrieval tools (`lightbridge_vector_semantic_search`, `graph_find_symbol`,
`graph_get_callers`, `read_file`) and records findings via mediated write tools. The operator-owned
`reviewSystemPrompt` already says "ground every claim in evidence" and "calibrate uncertainty out
loud." That guidance is necessary but, as observed in production, **not sufficient** — because it does
not tell the model what an *empty* tool result means.

The measured failure mode (highest priority):

- **Hallucination on empty retrieval.** When the index is hollow or commit-mismatched, a symbol /
  vector search returns `0 hits`. Instead of reading "I could not verify this," the model
  *rationalizes the emptiness into a confident false claim.* On PR
  [#187](https://github.com/adorsys-gis/lightbridge-code-intelligence/pull/187) it wrote *"symbol
  searches confirm no lingering references"* and concluded the PR had *removed* features it never
  touched. The agent turned "the search found nothing" into "the thing does not exist / was removed" —
  the precise inversion that destroys trust, because the verdict is both confident and wrong.

This is a grounding/calibration gap, distinct from the confidently-wrong-*finding* gap
([ADR-0043](0043-review-finding-verification.md)). ADR-0043's refute pass forces re-verification of
recorded P0/P1 *findings*; it does not govern the *narrative reasoning* (the `finish` summary and the
turn-by-turn claims) where this failure lives, nor does it teach the model how to interpret an empty
result in the first place. The fix is a grounding contract in the operator prompt, backed by primary
prompt-engineering guidance from Anthropic and OpenAI.

This is epic [#177](https://github.com/adorsys-gis/lightbridge-code-intelligence/issues/177).

## Decision Drivers

- **Kill the empty-retrieval hallucination** — the highest-priority observed failure.
- **A wrong claim costs more trust than a missed nit** — the project's standing review doctrine.
- **Prompt-only, model-portable.** The reviewer runs on **GLM** behind an OpenAI-compatible Chat
  Completions gateway, not Claude or GPT. The decision must rest on *general* grounding principles
  (which transfer), not Claude- or GPT-specific features.
- **The runner already surfaces the signal.** `result_summary` renders an empty retrieval as
  `0 hits` in logs (agent.rs); the gateway returns a literal `[]` to the model. The model *sees* the
  emptiness — it just mis-reads it. So this is a prompt-interpretation fix, not a tooling fix.
- **No code change required to ship.** The grounding contract is operator config; the runner already
  carries the evidence/retract machinery it leans on.

## Decision

Add an explicit **Grounding & uncertainty** contract to the operator `reviewSystemPrompt`. Three rules,
each tied to a primary source and to the observed failure:

1. **Empty / failed tool result means "could not verify" — never "absent / removed."** State it
   literally so the model cannot re-interpret `0 hits` as proof of non-existence:

   > A tool that returns no results (`0 hits`, `[]`, "not found"), or an error, means **you could not
   > verify this — the index may be stale, hollow, or mismatched to this commit.** It is **never**
   > evidence that something does not exist, was removed, or is unused. If a search comes back empty,
   > confirm with `read_file` against the actual checkout before making any claim. If you still cannot
   > confirm it, say *"could not verify"* and do not assert it as fact — in a finding or in your
   > verdict.

   This is the direct analog of Anthropic's "if it can't find a quote, it must retract the claim" and
   OpenAI's "do NOT guess or make up an answer."

2. **Cite or don't claim, applied to the *narrative*, not just findings.** ADR-0043 already requires an
   `evidence` field on each `add_review_comment`. Extend the same bar to the `finish` verdict and any
   in-prose assertion: every factual statement about the code (in a finding *or* the summary) must rest
   on something a tool actually returned. An unprovable statement is dropped, not hedged into the
   verdict. Anthropic: "ground responses in quotes… quote relevant parts first before carrying out its
   task."

3. **Permission to say "I don't know."** Explicitly authorize the calibrated non-answer, because models
   default to fabricating a confident one. Anthropic's *Reduce hallucinations* names this as the single
   highest-leverage anti-hallucination move ("explicitly give Claude permission to admit uncertainty…
   can drastically reduce false information"):

   > You are not penalized for saying *"I reviewed X but could not verify Y"*. A precise "I could not
   > confirm this" is worth more than a confident guess. Reserve confident language for claims a tool
   > result backs.

The concrete wording lands in the operator-owned prompt (a full revised draft is proposed in
[ADR-0048](0048-review-prompt-structure-and-technique.md), which carries the structural rewrite this
contract slots into). The runner is unchanged; this decision deploys via `ai-helm-values`
`config.reviewSystemPrompt` at the human operator's discretion — **this ADR proposes the wording, it
does not deploy it.**

### Why a prompt rule and not a code guard

A code guard *could* refuse to let the model `finish` while it has made empty-retrieval claims — but the
claims live in free-text reasoning the runner does not parse, and a regex on "no lingering references"
is brittle and adversarial-fragile. The robust lever is the same one ADR-0042/0043 used: shape the
model's behaviour in the prompt, and keep the deterministic guards (refute pass, coverage gate) for the
*structured* outputs the runner can actually inspect. The runner already does the one mechanical thing
that helps — it labels `0 hits` explicitly in the result summary so the emptiness is unmissable.

## Consequences

- **Good:** the exact #187 failure ("searches confirm no lingering references" → false "removed"
  claim) is now directly contradicted by the prompt: an empty search is defined as *unverified*, and
  the model is told to fall back to `read_file` and otherwise say "could not verify." Calibrated
  uncertainty is explicitly rewarded, which the literature says is the strongest single lever.
- **Good:** composes with ADR-0043 — that ADR verifies recorded *findings*; this one governs the
  *reasoning and verdict* and the *interpretation of empty results* upstream of any finding.
- **Cost / limits:** prompt grounding reduces but does not eliminate hallucination (Anthropic states
  this plainly). A determined-wrong model can still fabricate; the refute pass (ADR-0043) and the human
  reviewer remain the backstops. The "confirm with `read_file`" instruction can cost an extra read when
  retrieval is empty — acceptable, and bounded by the existing `max_files_read` budget (ADR-0042).
- **Verification (proposed):** the offline eval harness in
  [ADR-0049](0049-eval-driven-reviewer-prompt-iteration.md) includes a golden case that *simulates a
  hollow index* (all retrieval returns `[]`) and asserts the run does **not** emit an
  absence/removal claim — turning this ADR from a hope into a testable contract before deploy.

## What this deliberately defers

- **A structured "verifiability" signal on findings** (machine-checkable evidence) — ADR-0043's
  `evidence` field is the current mechanism; a stricter schema is a later step if needed.
- **Detecting a hollow index server-side** and refusing to start a review against a known-stale index —
  a control-plane concern, tracked separately; this ADR makes the agent *safe* against a hollow index
  rather than *preventing* one.

## References

- Epic [#177](https://github.com/adorsys-gis/lightbridge-code-intelligence/issues/177).
- [ADR-0037](0037-agent-acts-via-mediated-tools.md) — the agent loop + prompt assembly this governs.
- [ADR-0043](0043-review-finding-verification.md) — evidence citation + refute pass on *findings* (this
  ADR extends the bar to the *narrative* and the *empty-result interpretation*).
- [ADR-0048](0048-review-prompt-structure-and-technique.md) — the structural rewrite this contract slots
  into (carries the full revised prompt draft).
- [ADR-0049](0049-eval-driven-reviewer-prompt-iteration.md) — the eval that makes this contract testable.
- Anthropic, *Reduce hallucinations* — allow "I don't know"; quote-then-answer; "if it can't find a
  quote, it must retract the claim": https://platform.claude.com/docs/en/docs/test-and-evaluate/strengthen-guardrails/reduce-hallucinations
- Anthropic, *Prompt engineering — long context tips* — ground responses in quotes; query-at-end:
  https://platform.claude.com/docs/en/docs/build-with-claude/prompt-engineering/use-xml-tags
- OpenAI, *GPT-4.1 Prompting Guide* — agentic tool-calling reminder ("do NOT guess or make up an
  answer"): https://developers.openai.com/cookbook/examples/gpt4-1_prompting_guide
