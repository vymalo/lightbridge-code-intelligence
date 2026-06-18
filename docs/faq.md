# FAQ

## What is Lightbridge?

Lightbridge is a GitHub App that helps with pull request reviews and repository questions by using
repository-aware retrieval instead of looking only at the current diff.

## Does Lightbridge replace human reviewers?

No. Lightbridge augments human review. Code owners, maintainers, and domain experts remain the
final decision makers.

## Why not just use a simple AI bot?

Because repository context matters. A good review often depends on existing abstractions, nearby
tests, related docs, and call paths.

## Why use both Neo4j and pgvector?

They answer different questions.

- Neo4j answers structure-heavy questions such as "what calls this?" or "which tests target this
  symbol?"
- pgvector answers semantic questions such as "where is behavior like this already implemented?" or
  "which docs talk about this feature?"

See [ADR-0003](adr/0003-dual-retrieval-neo4j-pgvector.md).

## Why use a GitHub App instead of a personal access token?

A GitHub App gives a cleaner permission model, installation scoping, webhook integration, and
short-lived installation tokens. See [ADR-0001](adr/0001-use-github-app.md).

## What happens when a repository is new?

Lightbridge first builds a baseline index. During that time, it can reply that indexing is still in
progress.

## Can Lightbridge review PRs from forks?

Yes, but the execution profile should be stricter. For untrusted input, disable unnecessary
shell/network capabilities and keep write actions tightly controlled.

## Can Lightbridge write code changes automatically?

Not by default. The agent proposes findings, while the Rust control plane validates any write-back
action before posting it to GitHub. See [ADR-0002](adr/0002-rust-control-plane-trust-boundary.md).

## What is the main security concern?

Prompt injection and over-privileged execution. The design addresses this by isolating tasks in
Kubernetes, using narrow MCP profiles, verifying GitHub events, and keeping write authority in the
control plane.

## How does authentication (authN) differ from authorization (authZ)?

They are deliberately separate planes:

- **Authentication (authN)** answers *"who is this web user?"* It is handled by the Next.js web app
  (`apps/web`) using **better-auth**, with a custom `rust-backend` plugin that POSTs credentials to
  **our own standalone, portable Rust backend** (the control plane) at
  `${AUTH_BACKEND_URL}/auth/verify`. This Rust backend *is* part of this project. See
  [ADR-0007](adr/0007-better-auth-rust-backend-plugin.md).
- **Authorization (authZ)** answers *"is this caller allowed to do this?"* at the gateway edge. It
  is handled by **Envoy + Authorino** together with the separate
  [`ADORSYS-GIS/lightbridge-authz`](https://github.com/ADORSYS-GIS/lightbridge-authz) component.
  `lightbridge-authz` is **not** this project's auth backend, and our better-auth Rust backend is
  **not** the gateway authorizer.

In short: better-auth + our Rust backend = authN (login). Envoy/Authorino/lightbridge-authz = authZ
(access control at the gateway). They do not overlap.

## What is the MVP?

A GitHub App that:
- receives `@lightbridge` webhook events
- queues tasks
- replies "still indexing" when needed
- runs isolated agent jobs
- posts structured comments back to GitHub

## How should teams adopt it?

Start with one or two internal repositories, comment-only mode, and explicit success criteria such
as review usefulness, false-positive rate, and turnaround time.
