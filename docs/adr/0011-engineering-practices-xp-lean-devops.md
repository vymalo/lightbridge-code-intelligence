# ADR-0011: Engineering practices — XP + Lean + DevOps + shift-left

- **Status:** Accepted
- **Date:** 2026-06-18

## Context and Problem Statement

The team needs an explicit, shared delivery model so practices (testing, review, release cadence,
ownership) are consistent rather than per-author. Lightbridge is a small system with a high quality
bar (it acts on other people's repositories), so quality and feedback speed matter.

## Decision Drivers

- Fast, reliable feedback loops
- Built-in quality rather than late QA
- Clear ownership of running services
- Alignment with the AI Governance gate ([ADR-0008](0008-adopt-ai-governance-framework.md))

## Considered Options

- **XP + Lean + DevOps with shift-left** — pairing, TDD, CI, small releases, collective ownership;
  eliminate waste and build quality in; you-build-it-you-run-it with automation and observability;
  testing/security/review pushed early.
- **Ad hoc / no explicit model** — flexible, but inconsistent and hard to onboard into.
- **Heavyweight stage-gated process** — predictable, but slow feedback and high overhead.

## Decision Outcome

Chosen option: **XP + Lean + DevOps + shift-left** as the delivery model, documented in
[ways-of-working/engineering-practices.md](../ways-of-working/engineering-practices.md). Quality
gates (`just lint`, `just test`, clippy, Biome, `cargo nextest`, wiremock) run locally before push
and again in CI; the governance gate enforces DoR/DoD on PRs.

### Consequences

- Good, because defects and security issues are caught early and cheaply.
- Good, because ownership and cadence are explicit and easy to onboard into.
- Bad, because the practices (pairing, TDD, pre-push gates) require discipline and buy-in.
- Neutral, because these practices are tied to the tooling in
  [ADR-0013](0013-local-dev-and-build-tooling.md).
