# ADR-0054: Stay on MiniMax-M2.7 (FP8) on DeepInfra for the review agent

- **Status:** Accepted
- **Date:** 2026-06-26
- **Deciders:** @stephane-segning

## Context and Problem Statement

The native review agent ([ADR-0026](0026-native-review-agent.md)) runs a single model
([ADR-0053](0053-remove-review-fallback-model.md)) configured per-model via `review.model`
([ADR-0051](0051-per-model-config.md)), served over an OpenAI-compatible endpoint. Today that is
**`MiniMaxAI/MiniMax-M2.7`** (228B MoE, **FP8**, 196,608-token context) on **DeepInfra**, priced
**$0.05 cached / $0.25 in / $1.00 out per 1M tokens**.

M2.7 "works fine," but two questions were open:

1. Is there a model that is a **genuine Pareto improvement** — both *faster* and *more intelligent* —
   we should switch to?
2. A prior look at **GLM** felt **very slow** (its FP4 quant on DeepInfra). Was that a quant problem,
   a model problem, or a provider problem?

We are cost-anchored to DeepInfra: cheap FP4/FP8 serverless inference is why most of our models live
there. Any move must respect that.

## Decision

**Stay on MiniMax-M2.7 (FP8) on DeepInfra.** No model or provider change. This ADR records the
market scan behind that choice and the triggers that would reopen it. The lever for any future change
is `review.model` in `ai-helm-values` (a one-line, no-rebuild swap per ADR-0051) — not a code change.

## Findings (market scan, mid-2026)

> **Confidence:** provider **throughput / TTFT / price** figures below are well-corroborated across
> independent sources (provider rate cards, Artificial Analysis). The **intelligence** percentages
> (SWE-bench, τ²-Bench, BFCL) come from mixed-quality aggregators with inconsistent version numbering
> and should be treated as **directional**; trust an in-house eval over any leaderboard.

### 1. "GLM is slow" was a DeepInfra-serving artifact, not FP4 as a class

GLM-4.7 **full** on DeepInfra is **FP4 at ~22.5 tok/s** — vs **~789.8 tok/s on Cerebras (~35× faster)**
for the same model. So the slowness was **DeepInfra's GLM kernel**, not FP4 in general and not GLM as a
model. Our M2.7 is **FP8** (higher quality retention than FP4) and is served well on DeepInfra. Takeaway:
quant class alone does not predict speed — the **(model × provider)** pairing does.

### 2. M2.7 on DeepInfra is the price floor, at identical quant

Same model, same FP8, across providers:

| Provider | In $/M | Out $/M | Cached | Quant | Context |
|---|---|---|---|---|---|
| **DeepInfra** *(current)* | **$0.25** | **$1.00** | **$0.05** | FP8 | 196,608 |
| Novita | $0.30 | $1.20 | $0.06 | FP8 | 204,800 |
| Fireworks | $0.30 | $1.20 | $0.06 | FP8 (228B MoE) | ~196K |

DeepInfra is ~20% cheaper on both input and output for an **identical** FP8 228B MoE — no quality
traded for the price. Novita/Fireworks simply carry margin.

### 3. The only axis a provider move would buy is raw throughput

For M2.7 specifically (other providers, by output speed):

- **Together AI ≈ 542 tok/s** — fastest M2.7 endpoint, ~cost-neutral (~$0.22/M blended).
- **SambaNova ≈ 424 tok/s** but **TTFT ≈ 7.75 s** — high first-token latency is poison for a
  multi-turn agentic loop (TTFT is paid every tool round-trip), so *not* suitable despite the throughput.
- **DeepInfra / Novita / Fireworks** ≈ double-digit–~125 tok/s — the "fine, not fast" tier we are in.

### 4. "Faster AND smarter" exists only off DeepInfra (Cerebras), and breaks the cost model

**GLM-4.7 on Cerebras**: ~1,150 tok/s, ranked **#1 on Berkeley Function-Calling (tool use)**,
**τ²-Bench 87.4 vs MiniMax M2's 77.2** (agentic), ~Sonnet-4.5-class. But Cerebras prices GLM-4.7 as a
**Preview** ($2.25 in / $2.75 out) or via **monthly subscription Code plans** ($50/$200) — a
rate-limited, non-metered model that does not fit our per-token DeepInfra cost posture.

