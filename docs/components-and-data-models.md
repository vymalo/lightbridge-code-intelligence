# Components and Data Models

## Component responsibilities

| Component | Responsibilities | Does not own |
|---|---|---|
| GitHub App | Identity, permissions, webhooks | Business logic |
| Rust Control Plane | Webhook verification, task creation, idempotency, policy, GitHub writes | Deep repo reasoning |
| Indexer Job | Clone repo, parse code, build graph/vector indexes | GitHub write actions |
| OpenCode Agent Job | Investigation and review reasoning | Trust decisions, durable writes |
| Postgres | Source-of-truth metadata | Semantic retrieval |
| Neo4j | Structural code graph | System-of-record task state |
| pgvector | Chunk embeddings and ANN search | Code topology |

## State enums

```rust
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, sqlx::Type)]
#[sqlx(type_name = "repo_index_status", rename_all = "snake_case")]
pub enum RepoIndexStatus {
    Pending,
    Running,
    Ready,
    Failed,
    Stale,
    Disabled,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, sqlx::Type)]
#[sqlx(type_name = "task_status", rename_all = "snake_case")]
pub enum TaskStatus {
    Received,
    WaitingForIndex,
    Queued,
    Running,
    PostingResult,
    Succeeded,
    Failed,
    TimedOut,
    Cancelled,
}
```

## Postgres schema

```sql
CREATE TABLE github_deliveries (
  delivery_id        text PRIMARY KEY,
  event_name         text NOT NULL,
  installation_id    bigint,
  repository_id      bigint,
  received_at        timestamptz NOT NULL DEFAULT now(),
  payload_json       jsonb NOT NULL
);

CREATE TABLE repositories (
  id                 bigserial PRIMARY KEY,
  github_repo_id     bigint UNIQUE NOT NULL,
  owner              text NOT NULL,
  name               text NOT NULL,
  default_branch     text NOT NULL,
  active             boolean NOT NULL DEFAULT true
);

CREATE TABLE repo_indexes (
  id                 bigserial PRIMARY KEY,
  repository_id      bigint NOT NULL REFERENCES repositories(id),
  branch             text NOT NULL,
  commit_sha         text NOT NULL,
  graph_version      text NOT NULL,
  vector_version     text NOT NULL,
  status             text NOT NULL,
  started_at         timestamptz,
  completed_at       timestamptz,
  UNIQUE(repository_id, branch, commit_sha, graph_version, vector_version)
);

CREATE TABLE tasks (
  id                 uuid PRIMARY KEY,
  repository_id      bigint NOT NULL REFERENCES repositories(id),
  installation_id    bigint NOT NULL,
  github_delivery_id text NOT NULL REFERENCES github_deliveries(delivery_id),
  target_type        text NOT NULL,
  target_id          bigint NOT NULL,
  command_text       text NOT NULL,
  base_sha           text,
  head_sha           text,
  status             text NOT NULL,
  priority           integer NOT NULL DEFAULT 100,
  created_at         timestamptz NOT NULL DEFAULT now(),
  started_at         timestamptz,
  completed_at       timestamptz
);

CREATE INDEX tasks_repo_status_idx ON tasks(repository_id, status);
CREATE INDEX tasks_created_at_idx ON tasks(created_at DESC);
```

> The Postgres schema above is mirrored by the schema-first definition in
> `services/control-plane/schema/control-plane.cstack`. See
> [ADR-0005](adr/0005-cratestack-schema-first-control-plane.md) for how cratestack generates the
> server, clients, and policies from that file (codegen wiring deferred), and why hand-written
> types in `services/control-plane/src/types.rs` mirror it in the meantime.

## Rust structs

```rust
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Task {
    pub id: uuid::Uuid,
    pub repository_id: i64,
    pub installation_id: i64,
    pub github_delivery_id: String,
    pub target_type: String,
    pub target_id: i64,
    pub command_text: String,
    pub base_sha: Option<String>,
    pub head_sha: Option<String>,
    pub status: String,
    pub priority: i32,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RepoIndex {
    pub repository_id: i64,
    pub branch: String,
    pub commit_sha: String,
    pub graph_version: String,
    pub vector_version: String,
    pub status: String,
}
```

## Neo4j schema

### Node labels
- `Repository`
- `Branch`
- `Commit`
- `File`
- `Module`
- `Symbol`
- `Function`
- `Class`
- `Struct`
- `Enum`
- `Trait`
- `Test`
- `DocChunk`
- `PullRequest`

### Relationships
- `(:Repository)-[:HAS_BRANCH]->(:Branch)`
- `(:Branch)-[:POINTS_TO]->(:Commit)`
- `(:Commit)-[:CONTAINS]->(:File)`
- `(:File)-[:DEFINES]->(:Symbol)`
- `(:Function)-[:CALLS]->(:Function)`
- `(:File)-[:IMPORTS]->(:File)`
- `(:Test)-[:TESTS]->(:Function)`
- `(:PullRequest)-[:TOUCHES]->(:File)`

### Constraints

```cypher
CREATE CONSTRAINT repo_key IF NOT EXISTS
FOR (r:Repository) REQUIRE (r.repo_id) IS UNIQUE;

CREATE CONSTRAINT file_key IF NOT EXISTS
FOR (f:File) REQUIRE (f.repo_id, f.commit_sha, f.path) IS UNIQUE;

CREATE CONSTRAINT symbol_key IF NOT EXISTS
FOR (s:Symbol) REQUIRE (s.repo_id, s.commit_sha, s.fqn) IS UNIQUE;
```

## pgvector chunk schema

```sql
CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE code_chunks (
  id                 bigserial PRIMARY KEY,
  repository_id      bigint NOT NULL REFERENCES repositories(id),
  commit_sha         text NOT NULL,
  path               text NOT NULL,
  language           text NOT NULL,
  symbol_name        text,
  chunk_type         text NOT NULL,
  start_line         integer NOT NULL,
  end_line           integer NOT NULL,
  content            text NOT NULL,
  embedding          vector(1536) NOT NULL,
  metadata_json      jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX code_chunks_repo_sha_path_idx
  ON code_chunks(repository_id, commit_sha, path);

CREATE INDEX code_chunks_embedding_hnsw_idx
  ON code_chunks USING hnsw (embedding vector_cosine_ops);
```

## Example retrieval query

```sql
SELECT id, path, symbol_name, start_line, end_line, 1 - (embedding <=> $1) AS similarity
FROM code_chunks
WHERE repository_id = $2
  AND commit_sha = $3
ORDER BY embedding <=> $1
LIMIT 20;
```
