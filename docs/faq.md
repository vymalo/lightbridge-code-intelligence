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

## My repo was indexed once but never again — why?

The baseline index runs once when the repo is approved. After that it is kept fresh by a
**push-driven re-index**: every push to the **default branch** (e.g. a merged PR) queues a new
`index` task. If indexing only ever ran once and now returns stale/0-hit results on new code, the
cause is almost always that the **GitHub App is not subscribed to the `push` event**. Add the *Push*
subscription (it needs only `Contents: Read`, already granted) — see
[the GitHub App setup](github-app-and-control-plane.md#recommended-webhook-subscriptions). The
control plane's `handle_push` then re-indexes on each default-branch move (deduped against an
in-flight index).

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

- **Authentication (authN)** answers *"who is this web user?"* Login is owned by **Keycloak** (the
  OIDC provider) — we manage no credentials. The Next.js web app (`apps/web`) is an
  **OIDC client** (Authorization-Code + PKCE) that stores the access token in an httpOnly cookie;
  the Rust control plane is a pure **resource server** that validates that JWT against Keycloak's
  JWKS. See [ADR-0014](adr/0014-keycloak-oidc-resource-server.md).
- **Authorization (authZ)** answers *"is this caller allowed to do this?"* at the gateway edge. It
  is handled by **Envoy + Authorino** together with the separate
  [`ADORSYS-GIS/lightbridge-authz`](https://github.com/ADORSYS-GIS/lightbridge-authz) component.
  `lightbridge-authz` is **not** this project's auth backend, and our OIDC resource server is
  **not** the gateway authorizer.

In short: Keycloak + our OIDC client/resource server = authN (login). Envoy/Authorino/lightbridge-authz
= authZ (access control at the gateway). They do not overlap, though both validate the same
Keycloak-issued JWTs.

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
