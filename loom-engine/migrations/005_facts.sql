-- 005_facts.sql
-- Facts: Temporal graph edges representing relationships between entities.
-- Each fact is a subject-predicate-object triple with provenance tracking.
-- Facts support temporal validity (valid_from/valid_until) and supersession chains.
-- Fact state is separated into a derived table (loom_fact_state) that can be recomputed.

CREATE TABLE loom_facts (
  -- CANONICAL: Source of truth
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  subject_id      UUID NOT NULL REFERENCES loom_entities(id),
  predicate       TEXT NOT NULL,                    -- relationship type (canonical or custom)
  object_id       UUID NOT NULL REFERENCES loom_entities(id),
  namespace       TEXT NOT NULL,                    -- isolation boundary

  -- TEMPORAL: Tracks when this fact was/is valid
  valid_from      TIMESTAMPTZ NOT NULL DEFAULT now(),
  valid_until     TIMESTAMPTZ,                     -- NULL = currently valid

  -- PROVENANCE: Links back to source evidence
  source_episodes UUID[] NOT NULL,                  -- which episodes this fact was extracted from
  superseded_by   UUID REFERENCES loom_facts(id),   -- points to the newer contradicting fact

  -- EVIDENCE STATUS: Reliability classification
  evidence_status TEXT NOT NULL DEFAULT 'extracted' CHECK (evidence_status IN (
    'user_asserted',                                -- explicitly stated by user
    'observed',                                     -- directly observed in episode
    'extracted',                                    -- extracted by LLM
    'inferred',                                     -- inferred from other facts
    'promoted',                                     -- promoted from candidate
    'deprecated',                                   -- marked as no longer relevant
    'superseded'                                    -- replaced by a newer fact
  )),
  evidence_strength TEXT CHECK (evidence_strength IN ('explicit', 'implied')),

  properties      JSONB DEFAULT '{}',               -- flexible additional properties
  created_at      TIMESTAMPTZ DEFAULT now(),

  -- DELETION SEMANTICS: Soft delete
  deleted_at      TIMESTAMPTZ
);

-- Fact serving state: Derived, recomputable.
-- Separated from canonical data so it can be rebuilt without losing truth.
-- Tracks embedding, tier placement, salience, and access patterns for retrieval ranking.
CREATE TABLE loom_fact_state (
  fact_id         UUID PRIMARY KEY REFERENCES loom_facts(id),
  embedding       vector(768),                      -- nomic-embed-text via Ollama
  salience_score  FLOAT DEFAULT 0.5,
  access_count    INT DEFAULT 0,
  last_accessed   TIMESTAMPTZ,
  tier            TEXT DEFAULT 'warm' CHECK (tier IN ('hot', 'warm')),
  pinned          BOOLEAN DEFAULT false,            -- user-pinned to hot tier
  updated_at      TIMESTAMPTZ DEFAULT now()
);

-- Current facts by namespace and subject: the primary retrieval path.
-- Partial index filters to only currently-valid, non-deleted facts.
CREATE INDEX idx_facts_current ON loom_facts (namespace, subject_id)
  WHERE valid_until IS NULL AND deleted_at IS NULL;

-- Object-side lookup for reverse traversal (find facts pointing to an entity).
CREATE INDEX idx_facts_object ON loom_facts (object_id)
  WHERE valid_until IS NULL AND deleted_at IS NULL;

-- Evidence status index for filtering by reliability level.
CREATE INDEX idx_facts_status ON loom_facts (evidence_status);

-- Predicate index for predicate-based queries (e.g. "all uses relationships").
-- Partial index filters to currently-valid, non-deleted facts.
CREATE INDEX idx_facts_predicate ON loom_facts (predicate)
  WHERE valid_until IS NULL AND deleted_at IS NULL;
