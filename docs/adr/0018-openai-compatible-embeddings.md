# ADR-0018 — OpenAI-compatible API for all embeddings

| Field      | Value |
|------------|-------|
| Status     | Accepted |
| Date       | 2026-06-19 |
| Deciders   | @ssegning |
| Epic       | #5 (indexer + agent, slice 2) |

## Context

The pgvector indexer (slice 2 of epic #5) must embed code chunks for semantic search.
Two options exist:

1. **Local in-process model** — a bundled ONNX/llama.cpp runtime; self-contained but adds >1 GB to
   the container image, requires GPU or significant CPU budget, and diverges from the org's existing
   AI infrastructure.
2. **OpenAI-compatible HTTP API** — a remote `POST /v1/embeddings` call against any OpenAI-spec
   endpoint; no model weight in the image, and the prod infra already runs such an endpoint
   (eaig/core-gateway, same gateway LibreChat's RAG uses).

The organisation operates an Envoy AI Gateway (`eaig`, project `core-gateway`) that exposes an
OpenAI-compatible API for all LLM and embedding traffic.  LibreChat's RAG already uses it with
`text-embedding-3-small` (1536-dim).

## Decision

The indexer runner will embed code chunks **exclusively via an OpenAI-compatible `POST
{base}/v1/embeddings` endpoint**. No local model is bundled. All three config values are required
at runtime — there is no default for `EMBEDDINGS_MODEL` so a misconfigured Job fails loudly with a
named missing-variable error rather than silently using a wrong model.

| Env var              | Description                              | Prod value |
|----------------------|------------------------------------------|------------|
| `EMBEDDINGS_BASE_URL`| Base URL without trailing `/v1`          | `https://core-gateway-internal.envoy-gateway-system.svc.cluster.local` |
| `EMBEDDINGS_API_KEY` | Bearer key for the endpoint              | ESO secret `converse_openai_api_key` (remote ref `ai/camer/digital/prod/env`) |
| `EMBEDDINGS_MODEL`   | Model identifier (no default)            | `text-embedding-3-small` |

The `code_chunks` table's `embedding` column is typed `vector(1536)` — fixed at migration time to
match `text-embedding-3-small`.  Switching to a different-dimension model (e.g.
`qwen3-embedding-8b` at 4096 dims) requires a new migration and full re-indexing.

## Consequences

**Good**
- Image stays small (~70 MB Rust binary + git + ca-certificates; no model weights).
- Single embeddings path across all services matches the org's AI gateway strategy.
- Rate limiting, load balancing, model switching, and observability are handled by the gateway.
- Local / CI testing can point `EMBEDDINGS_BASE_URL` at a wiremock stub or a local
  OpenAI-compatible server (e.g. Ollama) without code changes.

**Trade-offs**
- Network hop to the embeddings endpoint adds latency per batch; mitigated by batching 32 chunks
  per request (`EMBED_BATCH_SIZE`).
- Indexing fails if the gateway is unavailable; accepted — the task system retries failed Jobs.
- Changing the embedding model mid-fleet requires a coordinated migration + re-indexing across all
  indexed repositories.
