-- ADR-0035 part 2: capture 👍/👎 on the bot's posted comments as a review-quality signal.
-- GitHub does NOT webhook reactions, so a singleton `poller` role periodically reads the reactions
-- REST API for the comments we own and reconciles them into `review_feedback`.
--
-- `review_comments` records the comment ids we create at write-back (the create-review response only
-- has the review id, so the inline-comment ids come from a follow-up GET) so the poller knows what to
-- poll and how (the reactions endpoint differs by kind: inline = pulls/comments/{id}, reply =
-- issues/comments/{id}). `file`/`line` correlate an inline comment back to its finding.
CREATE TABLE IF NOT EXISTS review_comments (
    id                UUID        PRIMARY KEY,
    task_id           UUID        NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    github_comment_id BIGINT      NOT NULL,
    kind              TEXT        NOT NULL CHECK (kind IN ('inline', 'reply')),
    file              TEXT,       -- inline only (correlate to the finding)
    line              INT,        -- inline only
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (kind, github_comment_id)
);
CREATE INDEX IF NOT EXISTS review_comments_task_idx ON review_comments (task_id);

-- One reaction on one of our comments. The poller reconciles this against GitHub each cycle: a new
-- reaction is inserted, one that has disappeared (the user un-reacted) is deleted — which is how we
-- get the "deleted" case without a webhook. UNIQUE makes the upsert/reconcile idempotent.
CREATE TABLE IF NOT EXISTS review_feedback (
    id                UUID        PRIMARY KEY,
    task_id           UUID        NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    github_comment_id BIGINT      NOT NULL,
    comment_kind      TEXT        NOT NULL,           -- 'inline' | 'reply'
    reactor           TEXT        NOT NULL,           -- GitHub login
    reaction          TEXT        NOT NULL,           -- '+1' | '-1' | 'heart' | 'rocket' | ...
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (github_comment_id, comment_kind, reactor, reaction)
);
CREATE INDEX IF NOT EXISTS review_feedback_task_idx ON review_feedback (task_id);
