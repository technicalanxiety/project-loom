-- 004_predicates.sql
-- Canonical predicate registry with pack column and regulatory category.
-- Predicates define the relationship types used in fact triples (subject-predicate-object).
-- Each predicate belongs to exactly one pack and one category.
-- Custom predicates extracted by the LLM that don't match canonical entries
-- are tracked in loom_predicate_candidates for operator review and potential promotion.

CREATE TABLE loom_predicates (
  predicate       TEXT PRIMARY KEY,                 -- canonical relationship name
  category        TEXT NOT NULL CHECK (category IN (
    'structural',                                   -- containment, dependency, composition
    'temporal',                                     -- replacement, succession
    'decisional',                                   -- decisions, authorship
    'operational',                                  -- deployment, configuration, blocking
    'regulatory'                                    -- compliance, audit, governance
  )),
  pack            TEXT NOT NULL DEFAULT 'core'
                  REFERENCES loom_predicate_packs(pack),
  inverse         TEXT,                             -- inverse predicate name (e.g. uses <-> used_by)
  description     TEXT,
  usage_count     INT DEFAULT 0,                    -- incremented each time this predicate is used in a fact
  created_at      TIMESTAMPTZ DEFAULT now()
);

-- Predicate candidates: Custom predicates extracted by the LLM that don't match
-- any canonical predicate. Tracked here for operator review.
-- When occurrences reach 5, the candidate is flagged for review in the dashboard.
-- Operators can map to an existing canonical predicate or promote to a pack.
CREATE TABLE loom_predicate_candidates (
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  predicate       TEXT NOT NULL,                    -- the custom predicate text
  occurrences     INT DEFAULT 1,                    -- how many facts use this predicate
  example_facts   UUID[],                           -- sample fact IDs for operator review
  mapped_to       TEXT REFERENCES loom_predicates(predicate),  -- if mapped to existing canonical
  promoted_to_pack TEXT REFERENCES loom_predicate_packs(pack), -- target pack when promoted
  created_at      TIMESTAMPTZ DEFAULT now(),
  resolved_at     TIMESTAMPTZ                       -- when operator resolved this candidate
);

-- Pack lookup index for loading all predicates belonging to a pack.
-- Used during pack-aware prompt assembly for fact extraction.
CREATE INDEX idx_predicates_pack ON loom_predicates (pack);
