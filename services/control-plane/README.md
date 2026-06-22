# control-plane

The Rust (Axum) **control plane** — the system's trust boundary
([ADR-0002](../../docs/adr/0002-rust-control-plane-trust-boundary.md)). It owns the GitHub App private
key, mints short-lived per-task installation tokens, persists task state in Postgres, and dispatches one
Kubernetes Job per task.

## Roles

One binary, selected by the first arg (default `serve`):

- **`serve`** — the HTTP API: GitHub webhooks (HMAC-verified, delivery-id deduped), the internal
  task/runner API (`/internal/tasks/{id}` — context + minted token + status/chunks/review/graph/search,
  [ADR-0017](../../docs/adr/0017-agent-runner-control-plane-bootstrap.md)), the dashboard API
  (`/tasks`, `/repositories`, `/admin/*`, `/me`, `/tasks/{id}/review|cancel`), and `/metrics`.
  Verifies OIDC JWTs and enforces **permission-based authz**
  ([ADR-0023](../../docs/adr/0023-db-backed-rbac.md)).
- **`dispatcher`** — the queue consumer: claims `queued` tasks (`SELECT … FOR UPDATE SKIP LOCKED`),
  launches the agent Job (`k8s.rs`), and runs the **reaper** (deletes cancelled/finished Jobs) and the
  **purge reconciler** (removes data for denied/removed repos). See RFC-0001.

```bash
cargo run -p control-plane              # serve (default)
cargo run -p control-plane dispatcher   # dispatcher
```

## What it talks to

- **Postgres (+pgvector)** — the work queue, repositories, reviews, and the `code_chunks` semantic index.
  Schema via hand-written SQLx migrations (cratestack deferred,
  [ADR-0005](../../docs/adr/0005-cratestack-schema-first-control-plane.md)).
- **Neo4j** — the structural graph (the control plane writes;
  [ADR-0019](../../docs/adr/0019-graphify-cli-structural-graph.md)).
- **GitHub** — webhooks in; minted installation tokens used by runners to clone; validated review
  write-back ([ADR-0022](../../docs/adr/0022-review-writeback-control-plane.md)).
- **Kubernetes** — builds + launches the per-task Job (`k8s.rs::job_manifest`); one Job per task
  ([ADR-0004](../../docs/adr/0004-one-k8s-job-per-task.md)).

## Trust boundary

The GitHub App key never leaves this service. A Job receives only a short-lived (~1h) installation token
plus the shared `AGENT_RUNNER_TOKEN` and embeddings key — never datastore credentials
([ADR-0020](../../docs/adr/0020-mcp-servers-via-control-plane.md)). The runner *proposes*; the control
plane *validates and writes back*.

## Configuration

Read from `/etc/lightbridge/control-plane.json` (mounted ConfigMap) or env. Key knobs: `agent.*`
(runner image, namespace, service account, resources, deadline, CA secret), `dispatcher.*` (lease /
poll / reap cadences), the OIDC issuer/audience + `PERMISSIONS_CLAIM`, and `AGENT_RUNNER_TOKEN`. See
[`src/config.rs`](src/config.rs) and [docs/kubernetes-deployment.md](../../docs/kubernetes-deployment.md).

## Tests

`cargo nextest run -p control-plane` (`#[sqlx::test]` against a throwaway Postgres). The runner↔control
contract is tested on both sides (wiremock in the runner, `#[sqlx::test]` here).
