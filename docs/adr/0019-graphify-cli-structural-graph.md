# ADR-0019 — Graphify (standalone CLI) as the structural-graph extractor

| Field      | Value |
|------------|-------|
| Status     | Accepted |
| Date       | 2026-06-19 |
| Deciders   | @ssegning |
| Epic       | #5 (indexer + agent, slice 3) |
| Refines    | [ADR-0010](0010-graphify-treesitter-indexing-baseline.md), builds on [ADR-0003](0003-dual-retrieval-neo4j-pgvector.md) |

## Context

Slice 3 builds the **structural** half of dual retrieval (ADR-0003): a Neo4j graph of symbols and
their `contains` / `method` / `calls` relationships. ADR-0010 named "Graphify + tree-sitter" as the
indexing baseline but didn't pin *how* Graphify is integrated. Two facts forced a concrete decision:

1. **Slice 2 already shipped our own tree-sitter chunker** for the pgvector (semantic) path — we did
   not use Graphify there. For slice 3 we either build a *second* hand-rolled tree-sitter pass
   (symbols + call graph) or lean on Graphify for the graph.
2. **What Graphify actually is** (verified by installing `graphifyy==0.8.44` and running it): a
   Python CLI that extracts an AST→graph over **36 languages**, fully **headless with no API key**
   (`graphify extract <dir> --no-cluster`), emitting `graphify-out/graph.json` (nodes with
   `source_file` + start line; edges with `relation` ∈ {`contains`, `method`, `calls`, …}). It has
   **no embeddings / pgvector** capability, and the current release has **no direct Neo4j push** —
   it just produces `graph.json`.

## Decision

**Use Graphify as the structural-graph extractor, bundled into the agent-runner image; the runner
spawns it and the control plane owns the Neo4j write.**

- The runner image now bundles **Python + `graphifyy`** alongside the Rust binary (one image, built
  in stages so compilers don't ship to runtime).
- The runner spawns `graphify extract <checkout> --no-cluster` (no LLM — AST only), parses
  `graph.json`, and POSTs nodes+edges to **`POST /internal/tasks/{id}/graph`**.
- The **control plane** writes to Neo4j over Bolt (`neo4rs`), scoping every node/edge by
  `(repository_id, commit_sha)`. The untrusted per-task Job never holds Neo4j credentials — same
  trust boundary as chunk ingestion (ADR-0002).
- **Graphify does not replace the slice-2 chunker.** It has no embeddings; the semantic (pgvector)
  path stays with our tree-sitter chunker. The two indexers map cleanly onto dual retrieval:
  tree-sitter chunker → pgvector (semantic), Graphify → Neo4j (structural).

## Consequences

**Good**
- 36-language structural coverage for free, vs the 4 languages our chunker handles — and we avoid
  hand-rolling/maintaining a second tree-sitter symbol+call extractor.
- Headless, offline, deterministic AST extraction; no API key, no paid calls (the runner explicitly
  strips `*_API_KEY` env before spawning so a stray key can't trigger Graphify's optional LLM pass).
- Neo4j credentials stay in the control plane.

**Trade-offs**
- The runner image is larger (Python runtime + 36 grammar wheels). Accepted — the goal is broad
  language support in one image.
- We depend on Graphify's `graph.json` schema; pinned to `0.8.44` and guarded by a parser unit test
  on a captured sample. A future Graphify that pushes to Neo4j directly could simplify this, but we
  keep the write in the control plane regardless (trust boundary).
- Graph edges are AST-approximate (no semantic resolver). Acceptable as the baseline (ADR-0010);
  Graphify's `--mode deep` LLM inference is deliberately **off**.

## Alternatives rejected

- **Hand-roll a second tree-sitter pass for the graph** — duplicates Graphify for fewer languages
  and more maintenance.
- **Have Graphify also feed pgvector** — impossible; it produces no embeddings and its nodes carry
  no end-range/body to embed.
- **Let the runner write Neo4j directly** — would put Neo4j credentials in the untrusted Job.
