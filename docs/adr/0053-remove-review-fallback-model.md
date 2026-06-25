# ADR-0053: Remove the review fallback model

- **Status:** Accepted
- **Date:** 2026-06-26
- **Deciders:** @stephane-segning

## Context and Problem Statement

[ADR-0039](0039-agent-llm-resilience-and-observability.md) added optional **failover to a secondary
review model**: on a turn where the primary exhausts its transient retries, the loop re-runs that turn
against a configured fallback model. [ADR-0051](0051-per-model-config.md) then gave the fallback its
**own** per-model config block (model id + generation params + timeout + retries).

In practice the failover earns little and costs a lot of surface area:

- It needs a **second `ChatClient`**, a per-turn failover branch in the agent loop, and a parallel
  config tree (`FallbackFile`/`ModelTuningFile` file structs, a `FallbackConfig` runtime struct, and a
  `resolve_fallback` that inherits the primary's effective values).
- A model swap is an **operational** decision (change `LLM_MODEL` / `review.model` in `ai-helm-values`,
  which re-renders the agent secret with no rebuild — see ADR-0051), **not** a per-turn runtime
  concern. The primary model + bounded retry/backoff + the per-run circuit breaker already cover a
  transient gateway blip; a *persistently* unreliable model is fixed by swapping it, not by carrying a
  standby on every run.
- The fallback path is rarely exercised and adds a second failure mode to reason about.

## Decision

**Remove the review fallback/failover entirely.** A review runs a **single** model with the existing
retry + circuit-breaker resilience (ADR-0039). The agent loop no longer builds a fallback client or
branches to it; the config no longer carries a fallback model.

**Transition-safety (this is a config *removal*, which `deny_unknown_fields` makes dangerous):** the
live `ai-helm-values` still sets `config.model.fallbackModel`, which renders into the runner's
`agent.json`. If the new runner image simply dropped the field, `deny_unknown_fields` would reject that
`agent.json` and **fail every review closed**. So `ReviewFile.fallback_model` and `ReviewFile.fallback`
are **kept as parsed-but-ignored fields** (with a startup `warn!` when either is set), and removed in a
later step. The three-step rollout:

1. **This change** — drop the failover logic + runtime structs; keep the two file fields parsed/ignored.
2. **Operator** — delete `fallbackModel` from `ai-helm-values`.
3. **Follow-up PR** — delete the now-dead `fallback_model` / `fallback` fields from `ReviewFile`.

## Consequences

- **Good:** simpler transport (one client, no per-turn failover branch) and a smaller config surface
  (deleted `FallbackFile`, `ModelTuningFile`, `FallbackConfig`, `resolve_fallback`). One failure mode,
  not two.
- **Cost / limits:** no automatic model failover within a run. A persistently bad model degrades
  reviews until an operator swaps `review.model`. Acceptable: that swap is a one-line, no-rebuild
  change, and the circuit breaker already fails a down chain fast instead of burning the turn budget.
- **Supersedes:** the **failover** portion of [ADR-0039](0039-agent-llm-resilience-and-observability.md)
  (retry + circuit breaker stand) and the **fallback** portion of [ADR-0051](0051-per-model-config.md)
  (the primary's per-model config + the embeddings config block stand).

## References

- [ADR-0039](0039-agent-llm-resilience-and-observability.md) — resilience (retry/breaker kept; failover
  removed).
- [ADR-0051](0051-per-model-config.md) — per-model config (primary + embeddings kept; fallback removed).
- Epic [#177](https://github.com/adorsys-gis/lightbridge-code-intelligence/issues/177).
