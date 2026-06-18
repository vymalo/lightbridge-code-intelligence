# ADR-0003: Dual retrieval — Neo4j and pgvector are complementary

- **Status:** Accepted
- **Date:** 2026-06-18

## Context and Problem Statement

Good code review needs two different kinds of recall: *structural* (what calls this function, which
tests cover it, what imports what) and *semantic* (where is similar behavior implemented, which
docs discuss this feature). A single store handles one of these well and the other poorly.

## Decision Drivers

- Structure-heavy queries (call graphs, containment, test ownership, PR impact)
- Semantic-similarity queries (natural-language and code-to-code retrieval)
- Operational simplicity vs. retrieval quality

## Considered Options

- **Neo4j only** — strong on topology, weak as a semantic similarity store.
- **pgvector only** — easy operationally, but loses relationships and topology.
- **Neo4j + pgvector** — a graph store for structure plus a vector store for semantics.

## Decision Outcome

Chosen option: **Neo4j + pgvector**, used as complementary indexes rather than interchangeable
ones. Neo4j answers structure/graph questions; pgvector (HNSW by default) answers semantic ones.
The indexing pipeline writes to both.

### Consequences

- Good, because each query type is served by the store that does it best.
- Good, because retrieval quality is materially higher than either store alone.
- Bad, because we operate two data stores with two ingestion paths to keep in sync.
- Neutral, because the [indexing pipeline](../indexing-and-storage.md) is designed to fan out to
  both from a single parse pass.
