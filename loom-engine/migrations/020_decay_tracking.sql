-- 020_decay_tracking.sql
-- Decay and staleness tracking for pruning: procedures, conflicts, and consolidation config.

-- Procedures: track when last matched by episode and when eligible for pruning.
ALTER TABLE loom_procedures
  ADD COLUMN IF NOT EXISTS last_matched_at TIMESTAMPTZ,        -- last time this procedure matched an episode
  ADD COLUMN IF NOT EXISTS decay_eligible_at TIMESTAMPTZ;      -- computed: last_matched_at + 90 days (or from config)

-- Resolution conflicts: track auto-resolution eligibility.
ALTER TABLE loom_resolution_conflicts
  ADD COLUMN IF NOT EXISTS auto_resolve_at TIMESTAMPTZ;         -- computed: created_at + 60 days (or from config)

-- Facts: ensure access_count exists for warm-tier retrieval signals.
-- (loom_fact_state.last_accessed already exists; access_count is used for pruning signals.)
DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM information_schema.columns
    WHERE table_name = 'loom_fact_state' AND column_name = 'access_count'
  ) THEN
    ALTER TABLE loom_fact_state ADD COLUMN access_count INT DEFAULT 0;
  END IF;
END $$;

-- Namespace consolidation config: schedule, cluster thresholds, TTL rules.
ALTER TABLE loom_namespace_config
  ADD COLUMN IF NOT EXISTS consolidation_min_cluster INT DEFAULT 5,
  ADD COLUMN IF NOT EXISTS consolidation_schedule TEXT DEFAULT '02:00',
  ADD COLUMN IF NOT EXISTS pruning_procedure_ttl_days INT DEFAULT 90,
  ADD COLUMN IF NOT EXISTS pruning_conflict_ttl_days INT DEFAULT 60,
  ADD COLUMN IF NOT EXISTS summary_invalidation_ttl_days INT DEFAULT 30;
