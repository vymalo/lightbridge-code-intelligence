-- ADR-0035 (part 1, prerequisite): persist the GitHub review id we create at write-back, so a later
-- feedback signal (👍/👎, captured by polling the reactions API — GitHub does not webhook reactions)
-- can be correlated back to the review/run. Per-inline-comment ids land with the capture work (they
-- aren't in the create-review response and need a follow-up GET). Nullable: older rows + non-PR runs
-- have none.
ALTER TABLE reviews ADD COLUMN IF NOT EXISTS github_review_id BIGINT;
