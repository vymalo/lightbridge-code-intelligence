-- Persist the runner's free-text status `detail` (#137). Until now the runner could report a
-- meaningful failure reason on `POST /internal/tasks/{id}/status`, but `set_status` only `info!`-logged
-- it and dropped it (the code there literally said "not persisted yet"). With no column to hold it, a
-- review that failed or posted nothing showed up green on the dashboard with no reason: a live 14-day
-- audit found 98 of 144 (~68%) "succeeded" PR-review tasks had posted nothing — failures swallowed as
-- success. This column records that reason so the console can surface it.
--
-- Nullable: a genuine clean success carries no detail, so `NULL` means "no recorded reason" and a
-- non-null value means the runner reported why the run did not post a review.
ALTER TABLE tasks ADD COLUMN IF NOT EXISTS error_detail TEXT;
