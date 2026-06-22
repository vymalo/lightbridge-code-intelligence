# agent-runner

The Rust binary that runs **inside each per-task Kubernetes Job**. It bootstraps from the control plane,
does the heavy repository work, and reports results back — holding no standing credentials.

## Lifecycle

1. **Bootstrap** ([ADR-0017](../../docs/adr/0017-agent-runner-control-plane-bootstrap.md)): the Job
   carries only its task id + the control-plane callback wiring. The runner calls
   `GET /internal/tasks/{id}` for its context plus a freshly-minted, short-lived installation token.
2. **Clone** the target repo at the head SHA using that token.
3. **Semantic index**: tree-sitter chunking → embeddings (OpenAI-compatible,
   [ADR-0018](../../docs/adr/0018-openai-compatible-embeddings.md)) → `code_chunks` (pgvector), posted to
   the control plane.
4. **Structural graph**: Graphify → the control plane writes Neo4j (best-effort / non-fatal,
   [ADR-0019](../../docs/adr/0019-graphify-cli-structural-graph.md)).
5. **Review**: the **native Rust agent loop** ([ADR-0026](../../docs/adr/0026-native-review-agent.md))
   reasons over the repo with the retrieval tools and **acts via mediated write tools as it goes**
   ([ADR-0037](../../docs/adr/0037-agent-acts-via-mediated-tools.md)) — `add_review_comment` (an inline
   finding), `add_comment` (a plain reply), `finish` (the verdict). The control plane buffers these and
   posts nothing until finalize, so a mid-run failure posts nothing. The agent's system prompt is
   **required operational config** (the ai-helm `config.reviewSystemPrompt`, mounted) — there is no
   built-in default; review fails closed without one. `REVIEW_AGENT=opencode` falls back to the legacy
   OpenCode subprocess (terminal JSON payload; retires with OpenCode/Bun, #140).
6. **Report** terminal status; on a clean finish the control plane flushes the buffer as one grouped
   review — validating findings against the PR diff
   ([ADR-0022](../../docs/adr/0022-review-writeback-control-plane.md)) and consolidating replies into a
   single comment.

The stage sequence lives in [`src/main.rs`](src/main.rs) (`run()`).

## Job kinds & index reuse

- **`index`** task (on repo approval): index only, no review.
- **`review`** task (on PR / `@mention`): a warm repo **reuses the base index** and skips re-indexing
  ([ADR-0025](../../docs/adr/0025-review-reuses-base-index.md)); a cold repo indexes first.

See [docs/jobs-and-lifecycle.md](../../docs/jobs-and-lifecycle.md).

## Security posture

No GitHub App key, no datastore credentials: retrieval tools are thin clients of the control-plane API
([ADR-0020](../../docs/adr/0020-mcp-servers-via-control-plane.md)). It holds only the short-lived install
token, the shared `AGENT_RUNNER_TOKEN`, the embeddings key, and a mounted internal-CA cert.

## Cancellation

Cooperative: the runner polls `GET /internal/tasks/{id}/status` (~10s) and exits promptly if the task was
cancelled upstream, complementing the dispatcher reaper deleting the Job (#116).

## Tests

`cargo nextest run -p agent-runner` — the control-plane contract is covered with **wiremock** (no cluster
needed).
