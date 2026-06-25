# ADR-0048: Reviewer prompt — structure & technique adapted to a GLM / OpenAI-compatible model

- **Status:** Accepted
- **Date:** 2026-06-25
- **Deciders:** @stephane-segning
- **As-built:** the revised prompt draft is finalized in
  [`docs/drafts/review-system-prompt.md`](../drafts/review-system-prompt.md) — it applies the prime
  directives (top anchor), the Grounding & uncertainty section (ADR-0047), worked good-vs-bad examples,
  induced planning/persistence, and a Final reminders (bottom anchor) block, with a gap analysis vs the
  live prompt and exact splice points for the unchanged catalogue/reporting sections. **Model
  reconciliation:** this ADR was written referencing **GLM**; the live reviewer is now **MiniMax-M2**
  (`adorsys-reviewer` = MiniMaxAI/MiniMax-M2, `contextWindow: 204800`). The decision is unchanged — every
  technique here was deliberately chosen as **model-portable** (GPT-style, non-reasoning), so it
  transfers; §5's "tune firm phrasing by eval, don't assume" now applies to MiniMax-M2, confirmed via the
  ADR-0049 harness. Deploy remains the operator's call.

## Context and Problem Statement

The operator `reviewSystemPrompt` ([ADR-0037](0037-agent-acts-via-mediated-tools.md)) has grown to
~6.5 KB across many incremental fixes (risk-first batching ADR-0042, coverage gate ADR-0041, evidence +
refute ADR-0043, feedback memory ADR-0044, the 2026-06-24 "surface findings at all priorities" fix). It
is good in substance but **unstructured for the model that reads it**: long prose paragraphs, the key
instructions buried mid-document, no worked examples, and forceful-but-vague phrasing in places. The
reviewer runs on **GLM behind an OpenAI-compatible Chat Completions tool-calling gateway** (model
`adorsys-reviewer`, fallback `adorsys-reviewer-pro`) — a GPT-style, non-reasoning model. Frontier-lab
prompt-engineering guidance gives concrete, citable levers for exactly this shape of prompt that the
current one does not use.

The observed failure modes this targets:

