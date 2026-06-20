# Documentation Index

This directory contains the complete documentation set for Lightbridge Code Intelligence.

## Table of contents

### Core docs
- [Executive summary](executive-summary.md)
- [Architecture overview](architecture.md)
- [Components and data models](components-and-data-models.md)
- [GitHub App and Rust control plane](github-app-and-control-plane.md)
- [Indexing and storage](indexing-and-storage.md)
- [OpenCode ACP and MCP integration](opencode-acp-mcp.md)
- [Kubernetes and deployment](kubernetes-deployment.md)
- [Security, observability, testing, rollout](security-observability-testing-rollout.md)
- [FAQ](faq.md)

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
5. [OpenCode ACP and MCP integration](opencode-acp-mcp.md)

### Platform engineer path
1. [Architecture overview](architecture.md)
2. [Kubernetes and deployment](kubernetes-deployment.md)
3. [Security, observability, testing, rollout](security-observability-testing-rollout.md)

### Web & auth path
1. [Architecture overview — Web & auth tier](architecture.md#web--auth-tier)
2. [ADR-0006: Next.js (App Router) for the web UI](adr/0006-nextjs-app-router-web-ui.md)
3. [ADR-0007: better-auth with a rust-backend delegation plugin](adr/0007-better-auth-rust-backend-plugin.md) (superseded by ADR-0014)
4. [ADR-0014: Keycloak OIDC — web client + control-plane resource server](adr/0014-keycloak-oidc-resource-server.md)
5. [FAQ — authN vs authZ](faq.md#how-does-authentication-authn-differ-from-authorization-authz)

## Design principles

- GitHub App, not a PAT-backed bot
- Rust control plane owns trust boundaries
- Graph + vector retrieval are complementary
- Agent execution is isolated per task
- All write actions are controller-validated
- Security posture depends on trust level of source branch / fork
- Authentication (authN) is delegated to our own portable Rust backend; authorization (authZ)
  at the gateway is a separate concern (Envoy/Authorino + `lightbridge-authz`)

<!-- graph smoke 2179600 -->
