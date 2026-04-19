-- 006_procedures.sql
-- Procedures: Candidate behavioral patterns observed across multiple episodes.
-- Procedures represent recurring workflows or practices that the system detects.
-- They require multiple observations before promotion to high confidence.
-- Promotion criteria: 3+ episodes, 7+ days, confidence >= 0.8.

CREATE TABLE loom_procedures (
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  pattern         TEXT NOT NULL,                    -- description of the behavioral pattern
  category        TEXT,                             -- optional categorization
  namespace       TEXT NOT NULL,                    -- isolation boundary
  source_episodes UUID[] NOT NULL,                  -- which episodes this pattern was observed in
  first_observed  TIMESTAMPTZ DEFAULT now(),
  last_observed   TIMESTAMPTZ DEFAULT now(),
  observation_count INT DEFAULT 1,                  -- how many times this pattern was seen
  evidence_status TEXT NOT NULL DEFAULT 'extracted'
    CHECK (evidence_status IN (
      'extracted',                                  -- initially extracted from episodes
      'promoted',                                   -- promoted after meeting criteria
      'deprecated'                                  -- no longer relevant
    )),
  confidence      FLOAT DEFAULT 0.3,               -- confidence score, increases with observations
  embedding       vector(768),                      -- nomic-embed-text via Ollama
  tier            TEXT DEFAULT 'warm' CHECK (tier IN ('hot', 'warm')),

  -- DELETION SEMANTICS: Soft delete
  deleted_at      TIMESTAMPTZ
);
