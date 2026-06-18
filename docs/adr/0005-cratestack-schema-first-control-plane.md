# ADR-0005: Adopt cratestack (schema-first Rust) for the control plane

- **Status:** Accepted
- **Date:** 2026-06-18

## Context and Problem Statement

The Rust control plane exposes an HTTP API over Postgres, with typed clients (consumed by the web
app) and policy enforcement on procedures. Hand-writing the server, the SQLx layer, the client
types, and the policy checks separately invites drift between them. We want one source of truth.

## Decision Drivers

- Single source of truth for schema, routes, clients, and policies
- Reduced drift between server, database, and consumers
- Avoiding premature lock-in to an unstable codegen grammar

## Considered Options

- **cratestack (schema-first)** — define models, procedures, and policies in
  `services/control-plane/schema/control-plane.cstack`; generate Axum + SQLx server, typed clients,
  and policy enforcement.
- **Hand-written Axum + SQLx** — full control, but every layer maintained and synchronized by hand.

## Decision Outcome

Chosen option: **cratestack**, with **codegen wiring deferred** until the cratestack **0.4.x**
grammar is pinned. Until then, hand-written types in `services/control-plane/src/types.rs` mirror
the `.cstack` schema so the modelling work is captured and reviewable now. The `.cstack` file is
the intended source of truth; the hand-written types must be kept in sync with it until codegen is
enabled.

### Consequences

- Good, because the schema is captured and reviewable today without betting on an unstable grammar.
- Good, because once pinned, codegen removes the manual server/client/policy synchronization burden.
- Bad, because until codegen is wired, `.cstack` and `types.rs` must be kept in sync by hand.
- Neutral, because banking-grade primitives (idempotency keys, optimistic locking, audit logging)
  are a follow-up once the grammar is fixed.
