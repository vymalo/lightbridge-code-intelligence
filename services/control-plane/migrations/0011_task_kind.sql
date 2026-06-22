-- ADR-0033: run kinds. A task now carries an explicit `kind` resolved from the inbound comment,
-- so a question gets answered instead of being forced through a diff-scoped review.
--   'review' — diff-scoped inline findings, validated + written back (the default, unchanged).
--   'ask'    — a conversational answer posted as a single reply comment, not diff-validated.
-- Existing rows default to 'review', preserving today's behaviour. The idempotency index already
-- discriminates by `command_text`, so distinct instructions never collide regardless of kind.
ALTER TABLE tasks ADD COLUMN IF NOT EXISTS kind TEXT NOT NULL DEFAULT 'review';
