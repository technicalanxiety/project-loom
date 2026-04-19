-- 007_resolution_conflicts.sql
-- Entity resolution conflict tracking.
-- When the 3-pass entity resolution algorithm encounters ambiguous semantic matches
-- (top two candidates within 0.03 similarity), it creates a new entity and logs
-- the conflict here for operator review via the dashboard.
-- Operators can resolve by merging, keeping separate, or splitting.

CREATE TABLE loom_resolution_conflicts (
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  entity_name     TEXT NOT NULL,                    -- the name that triggered the conflict
  entity_type     TEXT NOT NULL,                    -- the type of the ambiguous entity
  namespace       TEXT NOT NULL,                    -- isolation boundary
  candidates      JSONB NOT NULL,                   -- [{id, name, score, method}] candidate matches
  resolved        BOOLEAN DEFAULT false,            -- true after operator resolves
  resolution      TEXT,                             -- "merged:id" or "kept_separate" or "split:id1,id2"
  resolved_at     TIMESTAMPTZ,                      -- when the operator resolved this conflict
  created_at      TIMESTAMPTZ DEFAULT now()
);
