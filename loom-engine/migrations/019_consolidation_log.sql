-- 019_consolidation_log.sql
-- Consolidation pipeline telemetry: tracks synthesis, refresh, and pruning runs per namespace.

CREATE TABLE loom_consolidation_log (
  id                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  namespace             TEXT NOT NULL,
  run_type              TEXT NOT NULL CHECK (run_type IN ('consolidation', 'pruning')),
  started_at            TIMESTAMPTZ NOT NULL,
  completed_at          TIMESTAMPTZ,
  status                TEXT NOT NULL DEFAULT 'running'
                        CHECK (status IN ('running', 'completed', 'failed')),
  -- Consolidation fields
  clusters_found        INT,
  summaries_created     INT,
  summaries_refreshed   INT,
  -- Pruning fields
  procedures_pruned     INT,
  conflicts_resolved    INT,
  summaries_invalidated INT,
  -- Diagnostics
  error_detail          TEXT,
  duration_ms           INT
);

-- Query recent runs per namespace for dashboard.
CREATE INDEX idx_consolidation_log_ns ON loom_consolidation_log (namespace, started_at DESC);

-- Query all consolidation runs (not pruning).
CREATE INDEX idx_consolidation_log_type ON loom_consolidation_log (run_type, started_at DESC)
  WHERE run_type = 'consolidation';
