-- 002_entities.sql
-- Entities: Graph nodes representing real-world concepts extracted from episodes.
-- Entity types are constrained to exactly 10 categories for semantic consistency.
-- Entity state is separated into a derived table (loom_entity_state) that can be recomputed.

CREATE TABLE loom_entities (
  -- CANONICAL: Source of truth
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  name            TEXT NOT NULL,                    -- most specific common name
  entity_type     TEXT NOT NULL CHECK (entity_type IN (
    'person', 'organization', 'project', 'service', 'technology',
    'pattern', 'environment', 'document', 'metric', 'decision'
  )),
  namespace       TEXT NOT NULL,                    -- isolation boundary
  properties      JSONB DEFAULT '{}',               -- includes aliases array for resolution
  created_at      TIMESTAMPTZ DEFAULT now(),
  source_episodes UUID[],                           -- provenance: which episodes mentioned this entity

  -- DELETION SEMANTICS: Soft delete
  deleted_at      TIMESTAMPTZ,

  -- Uniqueness: one entity per (name, type, namespace) combination
  UNIQUE(name, entity_type, namespace)
);

-- Entity serving state: Derived, recomputable.
-- Separated from canonical data so it can be rebuilt without losing truth.
-- Tracks embedding, tier placement, salience, and access patterns for retrieval ranking.
CREATE TABLE loom_entity_state (
  entity_id       UUID PRIMARY KEY REFERENCES loom_entities(id),
  embedding       vector(768),                      -- nomic-embed-text via Ollama
  summary         TEXT,                             -- generated entity summary
  tier            TEXT DEFAULT 'warm' CHECK (tier IN ('hot', 'warm')),
  salience_score  FLOAT DEFAULT 0.5,
  access_count    INT DEFAULT 0,
  last_accessed   TIMESTAMPTZ,
  pinned          BOOLEAN DEFAULT false,            -- user-pinned to hot tier
  updated_at      TIMESTAMPTZ DEFAULT now()
);

-- Namespace + entity_type for filtered entity lookups.
-- Partial index excludes soft-deleted entities.
CREATE INDEX idx_entities_ns_type ON loom_entities (namespace, entity_type)
  WHERE deleted_at IS NULL;

-- GIN index on the aliases array inside properties JSONB.
-- Supports the alias-match pass of the 3-pass entity resolution algorithm.
CREATE INDEX idx_entities_aliases ON loom_entities
  USING gin ((properties->'aliases'))
  WHERE deleted_at IS NULL;
