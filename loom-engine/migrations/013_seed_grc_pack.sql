-- 013_seed_grc_pack.sql
-- GRC (Governance, Risk, Compliance) predicate pack with 23 regulatory predicates.
-- This pack is seeded but NOT auto-assigned to any namespace.
-- Operators must explicitly add 'grc' to a namespace's predicate_packs array
-- via loom_namespace_config to enable regulatory relationship extraction.

-- Seed the GRC pack into loom_predicate_packs
INSERT INTO loom_predicate_packs (pack, description)
VALUES ('grc', 'Governance, Risk, and Compliance predicates for regulatory and audit workflows');

-- Seed 23 regulatory predicates into the GRC pack
INSERT INTO loom_predicates (predicate, category, pack, inverse, description) VALUES
  -- SCOPING: What is in/out of scope for compliance
  ('scoped_as',                'regulatory', 'grc', 'scoping_includes',          'Subject is scoped as the object classification'),
  ('scoping_includes',         'regulatory', 'grc', 'scoped_as',                 'Subject scope includes the object'),
  ('de-scoped_from',           'regulatory', 'grc', NULL,                        'Subject was removed from scope of the object'),

  -- EXCEPTIONS: Granted exceptions to compliance requirements
  ('exception_granted_for',    'regulatory', 'grc', 'exception_applies_to',      'Subject has an exception granted for the object requirement'),
  ('exception_applies_to',     'regulatory', 'grc', 'exception_granted_for',     'Subject exception applies to the object'),

  -- CONTROL MAPPING: How controls map to requirements
  ('maps_to_control',          'regulatory', 'grc', 'control_mapped_from',       'Subject maps to the object control'),
  ('control_mapped_from',      'regulatory', 'grc', 'maps_to_control',           'Subject control is mapped from the object'),

  -- EVIDENCE: What evidence supports compliance
  ('evidenced_by',             'regulatory', 'grc', 'evidence_for',              'Subject is evidenced by the object'),
  ('evidence_for',             'regulatory', 'grc', 'evidenced_by',              'Subject is evidence for the object'),

  -- SATISFACTION: What satisfies a requirement
  ('satisfies',                'regulatory', 'grc', 'satisfied_by',              'Subject satisfies the object requirement'),
  ('satisfied_by',             'regulatory', 'grc', 'satisfies',                 'Subject is satisfied by the object'),

  -- PRECEDENT: Legal or regulatory precedent
  ('precedent_set_by',         'regulatory', 'grc', 'sets_precedent_for',        'Subject precedent was set by the object'),
  ('sets_precedent_for',       'regulatory', 'grc', 'precedent_set_by',          'Subject sets precedent for the object'),

  -- FINDINGS: Audit findings and their sources
  ('finding_on',               'regulatory', 'grc', 'finding_raised_by',         'Subject finding is on the object'),
  ('finding_raised_by',        'regulatory', 'grc', 'finding_on',               'Subject finding was raised by the object'),

  -- CONFLICTS AND SUPPLEMENTS: Regulatory interactions
  ('conflicts_with',           'regulatory', 'grc', NULL,                        'Subject conflicts with the object requirement'),
  ('supplements',              'regulatory', 'grc', 'supplemented_by',           'Subject supplements the object'),
  ('supplemented_by',          'regulatory', 'grc', 'supplements',               'Subject is supplemented by the object'),

  -- GAPS: Identified compliance gaps
  ('fills_gap_in',             'regulatory', 'grc', 'gap_filled_by',             'Subject fills a gap in the object'),
  ('gap_filled_by',            'regulatory', 'grc', 'fills_gap_in',              'Subject gap is filled by the object'),

  -- SUPERSESSION: Regulatory version supersession
  ('supersedes_in_context',    'regulatory', 'grc', 'superseded_in_context_by',  'Subject supersedes the object in regulatory context'),
  ('superseded_in_context_by', 'regulatory', 'grc', 'supersedes_in_context',     'Subject is superseded by the object in regulatory context'),

  -- COMPENSATION: Compensating controls
  ('compensated_by',           'regulatory', 'grc', 'compensates_for',           'Subject is compensated by the object control'),
  ('compensates_for',          'regulatory', 'grc', 'compensated_by',            'Subject compensates for the object deficiency');
