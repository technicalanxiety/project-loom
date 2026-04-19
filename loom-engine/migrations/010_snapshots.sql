-- 010_snapshots.sql
-- Hot tier snapshots for audit trail.
-- The scheduler captures daily snapshots of each namespace's hot tier contents.
-- This provides a historical record of what memory was always-injected at any point in time,
-- useful for debugging retrieval quality and understanding tier promotion/demotion patterns.

CREATE TABLE loom_snapshots (
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  snapshot_at     TIMESTAMPTZ DEFAULT now(),         -- when the snapshot was taken
  namespace       TEXT NOT NULL,                    -- which namespace this snapshot covers
  hot_entities    JSONB,                            -- hot tier entities at snapshot time
  hot_facts       JSONB,                            -- hot tier facts at snapshot time
  hot_procedures  JSONB,                            -- hot tier procedures at snapshot time
  total_tokens    INT                               -- total token count of hot tier
);