- **Over-rotating on blockers** (#2): a past "filter hard" instruction made the model narrate P2s in
  prose and close with "No P0/P1 findings." Partly fixed by the 2026-06-24 prompt change; reinforce
  with a worked example of a *good* P2 finding vs. a *bad* prose mention.
- **Reviewing the repo instead of the diff** (#3, the #1 trust-killer): pre-existing issues raised as
  findings. The rule exists but is one paragraph among many — promote it to a top-and-bottom anchored
  rule and a worked good-vs-bad example.
- **Convergence** (#4): runs that never `finish`. The code-side wind-down (ADR-0042/0045) is the real
  lever, but the prompt should reinforce persistence-to-completion the way the GPT-4.1 guide
  prescribes.
- **Anchoring** (#5): findings citing real lines that don't map to the diff's addable lines — a
  good-vs-bad example fixes the failure the prose describes.

The grounding/anti-hallucination contract is a *separate* decision
([ADR-0047](0047-review-prompt-grounding-and-uncertainty.md)); this ADR is about **structure and
technique** — how the prompt is shaped so a GLM model follows it reliably. The two compose: 0047 is the
content of the grounding section, 0048 is the scaffolding it lives in.

This is epic [#177](https://github.com/adorsys-gis/lightbridge-code-intelligence/issues/177).

## Decision Drivers

- **Model-portable, not Claude/GPT-specific.** Only general techniques that transfer to a GLM model
  behind an OpenAI-style API. Where lab guidance is model-class-specific (e.g. Anthropic's "de-escalate
  forceful language" is tuned to Claude 4's strong instruction-following; OpenAI's "don't prompt
  step-by-step" applies only to *reasoning* models), we note it and pick the variant that fits GLM.
- **Every change cites a source or an observed failure** — no stylistic churn for its own taste.
- **Don't regress** the levers ADR-0040–0045 installed (prior-review context, coverage gate, refute
  pass, budgets, wind-down). The prompt rewrite preserves their hooks (tool names, the evidence field,
  the finish-once contract).
- **Reviewable, testable wording.** Ship as a draft for human review and gate it behind the eval
  harness ([ADR-0049](0049-eval-driven-reviewer-prompt-iteration.md)).

## Decision

Adopt the following prompt-engineering techniques in the operator `reviewSystemPrompt`, each grounded in
a primary source. The runner is unchanged; deployment is the operator's call via `ai-helm-values`.

### 1. Explicit sectioning with Markdown headers (delimiters)

Keep the existing `#`-header structure and tighten it to the GPT-4.1 recommended ordering: **Role &
objective → Instructions (with sub-headers) → How you review (reasoning steps) → What to hunt for →
Output format → Examples → Final reminders.** Markdown headers are the recommended default delimiter;
the diff already arrives fenced as ` ```diff `. *Source:* GPT-4.1 guide ("recommended structure"); the
guide also warns **JSON delimiters "performed particularly poorly"** in long context, so we keep prose
sections, not a JSON-encoded prompt.

### 2. Critical instructions at BOTH the top and the bottom (long-context placement)

A ~6.5 KB prompt is long enough that the middle gets lost. Anchor the two highest-stakes rules — **(a)
review only the diff, not the repo**, and **(b) empty retrieval ≠ absence (ADR-0047)** — as a one-line
"prime directives" block near the top *and* restate them in a short "Final reminders" section at the
end. *Source:* GPT-4.1 guide — "place your instructions at both the beginning and end of the provided
context… if only once, above the context works better than below." Anthropic's long-context tip (data
at top, query/instructions positioned for salience) reinforces the same.

### 3. Worked few-shot examples — good vs. bad finding (the biggest missing lever)

The current prompt *describes* a complete finding but shows none. Add **3–5 short examples** (Anthropic:
"include 3–5 examples… relevant, diverse, structured") covering the exact failure modes:

- a **good in-scope P0** (cites evidence, anchored to an added line, concrete failure mode + fix);
- a **bad out-of-scope finding** (a pre-existing issue in unchanged code) → shown as what NOT to record,
  with the correct move (one terse out-of-scope note, never P0/P1);
- a **good P2 quality finding recorded as a finding** vs. the **bad P2 narrated in prose** (failure #2);
- an **anchoring failure** (cites a real file line that isn't an added/changed line) vs. the corrected
  anchor (failure #5);
- an **empty-retrieval** example: search returns `0 hits` → the *bad* "confirms it was removed" claim
  vs. the *good* "could not verify; read the file" move (failure #1, ADR-0047).

Examples must be **diverse** (don't let the model latch onto one pattern) and wrapped consistently
(e.g. a `Good:` / `Bad:` pair under an `## Examples` header). *Source:* Anthropic *Use examples
(multishot)*.

### 4. Induced planning + persistence-to-completion (agentic reminders)

GLM is a GPT-style, non-reasoning model, so the **GPT-4.1 induced-planning advice applies** (and the
OpenAI *reasoning-model* "don't say think-step-by-step" guidance does **not** — we cite it only as the
explicit contrast). Add, in the model's own register, the three GPT-4.1 agentic reminders adapted to a
reviewer:

- *Planning before tool calls:* "Before each batch of tool calls, state the hypothesis it tests; after
  the results, reflect on what they confirmed or refuted." (Reinforces ADR-0042's hypothesis-per-batch
  rule.) Anthropic's tool-use guidance independently recommends a chain-of-thought-before-tool-call
  nudge specifically for *less capable* models — which is exactly GLM's class.
- *Persistence:* "Keep going until you have reviewed the whole diff and can give a verdict — then
  `finish`. Do not yield with the review half-done." Pairs with the code-side wind-down (ADR-0042/0045)
  that mechanically forces convergence; the prompt makes the *intent* explicit (failure #4).
- *Tool-calling honesty:* the ADR-0047 "do NOT guess; use the tools" rule.

*Source:* GPT-4.1 guide (persistence / tool-calling / planning reminders); Anthropic *Tool use — define
tools* (CoT-before-tool-call for weaker models).

### 5. Be literal and specific; keep the "why"

Modern instruction-tuned models follow instructions literally — "a single sentence firmly and
unequivocally clarifying your desired behavior is almost always sufficient" (GPT-4.1 guide). So we keep
each rule's stated *motivation* (Anthropic: "Claude is smart enough to generalize from the
explanation") but cut hedged, repetitive prose into one unambiguous sentence per rule. *Caveat:*
Anthropic's "de-escalate forceful language" tip is tuned to Claude 4.x; GLM may still need firm
phrasing, so we keep imperatives where a failure mode was observed and tune by eval (ADR-0049) rather
than assume.

### 6. Tool-use surface hygiene (already mostly in code)

The tool descriptions (tools.rs) already follow the "3–4 sentences, say when to use and what it does
NOT return" guidance (Anthropic *Tool use*). One reinforcement in the prompt: explicitly remind the
model that `read_file` is the fallback when retrieval is empty (the tool description says so; the prompt
should too, tying into ADR-0047). No code change.

### Proposed revised prompt (draft for human review)

A full rewritten `config.reviewSystemPrompt` applying the above is provided as a draft at
[`docs/drafts/review-system-prompt.md`](../drafts/review-system-prompt.md) so the operator can review
the exact wording before any `ai-helm-values` change. **It is a proposal, not a deployment** — the live
prompt is unchanged until a human edits `ai-helm-values`.

## Consequences

- **Good:** the prompt becomes structured for the model that reads it — anchored prime directives,
  worked good-vs-bad examples for every observed failure, explicit planning/persistence in the model's
  register, and literal one-sentence rules. Each change traces to a cited technique or an observed
  failure.
- **Good:** preserves every existing lever (tool names, evidence field, finish-once, the budgets/gates)
  — it restructures and exemplifies, it does not remove behaviour.
- **Cost / risk:** examples and the top+bottom anchoring add length (the very thing we're partly fighting
  for convergence). Mitigated by cutting the hedged prose elsewhere so net size stays near the current
  ~6.5 KB, and by the context-window budget (ADR-0045) which bounds the conversation regardless. Some of
  the lab guidance is model-class-specific (flagged inline); GLM behaviour must be confirmed by eval,
  not assumed — hence the hard dependency on ADR-0049.
- **Verification (proposed):** every example-backed rule has a matching golden case in the eval harness
  (ADR-0049). The prompt change is gated behind a green eval run before the operator deploys it.

## What this deliberately defers

- **Splitting the system prompt into a cached static prefix + per-run suffix** (prompt caching) — a
  cost/latency optimization, not a quality one; revisit if the gateway supports caching.
- **Native extended-thinking / reasoning blocks** — Claude-only and not available on the OpenAI-style
  gateway; induced planning (#4) is the portable substitute. The substitution is split by concern:
  induced planning (#4) stands in for what a reasoning model's "think out loud" does for *convergence*,
  while the worked examples (#3) and top+bottom anchoring (#2) — plus the refute pass (ADR-0043) — are
  the substitutes for what it does for *quality/self-correction*. GLM won't self-correct on the first
  pass the way a reasoning model does, so these explicit scaffolds carry that load.

## References

- Epic [#177](https://github.com/adorsys-gis/lightbridge-code-intelligence/issues/177).
- [ADR-0037](0037-agent-acts-via-mediated-tools.md) — prompt assembly + the mediated tools the examples
  reference.
- [ADR-0041](0041-full-diff-coverage-gate.md) — the coverage gate this prompt history references.
- [ADR-0042](0042-risk-first-review-and-parallel-batching.md) — hypothesis-per-batch + budgets the
  planning reminder reinforces.
- [ADR-0043](0043-review-finding-verification.md) — the evidence field the examples cite + the refute
  pass that complements the examples for quality.
- [ADR-0044](0044-feedback-memory-m1.md) — feedback memory, part of the prompt history this restructures.
- [ADR-0045](0045-context-window-budget.md) — the context budget that bounds the prompt + examples length.
- [ADR-0047](0047-review-prompt-grounding-and-uncertainty.md) — the grounding section this scaffolds.
- [ADR-0049](0049-eval-driven-reviewer-prompt-iteration.md) — the eval that gates this rewrite.
- OpenAI, *GPT-4.1 Prompting Guide* — structure/ordering, delimiters (JSON poor), top+bottom instruction
  placement, the three agentic reminders, literal instruction-following:
  https://developers.openai.com/cookbook/examples/gpt4-1_prompting_guide
- Anthropic, *Prompt engineering* (be clear and direct; use examples / multishot; chain-of-thought; use
  XML tags; long-context tips):
  https://platform.claude.com/docs/en/docs/build-with-claude/prompt-engineering/use-xml-tags
- Anthropic, *Tool use — implement tool use / overview* — tool description quality, CoT-before-tool-call
  for weaker models: https://platform.claude.com/docs/en/docs/build-with-claude/tool-use/implement-tool-use
- Anthropic, *Building effective agents* — ACI design, evaluator-optimizer loop, simplest-thing-first:
  https://www.anthropic.com/engineering/building-effective-agents
- OpenAI, *Reasoning best practices* — cited only as the contrast ("avoid step-by-step" applies to
  reasoning models, not GLM): https://developers.openai.com/api/docs/guides/reasoning-best-practices
