-- 009_audit_log.sql
-- Comprehensive audit log for compilation traces.
-- Every loom_think call writes a full trace here, capturing the classification,
-- retrieval profiles executed, candidates found/selected/rejected with score breakdowns,
-- token counts, output format, and latency breakdown across pipeline stages.
-- This enables the dashboard's compilation trace viewer and retrieval quality metrics.

CREATE TABLE loom_audit_log (
  id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  created_at          TIMESTAMPTZ DEFAULT now(),

  -- QUERY CONTEXT
  task_class          TEXT NOT NULL,                 -- classified intent (debug, architecture, etc.)
  namespace           TEXT NOT NULL,                 -- which namespace was queried
  query_text          TEXT,                          -- the original query text
  target_model        TEXT,                          -- which model the context was compiled for

  -- CLASSIFICATION RESULTS
  primary_class       TEXT NOT NULL,                 -- primary task class
  secondary_class     TEXT,                          -- secondary task class (if confidence gap < 0.3)
  primary_confidence  FLOAT,                         -- confidence score for primary class
  secondary_confidence FLOAT,                        -- confidence score for secondary class

  -- RETRIEVAL EXECUTION
  profiles_executed   TEXT[],                        -- which retrieval profiles ran
  retrieval_profile   TEXT NOT NULL,                 -- primary retrieval profile used
  candidates_found    INT,                           -- total candidates from all profiles
  candidates_selected INT,                           -- candidates included in final package
  candidates_rejected INT,                           -- candidates excluded from final package

  -- CANDIDATE DETAILS (for drill-down in dashboard)
  selected_items      JSONB,                         -- [{type, id, memory_type, score_breakdown}]
  rejected_items      JSONB,                         -- [{type, id, memory_type, reason}]

  -- OUTPUT
  compiled_tokens     INT,                           -- total tokens in compiled package
  output_format       TEXT,                          -- 'structured' or 'compact'

  -- LATENCY BREAKDOWN (milliseconds)
  latency_total_ms    INT,                           -- end-to-end latency
  latency_classify_ms INT,                           -- intent classification stage
  latency_retrieve_ms INT,                           -- retrieval profile execution stage
  latency_rank_ms     INT,                           -- ranking and trimming stage
  latency_compile_ms  INT,                           -- package compilation stage

  -- FEEDBACK
  user_rating         FLOAT                          -- optional user feedback score
);