### 5. Within DeepInfra, alternatives are a different cost/intelligence point — not a free win

| Model (DeepInfra) | Quant | In $/M | Out $/M | Speed | vs M2.7 |
|---|---|---|---|---|---|
| **MiniMax-M2.7** *(current)* | FP8 | 0.25 | 1.00 | ~68 t/s, TTFT <1s | baseline |
| GLM-4.7 (full) | FP4 | 0.40 | 1.75 | **~22.5 t/s** | slower + dearer — avoid here |
| GLM-4.7-Flash | FP8 | 0.06 | ~0.14 blended | ~228 t/s, 0.75s TTFT | faster + cheaper, lower ceiling |
| DeepSeek-V4-Flash | — | 0.10 | 0.20 | ~83 t/s | cheaper + faster, intelligence TBD |
| DeepSeek-V4-Pro | FP4 | 1.30 | 2.60 | — | smarter, pricier |
| Kimi K2.6 | FP4 | 0.75 | 3.50 | — | strongest agentic, **3.5× output cost** |
| Qwen3-235B-Instruct-2507 | FP8 | 0.09 | 0.10 | — | dirt cheap, non-reasoning |

The genuinely-smarter options cost either **speed** (GLM-full 22.5 t/s) or **money** (Kimi $3.50 out);
the genuinely-cheaper/faster options (the Flash class) trade reasoning ceiling. There is **no
strictly-dominant upgrade inside DeepInfra** over M2.7-FP8 for agentic review.

## Consequences

- **Good:** lowest token cost for the model we run, at the best-quality quant available (FP8, not FP4);
  sub-second TTFT keeps the multi-turn tool loop responsive; zero migration/eval risk. The decision is
  evidence-backed and the reopening triggers are explicit.
- **Cost / limits:** we accept M2.7's mid-tier **throughput** (~tens of tok/s) — we are optimizing
  cost+TTFT+quality, *not* peak tokens/sec. A "faster **and** smarter" model exists only on Cerebras,
  whose subscription/preview pricing we have explicitly declined for now.

### Reopen this decision if

- **Wall-clock latency** starts to matter more than the ~20% token premium → first test **M2.7 on
  Together AI (~542 tok/s, ~cost-neutral)** — same model, lowest-risk speed win — *before* changing models.
- We want a **cheaper default** → eval **GLM-4.7-Flash** or **DeepSeek-V4-Flash** (both faster + cheaper
  on DeepInfra) on our golden review cases ([ADR-0049](0049-eval-driven-reviewer-prompt-iteration.md)).
- We want a **higher intelligence ceiling** regardless of cost/speed → eval **Kimi K2.6** (DeepInfra) or
  **GLM-4.7 on Cerebras** (accepting its non-metered pricing).
- DeepInfra ships a **fast GLM-full kernel** (current ~22.5 tok/s is the blocker, not the model).

Any swap is a `review.model` change in `ai-helm-values` ([ADR-0051](0051-per-model-config.md)) and
should be gated by an eval pass ([ADR-0049](0049-eval-driven-reviewer-prompt-iteration.md)), not shipped
on leaderboard scores.

## References

- [ADR-0026](0026-native-review-agent.md) — native review agent (single OpenAI-compatible model).
- [ADR-0038](0038-per-repo-review-model.md) — per-repo model from an operator allowlist.
- [ADR-0051](0051-per-model-config.md) — per-model config block (the swap lever).
- [ADR-0053](0053-remove-review-fallback-model.md) — single model + retry/breaker (no failover).
- [ADR-0049](0049-eval-driven-reviewer-prompt-iteration.md) — eval-driven model/prompt changes.
- Provider/benchmark sources (mid-2026): Artificial Analysis (MiniMax-M2.7, GLM-4.7, Cerebras
  provider pages), DeepInfra model pages + blog benchmarks, Novita & Fireworks M2.7 model pages,
  Cerebras GLM-4.7 blog. Throughput/price corroborated; intelligence scores directional.
