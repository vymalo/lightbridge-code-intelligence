# ADR-0012: RFC process alongside ADRs

- **Status:** Accepted
- **Date:** 2026-06-18

## Context and Problem Statement

ADRs record decisions that have already been made, tersely. But some changes are substantial enough
that they need to be *proposed and socialized* — with motivation, design detail, drawbacks, and
alternatives — before a decision is reached. ADRs alone do not provide that deliberation space.

## Decision Drivers

- A place to propose and discuss substantial changes before deciding
- A clear handoff from proposal to recorded decision
- Lightweight enough not to slow small changes

## Considered Options

- **RFCs alongside ADRs** — RFCs (Rust-RFC-style) propose and socialize substantial changes; an
  accepted RFC yields one or more ADRs that record the resulting decisions.
- **ADRs only** — simpler, but no structured space for proposing large changes.
- **RFCs only** — loses the terse, immutable decision log that ADRs provide.

## Decision Outcome

Chosen option: **run an RFC process alongside ADRs.** RFCs live in [`docs/rfc/`](../rfc/README.md)
with lifecycle Draft → Proposed → Accepted/Rejected. An accepted RFC produces one or more ADRs. Small
or obvious decisions can go straight to an ADR without an RFC.

### Consequences

- Good, because substantial changes get deliberate, reviewable proposals before they are decided.
- Good, because the ADR log stays terse and immutable while RFCs hold the discussion.
- Bad, because contributors must judge when a change warrants an RFC vs. a direct ADR.
- Neutral, because the two artifacts are explicitly linked (RFC → resulting ADRs).
