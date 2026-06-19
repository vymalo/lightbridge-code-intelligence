# ADR-0020 — MCP servers are thin clients of the control-plane retrieval API

| Field      | Value |
|------------|-------|
| Status     | Accepted |
| Date       | 2026-06-19 |
| Deciders   | @ssegning |
| Epic       | #5 (indexer + agent, slice 4) |
| Builds on  | [ADR-0002](0002-rust-control-plane-trust-boundary.md), [ADR-0017](0017-agent-runner-control-plane-bootstrap.md), [ADR-0019](0019-graphify-cli-structural-graph.md) |

## Context

OpenCode (slice 5) investigates a repository through **MCP tools**: semantic search over pgvector
and structural queries over Neo4j (`docs/opencode-acp-mcp.md`). The MCP servers are stdio
subprocesses OpenCode spawns. The design doc's example config wires the graph server **directly to
Neo4j** (`NEO4J_URI`/`NEO4J_PASSWORD` in its env) and the vector server presumably to Postgres.

That would put datastore credentials inside the agent Job — an untrusted, per-task pod that clones
arbitrary repositories. Worse than for writes: the pgvector/Neo4j instances hold **every** repo's
index, so a read credential leaked from one repo's Job exposes all repos (cross-tenant).

## Decision

**The MCP servers are thin clients of the control plane's internal retrieval API (slice 4a), not of
the datastores.** They are built as two Rust binaries in the agent-runner crate, bundled in the same
image (`lightbridge-vector-mcp`, `lightbridge-graph-mcp`), using the official MCP SDK
(`rmcp`, stdio transport).

- They hold only the **runner bearer** (`AGENT_RUNNER_TOKEN`) and `TASK_ID`; the vector server also
  holds the embeddings key (`EMBEDDINGS_*`) to embed the query — an external service, already in the
  Job (ADR-0018). **No Postgres or Neo4j credentials** ever enter the Job.
- Every query is scoped **server-side** to the task's `(repository_id, commit_sha)` (resolved from
  the task, not the caller), so a Job can only read its own repo.
- Tools:
  - `lightbridge_vector_semantic_search(query, limit?)` — embeds the query, calls
    `POST /internal/tasks/{id}/search`.
  - `lightbridge_graph_find_symbol(term, limit?)` / `lightbridge_graph_get_callers(node_id, limit?)`
    — call `POST /internal/tasks/{id}/graph/query`.
- Tool errors are returned to the model as text (so it can retry/rephrase), not as transport
  failures.

GitHub read tooling is deferred: the runner already holds a short-lived installation token, so a
GitHub MCP can call GitHub directly (no new datastore boundary) — folded into a later slice with
write-back.

## Consequences

**Good**
- Consistent trust boundary: the Job holds **no** datastore credentials; reads are scoped per task.
- The MCP servers stay tiny — protocol glue over the slice-4a client; no DB drivers in them.
- Reuses `ControlPlaneClient` / `EmbeddingsClient`; ship in the existing combined image.

**Trade-offs**
- One extra network hop (MCP → control plane → datastore) vs a direct DB connection. Acceptable;
  the control plane is in-cluster and the queries are `LIMIT`-bounded.
- Diverges from `docs/opencode-acp-mcp.md`'s example config — that doc should be updated to drop the
  `NEO4J_*` env from the MCP block.

## Alternatives rejected

- **MCP servers connect directly to Postgres/Neo4j** (the doc's example) — puts cross-tenant-capable
  read credentials in an untrusted Job.
- **Implement MCP by hand** instead of `rmcp` — needless; `rmcp` is the official SDK and the stdio
  tool-server pattern is a few lines.
