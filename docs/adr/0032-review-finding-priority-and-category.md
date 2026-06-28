# ADR-0032: Review findings carry a priority (P0–P2) and a category, not `error|warning|info`

- **Status:** Accepted
- **Date:** 2026-06-22

## Context and Problem Statement

A review finding today has a free-text `severity` string, and the agent's output contract constrains it
to `error | warning | info` (`agent-runner/src/review/mod.rs`). The dashboard maps those onto daisyUI
badges (`apps/web/components/runs/review-output.tsx`). Two problems:

- **"Info" is meaningless for a review.** A reader cannot tell a cosmetic nit from a shippable-blocker
  security hole. We want an explicit, ranked scale (e.g. **P0–P2**) where the worst issues are loud.
- **The reviewer's mission is too narrow.** `DEFAULT_REVIEW_GUIDANCE` deliberately reviews only
  correctness/security/data-loss and **excludes style and quality nits**. We want reviews across **all
  dimensions** — security, correctness, code quality, code style, performance — but only if the reader
  can still triage by importance (otherwise broadening = noise).

How do we make findings triage-able and broaden coverage without drowning the signal?

## Decision Drivers

- **Triage at a glance:** a ranked priority, with security visually unmistakable.
- **Cover all dimensions** (the user's ask) without turning reviews into noise.
- **Stable machine contract:** the agent emits the level/category via the validated tool payload
  ([ADR-0026](0026-native-review-agent.md)), never scraped text.
- **Minimal blast radius:** the finding shape is shared by runner → control plane → web.

## Considered Options

- **A. Priority `P0|P1|P2` + a `category` enum** (security, correctness, quality, style, performance).
- **B. Keep `error|warning|info`, add `category` only.**
- **C. Numeric 0–10 score.**

## Decision Outcome

Chosen option: **A** — every finding carries **`priority` (`P0` | `P1` | `P2`)** and **`category`**
(`security` | `correctness` | `quality` | `style` | `performance`; extensible). Rendering rules:

- **Priority** drives the badge rank: **P0 red, P1 amber, P2 neutral.**
- **`category: security` is always rendered red regardless of priority** (the user's explicit ask — a P2
  security note still reads as security-coloured), with the priority shown alongside.
- The agent **reviews all categories**, but the guidance instructs it to **reserve P0/P1 for real harm**
  and file lower-value style/quality observations as **P2** so they never crowd out blockers. Readers
  (and the dashboard) can filter by priority/category.

The mapping P0→error / P1→warning / P2→info is retained internally only as a back-compat shim while
older rows exist; the contract, struct, and UI move to priority+category.

### Consequences

- Good: findings are triage-able; security is unmistakable; coverage broadens to all dimensions
  **without** losing the signal because low-value items are pinned to P2.
- Good: the level is part of the validated `submit_findings` payload — no parsing fragility.
- Bad: a schema change touches the runner `Finding`, the control-plane `Finding`/validation, the JSONB
  rows, and the web `Review` type + badges; needs a migration/back-compat read for existing rows.
- Neutral: this consciously **reverses** the "exclude nits" stance of `DEFAULT_REVIEW_GUIDANCE`; the
  priority scale is what makes that safe. Stays within [ADR-0029](0029-focused-review-not-generic-runner.md)
  — still *review output*, not running steps.

## Pros and Cons of the Options

### A. Priority + category (chosen)
- Good: ranked triage + dimension labelling; security colour rule is trivial; extensible categories.
- Bad: two fields to populate and validate; migration for old rows.

### B. Category only, keep error/warning/info
- Good: smallest change.
- Bad: doesn't fix the core complaint — "info" is still meaningless for ranking severity.

### C. Numeric 0–10 score
- Good: fine-grained.
- Bad: false precision; models are inconsistent at calibrated numbers; harder for humans to scan than
  three named buckets.

## More Information

- Builds on [ADR-0026](0026-native-review-agent.md) (validated tool payload) and the finding format
  (#103). Output still validated + posted per [ADR-0022](0022-review-writeback-control-plane.md).
- Code: `services/agent-runner/src/review/{mod,parse}.rs`, `services/control-plane/src/review.rs`,
  `apps/web/components/runs/review-output.tsx`, `apps/web/lib/domain/tasks.ts`.
