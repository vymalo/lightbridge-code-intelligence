-- Per-turn observability for cost/model dashboards (extends ADR-0034, feeds ADR-0046 dashboards):
--   * `model`            — the model that produced the turn. Captures primary→fallback failover
--                          (ADR-0039), so a run that failed over shows BOTH models across its turns.
--   * `reasoning_tokens` — the reasoning slice of `completion_tokens` for reasoning models
--                          (OpenAI `completion_tokens_details.reasoning_tokens`). It is a SUBSET of
--                          `completion_tokens`, NOT additive — don't double-count when summing totals.
-- Both nullable: pre-existing rows, non-reasoning models, and gateways that omit the detail leave them
-- NULL. Only assistant turns carry these (tool-result rows stay NULL, as with the token columns).
ALTER TABLE agent_transcript
    ADD COLUMN IF NOT EXISTS reasoning_tokens BIGINT,
    ADD COLUMN IF NOT EXISTS model            TEXT;
