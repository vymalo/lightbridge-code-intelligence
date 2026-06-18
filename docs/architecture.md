# Architecture Overview

## System context

Lightbridge is a webhook-first GitHub App that turns mentions such as `@lightbridge` into review
or Q&A tasks. The Rust control plane receives the event, verifies and normalizes it, persists task
state in Postgres, and launches a task-specific Kubernetes Job. That Job runs OpenCode in a
constrained environment with MCP access to graph, vector, and GitHub tooling.

## Provided concept diagram

```mermaid
stateDiagram-v2
  [*] --> User
  User --> Bot[github] : @lightbridge do we need this feature according to docs?
  Bot[github] --> OurSystem : Webhook
  OurSystem --> DB : Create task object
  OurSystem --> K8S : Create Job for the task
  OurSystem --> Neo4J : Code indexed
  OurSystem --> PgVector : Code chunks indexed
  K8S --> K8SJob : Do the task
  K8SJob --> RustACPClient: main
  K8SJob --> OpenCode: sidecar
  RustACPClient --> OpenCode: acp
  OpenCode --> Neo4J : mcp
  OpenCode --> PgVector : mcp
```

## Refined flowchart

```mermaid
flowchart TD
  GH[GitHub App lightbridge] -->|Webhook| API[Rust Control Plane]

  API --> VERIFY[Verify signature and normalize event]
  VERIFY --> ROUTER[Route command and target]
  ROUTER --> PG[(Postgres)]
  ROUTER --> READY{Repo index ready?}

  READY -->|No| WAIT[Reply still indexing or queue waiting task]
  READY -->|Yes| JOB[Create Kubernetes Job]

  JOB --> POD[Agent Pod]
  POD --> ACP[Rust ACP client]
  POD --> OC[OpenCode]
  ACP -->|ACP| OC

  OC --> MCPGH[GitHub MCP]
  OC --> MCPG[Neo4j MCP]
  OC --> MCPV[pgvector MCP]

  MCPGH --> GHA[GitHub API]
  MCPG --> NEO[(Neo4j)]
  MCPV --> VEC[(pgvector)]

  OC --> RESULT[Structured review result]
  RESULT --> VALIDATE[Control-plane validation]
  VALIDATE --> POST[Comment review or check run]
  POST --> GH
```

## Review sequence

```mermaid
sequenceDiagram
  participant U as User
  participant G as GitHub
  participant R as Rust Control Plane
  participant P as Postgres
  participant K as Kubernetes
  participant J as Agent Job
  participant O as OpenCode
  participant N as Neo4j
  participant V as pgvector

  U->>G: @lightbridge review this PR
  G->>R: Webhook delivery
  R->>R: Verify signature + dedupe delivery
  R->>P: Create task
  alt Repo not ready
    R->>G: Comment: still indexing
  else Repo ready
    R->>K: Create Job
    K->>J: Start pod
    J->>O: Start ACP session
    O->>N: Graph queries via MCP
    O->>V: Semantic queries via MCP
    O-->>J: Structured review result
    J-->>R: Result payload
    R->>R: Validate result
    R->>G: Post review/comment/check
  end
```

## Design-option comparisons

| Topic | Option | Pros | Cons | Recommendation |
|---|---|---|---|---|
| Bot identity | GitHub App | Least privilege, webhooks, installation scoping | Slightly more setup | Use |
| Bot identity | PAT-backed bot account | Fast to prototype | Weak trust boundary, broad tokens | Avoid |
| Retrieval backend | Neo4j only | Fewer moving parts | Graph store not ideal as sole semantic store | Avoid for MVP |
| Retrieval backend | pgvector only | Easy operationally | Loses relationships and topology | Avoid for full design |
| Execution | Shared worker pool | Lower startup overhead | Weaker isolation, harder debugging | Optional later |
| Execution | Per-task Job | Isolation, cleanup, per-task creds | Startup latency | Use |
| GitHub output | Comment only | Simple | Harder to summarize status | Start here |
| GitHub output | Checks + comments | Rich UX | More moving parts | Add after MVP |

## Trust boundary

The agent can inspect, reason, and propose. The Rust control plane decides what gets persisted,
posted, retried, or rejected. See [ADR-0002](adr/0002-rust-control-plane-trust-boundary.md).

## Web & auth tier

A Next.js (App Router) web console lives under `apps/web`. It gives operators a UI over the
control plane: repository onboarding, task history, index status, and audit trails. The web app
uses [**better-auth**](adr/0007-better-auth-rust-backend-plugin.md) for session management.

Authentication (**authN**) is not implemented inside Next.js. It is delegated to Lightbridge's
**own standalone, portable Rust backend** — the control plane — via a custom better-auth plugin
named `rust-backend`. That plugin POSTs submitted credentials to
`${AUTH_BACKEND_URL}/auth/verify`; the Rust backend validates them against its `AuthUser` store
and returns a session principal. Because the backend is standalone and portable, the same authN
surface can be reused outside the web app.

```mermaid
flowchart LR
  B[Browser] -->|credentials| W[Next.js apps/web<br/>better-auth + rust-backend plugin]
  W -->|POST AUTH_BACKEND_URL/auth/verify| RC[Rust control plane]
  RC -->|verify against AuthUser store| RC
  RC -->|session principal| W
  W -->|session cookie| B
```

> **authN is NOT authZ.** The path above is *authentication only* — proving who a web user is.
> Gateway **authorization** (decides what a caller may do at the API edge) is a separate concern
> handled by **Envoy** together with **Authorino** and the standalone
> [`ADORSYS-GIS/lightbridge-authz`](https://github.com/ADORSYS-GIS/lightbridge-authz) component.
> `lightbridge-authz` is **not** this project's auth backend, and our better-auth Rust backend is
> **not** the gateway authorizer. Keep the two cleanly separated. See
> [ADR-0007](adr/0007-better-auth-rust-backend-plugin.md) and the
> [FAQ](faq.md#how-does-authentication-authn-differ-from-authorization-authz).

## Control-plane implementation note

The control plane is built in **Rust (Axum)** and is **schema-first via
[cratestack](adr/0005-cratestack-schema-first-control-plane.md)**. The single source of truth is
`services/control-plane/schema/control-plane.cstack`, from which cratestack is intended to generate
the Axum + SQLx server, typed clients, and policy enforcement.

Codegen wiring is a **follow-up**: until the cratestack 0.4.x grammar is pinned, hand-written types
in `services/control-plane/src/types.rs` mirror the `.cstack` schema so the modelling work is
captured and reviewable now. The `.cstack` file and the hand-written types must be kept in sync
until codegen is enabled. See [ADR-0005](adr/0005-cratestack-schema-first-control-plane.md).
