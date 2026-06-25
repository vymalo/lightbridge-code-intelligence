# ADR-0045: Context-window budget — converge before overflow, never discard findings

- **Status:** Proposed
- **Date:** 2026-06-24
- **Deciders:** @stephane-segning

## Context and Problem Statement

The native review agent loop ([ADR-0026](0026-native-review-agent.md)) sends the *entire* accumulated
`messages` array to the model every turn — system prompt, the diff, and every prior turn's assistant
reply + tool results. Nothing trims, summarizes, or budgets that growth. With `max_turns` up to 150,
`read_file` returning up to 64 KiB per call, `max_files_read=30`, `max_searches=15`, and a diff up to
`max_diff_chars=60 000`, the conversation can exceed the model's context window.

When it does, the gateway returns an HTTP 400 ("context length exceeded"). The transport layer
([ADR-0039](0039-agent-llm-resilience-and-observability.md)) correctly classifies a 400 as **deterministic** (not transient),
so the loop does `return Err(...)` — which fails the whole task. Because findings only flush to GitHub
on `finish`, **every buffered finding from the run is discarded** at the exact moment the agent has
done the most work. That is the worst possible time to fail.

Two gaps:
1. **No budget.** The config knobs cap the *count* of reads/turns, never the *cumulative tokens*. The
   peak context size is unbounded relative to the model's window — the config can't express "stay under
   N tokens".
2. **No graceful degradation.** Overflow is a hard failure that throws away real, already-found bugs.

## Decision

Add a **context-token budget** to the agent loop, in two tiers.

### Tier 1 — never discard findings on overflow (backstop)

When a deterministic chat error is a **context-overflow** (matched against the gateway's error text:
`context length`, `maximum context`, `context_length_exceeded`, `too many tokens`, `reduce the
length`), the loop stops investigating and **finalizes** (returns `ReviewOutcome::Exhausted`) instead
of `return Err`. The existing finalize path posts the buffered findings with a truncation note
([ADR-0037](0037-agent-acts-via-mediated-tools.md)). A genuine non-overflow 4xx still fails fast as today.

### Tier 2 — converge before overflow (proactive)

A new optional config `context_window` (tokens; `None` = disabled, preserving current behavior):

- **Estimate** the conversation's token cost each turn with a cheap, conservative heuristic
  (`chars / 4` over the serialized messages + tool defs, plus a small per-message overhead). The
  gateway model isn't OpenAI-tokenized, so an exact tokenizer would be false precision and a heavy
  dependency; a conservative over-estimate with headroom is the right tool.
- **Token-aware wind-down.** Reuse the existing wind-down machinery: if `context_window` is set and the
  estimate exceeds `context_window × WINDDOWN_FRACTION` (0.75), enter wind-down — drop the investigation
  tools and force `finish` — exactly as the turn-budget and batch-budget triggers already do
  (`in_winddown = turn >= winddown || batches_spent || tokens_spent`).
- **Trim consumed tool output.** A single large `read_file` can blow past the threshold in one turn. So
  when the estimate is over budget, **shrink the content of the oldest tool-result messages** to a stub
  (`[earlier tool output elided to fit the context budget]`), keeping the message and its
  `tool_call_id` so the assistant/tool pairing stays valid. This reclaims the bulk (file/search bodies
  the agent has already reasoned over) while leaving the system prompt, the diff, the recorded-finding
  trail, and recent turns intact — enough headroom for the agent to emit its final verdict.

`context_window` is operator config, sourced like the other model knobs: `ai-helm-values`
`config.model.contextWindow` → chart `review.context_window` → `LLM_CONTEXT_WINDOW` / `ReviewConfig`.
Unset everywhere = today's behavior (no budgeting), so this is safe to ship dark and enable per-model.

## Consequences

- **Good:** a productive run can no longer be thrown away by overflow; long investigations converge to
  a verdict instead of dying; the peak context is bounded by config and tied to the actual model.
- **Good:** reuses the existing wind-down lever and the `Exhausted`-finalizes-anyway contract — small,
  composable change, not a new code path.
- **Cost / limits:** the token estimate is approximate; `WINDDOWN_FRACTION=0.75` leaves headroom for
  estimator error and the final turn. Trimming old tool output means a late turn can't re-read an
  elided result verbatim — acceptable, since by wind-down the agent should be concluding, not
  investigating. Tier 1 remains the backstop if the estimate still undershoots.
- **Follow-up:** if estimator drift proves material, revisit with a provider-reported token count
  (some gateways echo prompt tokens in `usage` on the *previous* turn — usable as a calibration signal).

## References

- Epic [#177](https://github.com/adorsys-gis/lightbridge-code-intelligence/issues/177).
- [ADR-0026](0026-native-review-agent.md) — the agent loop this budgets.
- [ADR-0037](0037-agent-acts-via-mediated-tools.md) — buffered findings + `Exhausted` finalizes anyway.
- [ADR-0039](0039-agent-llm-resilience-and-observability.md) — error classification; overflow is a deterministic 4xx today.
- [ADR-0042](0042-risk-first-review-and-parallel-batching.md) — the read budgets + wind-down this extends.
