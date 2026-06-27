-- Two-tier review (ADR-0062). `tier` distinguishes a FAST automatic review (on `pull_request opened`:
-- SAST + one diff-only LLM turn, no retrieval) from a DEEP `@mention`-triggered review (full retrieval,
-- multi-turn). Default `deep` so any pre-existing row, and any non-review task (an `index` task, which
-- ignores tier), is treated as the full/safe behavior; the webhook sets it explicitly per trigger.
ALTER TABLE tasks ADD COLUMN IF NOT EXISTS tier TEXT NOT NULL DEFAULT 'deep';
