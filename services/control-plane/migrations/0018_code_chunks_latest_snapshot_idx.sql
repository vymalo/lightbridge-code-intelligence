-- Support `latest_indexed_commit(repo)` (ADR-0050): `SELECT commit_sha FROM code_chunks
-- WHERE repository_id = $1 ORDER BY created_at DESC, id DESC LIMIT 1`. This runs on EVERY search and
-- graph retrieval (via `task_scope`) plus the per-task index-skip check, and `code_chunks` holds every
-- chunk across all repos (can reach millions of rows). Without this, each call scans + sorts the repo's
-- rows; with it, it's a single index lookup. Column order matches the ORDER BY so the LIMIT 1 is the
-- first index entry. (The existing `code_chunks_commit_idx (repository_id, commit_sha)` serves the
-- by-commit retrieval scan; it does not help the latest-by-time lookup.)
CREATE INDEX IF NOT EXISTS code_chunks_repo_recent_idx
    ON code_chunks (repository_id, created_at DESC, id DESC);
