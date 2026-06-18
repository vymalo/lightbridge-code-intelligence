# ADR-0013: Local dev & build tooling

- **Status:** Accepted
- **Date:** 2026-06-18

## Context and Problem Statement

The monorepo ([ADR-0009](0009-pnpm-turborepo-monorepo.md)) mixes TypeScript and Rust and depends on
local data services (Postgres+pgvector, Neo4j). Contributors need a consistent, low-friction way to
set up, run, test, and gate changes locally — across both languages — without memorizing disparate
commands.

## Decision Drivers

- One memorable human-facing entrypoint
- Reproducible local data plane
- Fast Rust test feedback and reliable HTTP mocking
- A path to closer-to-prod local testing without mandating it

## Considered Options

- **`just` + `cargo xtask` + docker compose + cargo-nextest + wiremock** — `just` as the task
  entrypoint, `cargo xtask` for heavier Rust automation, docker compose for Postgres+pgvector and
  Neo4j, `cargo-nextest` as the test runner, `wiremock` for HTTP mocking.
- **Make + shell scripts** — ubiquitous, but clumsy across the JS/Rust split.
- **Per-tool native commands only** — no wrapper, but high cognitive load.

## Decision Outcome

Chosen option: **`just` (task entrypoint) + `cargo xtask` (Rust automation) + docker compose
(Postgres+pgvector, Neo4j) + cargo-nextest (test runner) + wiremock (HTTP mocking).** `just setup`,
`just up`, `just dev`, `just lint`, and `just test` are the everyday recipes.

**multipass + k3s is recorded as a TENTATIVE local-cluster option** (`just k3s-up` / `just k3s-down`)
for closer-to-prod testing. It is not required for everyday work and may change.

### Consequences

- Good, because contributors learn one set of `just` recipes for both languages.
- Good, because the local data plane and test/mocking story are reproducible.
- Bad, because contributors must install several tools (just, docker, Rust, pnpm, optionally
  multipass/nextest).
- Neutral, because the multipass+k3s path is explicitly tentative and may be replaced.
