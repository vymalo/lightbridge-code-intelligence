# Architecture Decision Records

This directory records the significant architectural decisions for Lightbridge Code Intelligence,
using the [MADR](https://adr.github.io/madr/) format.

## Process

- An ADR captures **one** decision: its context, the chosen option, and the consequences.
- ADRs are **immutable once accepted**. We do **not** edit the substance of an accepted ADR.
  If a decision changes, write a **new** ADR that **supersedes** the old one, and update the
  superseded ADR's status to `Superseded by ADR-NNNN`.
- Status values: `Proposed`, `Accepted`, `Rejected`, `Deprecated`, `Superseded by ADR-NNNN`.
- New ADRs start from [the template](0000-adr-template.md) and are numbered sequentially with a
  kebab-case title, e.g. `0014-some-decision.md`.
- Substantial proposals are often socialized first as an [RFC](../rfc/README.md); an accepted RFC
  typically yields one or more ADRs.

## Index

| # | Title | Status |
|---|---|---|
| [0000](0000-adr-template.md) | ADR template | — |
| [0001](0001-use-github-app.md) | Use a GitHub App (not a PAT-backed bot) | Accepted |
| [0002](0002-rust-control-plane-trust-boundary.md) | The Rust control plane owns the trust boundary | Accepted |
| [0003](0003-dual-retrieval-neo4j-pgvector.md) | Dual retrieval: Neo4j + pgvector are complementary | Accepted |
| [0004](0004-one-k8s-job-per-task.md) | One Kubernetes Job per task for execution isolation | Accepted |
| [0005](0005-cratestack-schema-first-control-plane.md) | Adopt cratestack (schema-first Rust) for the control plane | Accepted |
| [0006](0006-nextjs-app-router-web-ui.md) | Next.js (App Router) for the web UI | Accepted |
| [0007](0007-better-auth-rust-backend-plugin.md) | better-auth for web auth via a rust-backend plugin | Superseded by ADR-0014 |
| [0008](0008-adopt-ai-governance-framework.md) | Adopt the ADORSYS-GIS AI Governance framework | Accepted |
| [0009](0009-pnpm-turborepo-monorepo.md) | pnpm + Turborepo monorepo layout | Accepted |
| [0010](0010-graphify-treesitter-indexing-baseline.md) | Graphify + tree-sitter as the indexing baseline | Accepted |
| [0011](0011-engineering-practices-xp-lean-devops.md) | Engineering practices: XP + Lean + DevOps + shift-left | Accepted |
| [0012](0012-rfc-process-alongside-adrs.md) | RFC process alongside ADRs | Accepted |
| [0013](0013-local-dev-and-build-tooling.md) | Local dev & build tooling (just, xtask, compose, nextest, wiremock) | Accepted |
| [0014](0014-keycloak-oidc-resource-server.md) | Keycloak OIDC — web client + resource server | Accepted |
| [0015](0015-web-console-design-language.md) | Web console design language & component system (shadcn/ui) | Accepted |
| [0016](0016-dashboard-information-architecture.md) | Dashboard information architecture & task-run views | Accepted |
| [0017](0017-agent-runner-control-plane-bootstrap.md) | Agent runner bootstraps from the control plane (no App key in the Job) | Accepted |
| [0018](0018-openai-compatible-embeddings.md) | OpenAI-compatible API for all embeddings (eaig/core-gateway; no bundled model) | Accepted |
| [0019](0019-graphify-cli-structural-graph.md) | Graphify (bundled CLI) extracts the structural graph; control plane writes Neo4j | Accepted |
| [0020](0020-mcp-servers-via-control-plane.md) | MCP servers are thin clients of the control-plane retrieval API (no datastore creds in the Job) | Accepted |
| [0021](0021-opencode-headless-review-agent.md) | OpenCode (headless `run`) is the review agent; generated opencode.json wires eaig + MCP | Superseded by 0026 |
| [0022](0022-review-writeback-control-plane.md) | Control plane validates findings against the PR diff and posts the review (no GitHub creds in the Job) | Accepted |
| [0023](0023-db-backed-rbac.md) | Permission-based authz: the token carries a permissions list under a configurable claim (aud still verified); enforce per-capability; no roles/DB | Accepted |
| [0024](0024-web-console-redesign-v2.md) | Web console redesign v2 — richer surfaces, grouped nav + ⌘K, runs table, insights (within the ADR-0015 contract) | Proposed |
| [0025](0025-review-reuses-base-index.md) | Review reuses the base index instead of re-indexing every run (perf) | Accepted |
| [0026](0026-native-review-agent.md) | Native Rust review agent with structured tool calls; drop OpenCode (robust output + control tools) | Accepted |
| [0027](0027-daisyui-design-system.md) | Adopt daisyUI (dracula theme) as the component layer; supersede ADR-0015's bespoke component/token mechanism | Proposed |
| [0028](0028-agent-job-control-sidecar.md) | Agent Job = control sidecar + single configurable main container (closed built-in pipeline) | Proposed |
| [0029](0029-focused-review-not-generic-runner.md) | Scope boundary: a focused code-review system, not a generic step/CI runner (reject arbitrary steps, CRD/operator, workflow engines) | Proposed |
| [0030](0030-repo-review-config.md) | Repo-level review configuration (`.lightbridge-code-review.jsonc`) — extend understanding, not execution | Proposed |
| [0031](0031-review-skills-commands.md) | Custom review skills/commands via the repo config (named prompts, `@mention`-invokable) | Proposed |
