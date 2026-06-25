# Documentation Index

This directory contains the complete documentation set for Lightbridge Code Intelligence.

## Table of contents

### Core docs
- [Executive summary](executive-summary.md)
- [Architecture overview](architecture.md)
- [Components and data models](components-and-data-models.md)
- [GitHub App and Rust control plane](github-app-and-control-plane.md)
- [Jobs and task lifecycle](jobs-and-lifecycle.md) — the two job kinds, state machine, cancellation + purge (with diagrams)
- [Indexing and storage](indexing-and-storage.md)
- [OpenCode ACP and MCP integration](opencode-acp-mcp.md)
- [Kubernetes and deployment](kubernetes-deployment.md)
- [Security, observability, testing, rollout](security-observability-testing-rollout.md)
- [FAQ](faq.md)

### Run it
- [Local setup guide](local-setup.md) — compose deps, GitHub App + webhook proxy, manual trigger, multipass + k3s

### Decisions and process
- [Architecture Decision Records (ADRs)](adr/README.md)
- [Requests for Comments (RFCs)](rfc/README.md)

### Ways of working
- [Engineering practices](ways-of-working/engineering-practices.md)
- [OKRs](ways-of-working/okrs.md)

## Reading paths

### Stakeholder path
1. [README](../README.md)
2. [Executive summary](executive-summary.md)
3. [Architecture overview](architecture.md)
4. [FAQ](faq.md)

### Backend engineer path
1. [Architecture overview](architecture.md)
2. [Components and data models](components-and-data-models.md)
3. [GitHub App and Rust control plane](github-app-and-control-plane.md)
4. [Indexing and storage](indexing-and-storage.md)
5. [Jobs and task lifecycle](jobs-and-lifecycle.md)
6. The review agent — [ADR-0026](adr/0026-native-review-agent.md) (native loop) + [ADR-0020](adr/0020-mcp-servers-via-control-plane.md) (retrieval tools) + [ADR-0039](adr/0039-agent-llm-resilience-and-observability.md) (LLM resilience: timeout/retry/circuit-breaker/failover + structured logging). Prompt engineering (epic #177): [ADR-0047](adr/0047-review-prompt-grounding-and-uncertainty.md) (grounding & uncertainty — empty retrieval ≠ absence), [ADR-0048](adr/0048-review-prompt-structure-and-technique.md) (prompt structure & technique for the GLM model) + the [revised-prompt draft](drafts/review-system-prompt.md), [ADR-0049](adr/0049-eval-driven-reviewer-prompt-iteration.md) (eval-driven prompt iteration). Historical: [OpenCode ACP/MCP](opencode-acp-mcp.md).

### Platform engineer path
1. [Architecture overview](architecture.md)
2. [Kubernetes and deployment](kubernetes-deployment.md)
3. [Security, observability, testing, rollout](security-observability-testing-rollout.md) + [ADR-0046](adr/0046-observability-dashboard-deployment.md) (how the Grafana dashboards deploy; most read Postgres, not Prometheus)

### Web & auth path
1. [Architecture overview — Web & auth tier](architecture.md#web--auth-tier)
2. [ADR-0006: Next.js (App Router) for the web UI](adr/0006-nextjs-app-router-web-ui.md)
3. [ADR-0014: Keycloak OIDC — web client + control-plane resource server](adr/0014-keycloak-oidc-resource-server.md) (supersedes the better-auth/rust-backend idea in [ADR-0007](adr/0007-better-auth-rust-backend-plugin.md))
4. [ADR-0023: permission-based authz (permissions claim, per-capability)](adr/0023-db-backed-rbac.md)
5. [ADR-0027: daisyUI (dracula) design system](adr/0027-daisyui-design-system.md)
6. [FAQ — authN vs authZ](faq.md#how-does-authentication-authn-differ-from-authorization-authz)

## Design principles

- GitHub App, not a PAT-backed bot
- Rust control plane owns trust boundaries
- Graph + vector retrieval are complementary
- Agent execution is isolated per task
- All write actions are controller-validated
- Security posture depends on trust level of source branch / fork
- Authentication (authN) is **Keycloak OIDC** — the web console is an OIDC client and the control
  plane a resource server ([ADR-0014](adr/0014-keycloak-oidc-resource-server.md), which supersedes the
  earlier better-auth/rust-backend plugin idea in ADR-0007). Authorization (authZ) is
  **permission-based**: the token carries a `permissions` list under a configurable claim, enforced
  per-capability ([ADR-0023](adr/0023-db-backed-rbac.md))
