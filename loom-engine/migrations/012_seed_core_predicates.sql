-- 012_seed_core_predicates.sql
-- Seed 25 core predicates across structural, temporal, decisional, and operational categories.
-- These ship with Project Loom and are always available in every namespace.
-- Each predicate has an inverse relationship for bidirectional graph traversal.
-- Predicates are inserted into the 'core' pack seeded in 003_predicate_packs.sql.

INSERT INTO loom_predicates (predicate, category, pack, inverse, description) VALUES
  -- STRUCTURAL: Containment, dependency, composition relationships
  ('uses',            'structural',  'core', 'used_by',          'Subject uses or consumes the object'),
  ('used_by',         'structural',  'core', 'uses',             'Subject is used or consumed by the object'),
  ('contains',        'structural',  'core', 'contained_in',     'Subject contains or includes the object'),
  ('contained_in',    'structural',  'core', 'contains',         'Subject is contained within the object'),
  ('depends_on',      'structural',  'core', 'dependency_of',    'Subject depends on the object'),
  ('dependency_of',   'structural',  'core', 'depends_on',       'Subject is a dependency of the object'),
  ('implements',      'structural',  'core', 'implemented_by',   'Subject implements the object'),
  ('implemented_by',  'structural',  'core', 'implements',       'Subject is implemented by the object'),
  ('integrates_with', 'structural',  'core', NULL,               'Subject integrates with the object'),

  -- TEMPORAL: Replacement and succession relationships
  ('replaced_by',     'temporal',    'core', 'replaced',         'Subject was replaced by the object'),
  ('replaced',        'temporal',    'core', 'replaced_by',      'Subject replaced the object'),

  -- DECISIONAL: Decision-making and authorship relationships
  ('decided',         'decisional',  'core', 'decided_by',       'Subject made the decision described by the object'),
  ('decided_by',      'decisional',  'core', 'decided',          'Subject decision was made by the object'),
  ('authored_by',     'decisional',  'core', 'authored',         'Subject was authored by the object'),
  ('authored',        'decisional',  'core', 'authored_by',      'Subject authored the object'),
  ('owns',            'decisional',  'core', 'owned_by',         'Subject owns or is responsible for the object'),
  ('owned_by',        'decisional',  'core', 'owns',             'Subject is owned by or responsibility of the object'),

  -- OPERATIONAL: Deployment, configuration, and blocking relationships
  ('deployed_to',     'operational', 'core', 'hosts',            'Subject is deployed to the object environment'),
  ('hosts',           'operational', 'core', 'deployed_to',      'Subject hosts or runs the object'),
  ('targets',         'operational', 'core', NULL,               'Subject targets or is aimed at the object'),
  ('manages',         'operational', 'core', 'managed_by',       'Subject manages the object'),
  ('managed_by',      'operational', 'core', 'manages',          'Subject is managed by the object'),
  ('configured_with', 'operational', 'core', NULL,               'Subject is configured with the object'),
  ('blocked_by',      'operational', 'core', 'blocks',           'Subject is blocked by the object'),
  ('blocks',          'operational', 'core', 'blocked_by',       'Subject blocks the object');
