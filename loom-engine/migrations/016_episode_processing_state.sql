-- 016_episode_processing_state.sql
-- Episode processing state machine for the background worker.
--
-- Before this migration, an episode was either `processed=true` or
-- `processed=false`. A poison-pill episode (e.g. content too large for the
-- embedding model's context window) stayed `processed=false` forever and
-- the worker retried it on every poll, generating infinite Ollama load.
--
-- This migration introduces an explicit state machine, attempt tracking,
-- and last-error persistence so the worker can:
--   1. Apply exponential backoff between retries for a given episode.
--   2. Give up after N attempts and mark the episode `failed`.
--   3. Surface failure reasons in the dashboard for operator triage.
--
-- The legacy `processed` column is retained (no writes from new code) to
-- keep downstream reporting and audit tooling working through the
-- transition. `processing_status` is the authoritative lifecycle field.

ALTER TABLE loom_episodes
  ADD COLUMN processing_status        TEXT        NOT NULL DEFAULT 'pending',
  ADD COLUMN processing_attempts      INTEGER     NOT NULL DEFAULT 0,
  ADD COLUMN processing_last_attempt  TIMESTAMPTZ,
  ADD COLUMN processing_last_error    TEXT;

-- Enforce the four-state invariant. No `retry` state — backoff is expressed
-- by `processing_last_attempt` within `pending`, not a distinct status.
ALTER TABLE loom_episodes
  ADD CONSTRAINT chk_processing_status
    CHECK (processing_status IN ('pending', 'processing', 'completed', 'failed'));

-- Backfill: any rows that already have `processed=true` are completed.
-- Everything else defaults to `pending` via the column DEFAULT above.
UPDATE loom_episodes
SET processing_status = 'completed'
WHERE processed = true;

-- Drop the old partial index on (processed, deleted_at); the worker now
-- polls by processing_status. Replace with a partial index that supports
-- the new poll query — pending episodes, oldest-first, with enough
-- columns to let the optimizer skip rows under backoff cheaply.
DROP INDEX IF EXISTS idx_episodes_unprocessed;

CREATE INDEX idx_episodes_processing_pending
  ON loom_episodes (processing_last_attempt NULLS FIRST, ingested_at)
  WHERE processing_status = 'pending' AND deleted_at IS NULL;

-- Dashboard surfacing index: failed episodes ordered by most recent
-- failure first.
CREATE INDEX idx_episodes_processing_failed
  ON loom_episodes (processing_last_attempt DESC)
  WHERE processing_status = 'failed' AND deleted_at IS NULL;
