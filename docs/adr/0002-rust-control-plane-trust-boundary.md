# ADR-0002: The Rust control plane owns the trust boundary

- **Status:** Accepted
- **Date:** 2026-06-18

## Context and Problem Statement

The system runs an LLM agent (OpenCode) that reasons over repository content and can suggest review
actions. If the agent could write directly to GitHub or mutate persistent state, a prompt-injected
or mistaken agent could take harmful actions. We need a clear boundary between *proposing* and
*acting*.

## Decision Drivers

- Prompt-injection and over-privileged-execution risk
- Need for auditability of every write
- Idempotency and policy enforcement on mutations
- Keeping secrets (GitHub App key) away from agent code

## Considered Options

- **Control plane owns all writes** — the agent returns a structured proposal; the Rust control
  plane validates and performs every persistent or GitHub-facing action.
- **Agent writes directly** — fewer hops, but the agent holds write credentials and decision
  authority.

## Decision Outcome

Chosen option: **the Rust control plane owns the trust boundary.** The agent may read, query the
graph/vector stores, and prepare structured findings, but it never posts to GitHub or persists
state. The control plane validates line references, deduplicates, applies policy, mints credentials,
and performs all writes.

### Consequences

- Good, because injected or erroneous agent output cannot directly mutate GitHub or the database.
- Good, because every write goes through one auditable, policy-enforcing path.
- Bad, because it adds a validation/serialization hop and a defined result schema to maintain.
- Neutral, because agent pods receive read-mostly credentials only (see
  [ADR-0004](0004-one-k8s-job-per-task.md)).
