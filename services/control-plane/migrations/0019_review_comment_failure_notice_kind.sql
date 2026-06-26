-- 0019: allow the `failure_notice` comment kind (ADR-0056).
--
-- When a review task fails terminally without finalizing, the control plane posts a brief
-- "review failed, retry" comment and records it in `review_comments` so the fallback is idempotent
-- across retries (the dedup gate `has_posted_to_github` queries this table). The 0015 CHECK only
-- allowed `inline` / `reply`, so extend it to cover the notice kind.
ALTER TABLE review_comments DROP CONSTRAINT IF EXISTS review_comments_kind_check;
ALTER TABLE review_comments
    ADD CONSTRAINT review_comments_kind_check
    CHECK (kind IN ('inline', 'reply', 'failure_notice'));
