-- Permalink to the posted review (epic #89). GitHub's create-review response carries an `html_url`;
-- we persist it so the console can link a run to the exact review on the PR. Nullable: older rows +
-- the rare case GitHub omits it.
ALTER TABLE reviews ADD COLUMN IF NOT EXISTS review_url TEXT;
