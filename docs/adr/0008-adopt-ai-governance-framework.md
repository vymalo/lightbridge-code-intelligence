# ADR-0008: Adopt the ADORSYS-GIS AI Governance framework

- **Status:** Accepted
- **Date:** 2026-06-18

## Context and Problem Statement

Lightbridge is built with substantial AI assistance and ships an AI agent that acts on
repositories. We need consistent governance over how AI is used in development and how changes are
reviewed and gated, rather than ad hoc per-author practices.

## Decision Drivers

- Consistent, auditable AI-usage practices across contributors
- A shared Definition of Ready / Definition of Done
- An enforceable PR gate rather than convention alone
- Reuse of an existing, maintained governance framework

## Considered Options

- **Adopt the ADORSYS-GIS AI Governance framework** — vendored templates plus a pinned caller
  workflow that enforces governance checks on pull requests.
- **Write our own lightweight policy** — tailored, but unmaintained and easy to let rot.
- **No formal governance** — fastest, but inconsistent and unauditable.

## Decision Outcome

Chosen option: **adopt the ADORSYS-GIS AI Governance framework**, with vendored templates and a
pinned caller workflow. It defines Definition of Ready/Done and AI usage declarations, and enforces
a governance gate on PRs.

### Consequences

- Good, because AI-usage practices and review gates are consistent and auditable.
- Good, because the framework is maintained externally; we pin and vendor for reproducibility.
- Bad, because contributors must satisfy the governance gate (DoR/DoD, AI declarations) on every PR.
- Neutral, because the framework ties into our [engineering practices](0011-engineering-practices-xp-lean-devops.md).
