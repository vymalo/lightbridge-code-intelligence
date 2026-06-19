-- Switch the embedding column to 4096 dimensions for `qwen3-embedding-8b` — the model the eaig
-- gateway actually serves (text-embedding-3-small / 1536 from migration 0004 is not configured
-- there; verified live 2026-06-19). Supersedes the dimension in ADR-0018.
--
-- No ANN index: 4096 exceeds pgvector's HNSW limit (2000 for `vector`, 4000 for `halfvec`), so we
-- can't index it. Search is already scoped to a single (repository_id, commit_sha) via
-- `code_chunks_commit_idx`, so an exact cosine scan over that small subset is fast enough.

-- Existing 1536-d rows are invalid for the new model; clear them (re-indexing repopulates).
TRUNCATE TABLE code_chunks;

DROP INDEX IF EXISTS code_chunks_embedding_idx;

ALTER TABLE code_chunks DROP COLUMN IF EXISTS embedding;
ALTER TABLE code_chunks ADD COLUMN embedding vector(4096) NOT NULL;
