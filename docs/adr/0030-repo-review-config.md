# ADR-0030: Repo-level review configuration (`.lightbridge-code-review.jsonc`)

- **Status:** Proposed
- **Date:** 2026-06-22

## Context and Problem Statement

[ADR-0029](0029-focused-review-not-generic-runner.md) decided that customization belongs at the
**understanding layer** — the reviewed repo telling Lightbridge *how to read it* — consumed as data,
never executed. How should a repository declare that context (its conventions, what to focus on or
ignore, project-specific guidance) so reviews are grounded in the repo's own intent, with sensible
defaults when nothing is declared?

## Decision Drivers

- **Better, repo-aware reviews** (the mission): a reviewer that knows the project's conventions and
  priorities is more useful than a generic one.
- **Bounded, owned schema** → low maintenance (vs an open-ended mechanism, ADR-0029).
- **Zero code execution:** the file is **untrusted, repo-provided input** (a fork PR can change it); it
  may only influence prompt context and filtering, never execution or credentials.
- **Conventional + discoverable:** match the ecosystem (`.coderabbit.yaml`, `.sourcery.yaml`,
  `renovate.json`).
- **Optional:** strong defaults when absent.

## Decision Outcome

Support an **optional `.lightbridge-code-review.jsonc`** at the repo root (JSONC so operators can
comment their config). It is read from the **cloned working tree** after the clone step, parsed as
**data**, validated, and fed into the native review agent's context ([ADR-0026](0026-native-review-agent.md)).
Sketch:

```jsonc
{
  // Project context the reviewer should assume.
  "conventions": ["Errors are values, never thrown across module boundaries", "..."],
  "architecture": "Short prose the agent prepends as grounding context.",
  "focus": ["src/payments/**"],          // prioritize these paths
  "ignore": ["**/generated/**", "vendor/**"],
  "instructions": "Extra review guidance — what matters in this codebase, house style.",
  "severity": { "min": "info" },          // tune what gets surfaced
  "skills": { /* see ADR-0031 */ }
}
```

### Trust model (load-bearing)

The file is **untrusted input**. It may only shape **prompt context and result filtering** — it never
executes code, never grants or reads credentials, and never moves the trust boundary
([ADR-0002](0002-rust-control-plane-trust-boundary.md)). Guards:

- **Size + schema validation:** cap file size; reject unknown/oversized fields; ignore (don't fail the
  review on) a malformed config, with a surfaced warning.
- **Control-plane validation still gates output:** every finding is re-validated against the PR diff and
  posted on the trusted side ([ADR-0022](0022-review-writeback-control-plane.md)), so a hostile config
  can at worst **degrade its own review**, not escalate.
- **Fork safety (decision point):** for a PR from a fork, prefer the **base branch's** config over the
  PR head's, so a PR cannot rewrite the rules that review it. (Default: base config; allow head only for
  same-repo branches. To confirm during the follow-up RFC.)

### Precedence

`repo file` > `built-in defaults`. (An org-level default layer is a possible later addition.)

### Consequences

- **Good:** reviews grounded in the repo's stated intent; the real, mission-aligned extensibility
  surface; a bounded schema is cheap to maintain; no execution means a tiny surface.
- **Bad:** a schema to design, validate, and version; fork-config trust nuance (base-vs-head) to get
  right; another input path.
- **Neutral:** consumed by the review agent (ADR-0026); the `skills` block is specified in
  [ADR-0031](0031-review-skills-commands.md); org-level config is a future option.

## Pros and Cons of the Options

### A repo-root config file consumed as data (chosen)

- Good: conventional, discoverable, optional; data-only (safe); directly improves review quality.
- Bad: schema + fork-trust nuance to own.

### No repo config (status quo)

- Good: nothing to maintain.
- Bad: reviews can't be grounded in repo-specific intent; users have no safe customization path.

## More Information

- Realizes the "extend understanding, not execution" principle of
  [ADR-0029](0029-focused-review-not-generic-runner.md); consumed by
  [ADR-0026](0026-native-review-agent.md); output still validated/posted per
  [ADR-0022](0022-review-writeback-control-plane.md).
- `skills`/`commands` block: [ADR-0031](0031-review-skills-commands.md).
- Precedent: CodeRabbit `.coderabbit.yaml`, Sourcery `.sourcery.yaml`, Renovate config.
- Expect a follow-up RFC to fix the full schema and the fork base-vs-head decision.
