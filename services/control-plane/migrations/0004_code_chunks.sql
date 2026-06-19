-- Enable pgvector. Idempotent; must run before any `vector` column is created.
CREATE EXTENSION IF NOT EXISTS vector;

-- Semantic code chunks produced by the indexer (epic #5, slice 2). One row per semantic unit
-- (function, class, impl block, …) or windowed fallback. The embedding is fixed at 1536 dimensions
-- matching text-embedding-3-small (ADR-0018); changing to a different-dimension model requires a
-- new migration and full re-indexing.
CREATE TABLE IF NOT EXISTS code_chunks (
    id            BIGSERIAL PRIMARY KEY,
    repository_id BIGINT  NOT NULL REFERENCES repositories (id) ON DELETE CASCADE,
    commit_sha    TEXT    NOT NULL,
    file_path     TEXT    NOT NULL,
    language      TEXT    NOT NULL,
    chunk_type    TEXT    NOT NULL,
    symbol_name   TEXT,
    start_line    INT     NOT NULL,
    end_line      INT     NOT NULL,
    content       TEXT    NOT NULL,
    embedding     vector(1536) NOT NULL,
    created_at    TIMESTAMPTZ  NOT NULL DEFAULT now(),
    UNIQUE (repository_id, commit_sha, file_path, start_line, end_line)
);

-- Approximate nearest-neighbour search (cosine; text-embedding-3-small normalises to unit length).
CREATE INDEX IF NOT EXISTS code_chunks_embedding_idx
    ON code_chunks USING hnsw (embedding vector_cosine_ops);

-- Retrieval by commit: enumerate all chunks indexed for a given snapshot.
CREATE INDEX IF NOT EXISTS code_chunks_commit_idx
    ON code_chunks (repository_id, commit_sha);
