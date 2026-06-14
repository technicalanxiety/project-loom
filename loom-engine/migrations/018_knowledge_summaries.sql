-- 018_knowledge_summaries.sql
-- Knowledge summaries: derived abstractions from fact clusters, produced by consolidation pipeline.

CREATE TABLE loom_summaries (
  id                    UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  namespace             TEXT NOT NULL,
  subject_entity_id     UUID NOT NULL REFERENCES loom_entities(id),
  summary_text          TEXT NOT NULL,
  source_facts          UUID[] NOT NULL,       -- facts that were consolidated
  fact_count            INT NOT NULL,          -- len(source_facts), denormalized for query
  evidence_status       TEXT NOT NULL DEFAULT 'extracted'
                        CHECK (evidence_status IN ('extracted', 'confirmed')),
  contains_sole_source  BOOLEAN NOT NULL DEFAULT false,
  synthesis_model       TEXT NOT NULL,         -- e.g., 'qwen2.5:32b'
  synthesis_prompt_ver  TEXT NOT NULL,         -- e.g., 'consolidation_v1'
  tier                  TEXT NOT NULL DEFAULT 'warm'
                        CHECK (tier IN ('hot', 'warm')),
  salience_score        FLOAT DEFAULT 0.0,
  created_at            TIMESTAMPTZ DEFAULT now(),
  refreshed_at          TIMESTAMPTZ DEFAULT now(),  -- last time this summary was re-synthesized
  invalidated_at        TIMESTAMPTZ,           -- set when a source fact is superseded
  deleted_at            TIMESTAMPTZ
);

-- Retrieval: namespace + tier, excluding soft-deleted and invalidated.
CREATE INDEX idx_summaries_namespace ON loom_summaries (namespace)
  WHERE deleted_at IS NULL;

-- Entity drill-down: all summaries for an entity.
CREATE INDEX idx_summaries_entity ON loom_summaries (subject_entity_id)
  WHERE deleted_at IS NULL;

-- Tier visibility: filter to hot/warm per compilation profile.
CREATE INDEX idx_summaries_tier ON loom_summaries (namespace, tier)
  WHERE deleted_at IS NULL AND invalidated_at IS NULL;

-- Serving state: embeddings, access tracking, token budget.
CREATE TABLE loom_summary_state (
  summary_id    UUID PRIMARY KEY REFERENCES loom_summaries(id),
  embedding     vector(768),              -- nomic-embed-text
  token_count   INT,                      -- estimated tokens for budget math
  access_count  INT DEFAULT 0,
  last_accessed TIMESTAMPTZ,
  updated_at    TIMESTAMPTZ DEFAULT now()
);

-- Summary embeddings indexed for similarity search.
CREATE INDEX idx_summary_embeddings ON loom_summary_state USING ivfflat (embedding vector_cosine_ops);
