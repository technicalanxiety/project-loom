-- 008_namespace_config.sql
-- Namespace configuration with predicate pack assignment.
-- Each namespace has its own token budgets for hot/warm tiers and
-- a list of predicate packs that control which relationship vocabularies
-- are available during fact extraction.
-- The core pack is always included via DEFAULT '{core}'.

CREATE TABLE loom_namespace_config (
  namespace         TEXT PRIMARY KEY,               -- namespace identifier
  hot_tier_budget   INT DEFAULT 500,                -- max tokens for hot tier memory
  warm_tier_budget  INT DEFAULT 3000,               -- max tokens for warm tier retrieval
  predicate_packs   TEXT[] DEFAULT '{core}',         -- which predicate packs this namespace uses
  description       TEXT,                           -- human-readable namespace description
  created_at        TIMESTAMPTZ DEFAULT now(),
  updated_at        TIMESTAMPTZ DEFAULT now()
);
