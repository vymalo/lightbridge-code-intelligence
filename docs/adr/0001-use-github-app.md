# ADR-0001: Use a GitHub App (not a PAT-backed bot) for GitHub integration

- **Status:** Accepted
- **Date:** 2026-06-18

## Context and Problem Statement

Lightbridge must read repositories, receive events, and post reviews/comments back to GitHub. The
integration identity can either be a GitHub App or a bot user authenticated with a personal access
token (PAT). The identity choice determines the permission model, how events arrive, and the blast
radius if a credential leaks.

## Decision Drivers

- Least privilege and per-installation scoping
- Native, verifiable event delivery (webhooks)
- Short-lived, narrowly scoped credentials
- Clear auditability of automated actions

## Considered Options

- **GitHub App** — fine-grained permissions, installation-scoped, webhook-native, short-lived
  installation access tokens (1-hour lifetime).
- **PAT-backed bot account** — fast to prototype, but broad long-lived tokens and a weak trust
  boundary.

## Decision Outcome

Chosen option: **GitHub App**. It gives the strongest permission model, native webhook delivery
with verifiable signatures, and short-lived installation tokens minted per task — keeping durable
broad credentials out of the system.

### Consequences

- Good, because permissions are minimized and scoped per installation.
- Good, because installation tokens are short-lived and minted per task, limiting leak impact.
- Bad, because initial setup (App registration, key management) is more involved than a PAT.
- Neutral, because the private key and webhook secret must be stored as Kubernetes Secrets and
  never handed to agent pods (see [ADR-0002](0002-rust-control-plane-trust-boundary.md)).
