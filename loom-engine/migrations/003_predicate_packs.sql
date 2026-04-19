-- 003_predicate_packs.sql
-- Predicate packs: Domain vocabulary sets that group related predicates.
-- Packs allow namespaces to opt into domain-specific relationship vocabularies
-- (e.g. core, grc, healthcare, finserv) without polluting the global predicate space.
-- The core pack is always included; additional packs are assigned per namespace.

CREATE TABLE loom_predicate_packs (
  pack            TEXT PRIMARY KEY,                 -- e.g. 'core', 'grc', 'healthcare', 'finserv'
  description     TEXT,
  created_at      TIMESTAMPTZ DEFAULT now()
);

-- Seed the core pack. Every namespace includes this pack by default.
INSERT INTO loom_predicate_packs (pack, description)
VALUES ('core', 'Core structural, temporal, decisional, and operational predicates shipped with Project Loom');
