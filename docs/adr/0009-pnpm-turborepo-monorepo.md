# ADR-0009: pnpm + Turborepo monorepo layout

- **Status:** Accepted
- **Date:** 2026-06-18

## Context and Problem Statement

Lightbridge spans a TypeScript web app, shared TypeScript packages, and Rust services. We need a
repository layout and tooling that share configuration, cache builds/tests, and keep cross-cutting
changes atomic — across both the JS/TS and Rust sides.

## Decision Drivers

- Atomic cross-package and cross-language changes
- Shared tooling and config (TypeScript, Biome)
- Fast, cached builds and tests
- A clean home for Rust automation

## Considered Options

- **pnpm workspaces + Turborepo, plus a Cargo `xtask`** — one repo with `apps/*`, `packages/*`,
  `services/*`, and Rust automation via `cargo xtask`.
- **Polyrepo** — independent repos per component; simpler per-repo, painful for cross-cutting work.
- **Nx / other monorepo tools** — capable, but heavier than needed here.

## Decision Outcome

Chosen option: **pnpm + Turborepo monorepo** with the layout `apps/*` (e.g. `apps/web`),
`packages/*` (e.g. `packages/auth`, `packages/tsconfig`), `services/*` (e.g.
`services/control-plane`), and a Cargo `xtask` crate for Rust automation. `just` is the single
human-facing task entrypoint ([ADR-0013](0013-local-dev-and-build-tooling.md)).

### Consequences

- Good, because cross-cutting changes land in one atomic commit/PR.
- Good, because Turborepo caches builds/tests and shared configs reduce drift.
- Bad, because mixing pnpm/Turborepo with Cargo requires bridging tooling (`just`, `xtask`).
- Neutral, because deployment manifests will live under `deploy/` (planned).
