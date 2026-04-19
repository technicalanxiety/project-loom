-- 011_traverse_function.sql
-- Graph traversal function for 1-2 hop neighborhood exploration.
-- Used by the graph_neighborhood retrieval profile to find related entities and facts.
-- Implements cycle prevention via a path array to avoid infinite loops in cyclic graphs.
-- Traverses facts in both directions (subject->object and object->subject).
-- Filters to namespace, currently-valid facts, and non-deleted entities.

CREATE OR REPLACE FUNCTION loom_traverse(
  p_entity_id UUID,
  p_max_hops INT DEFAULT 2,
  p_namespace TEXT DEFAULT 'default'
) RETURNS TABLE (
  entity_id UUID,
  entity_name TEXT,
  entity_type TEXT,
  fact_id UUID,
  predicate TEXT,
  evidence_status TEXT,
  hop_depth INT,
  path UUID[]
) AS $$
WITH RECURSIVE walk AS (
  -- Base case: the starting entity (hop 0)
  SELECT
    e.id,
    e.name AS entity_name,
    e.entity_type,
    NULL::UUID AS fact_id,
    NULL::TEXT AS predicate,
    NULL::TEXT AS evidence_status,
    0 AS hop_depth,
    ARRAY[e.id] AS path
  FROM loom_entities e
  WHERE e.id = p_entity_id
    AND e.namespace = p_namespace
    AND e.deleted_at IS NULL

  UNION ALL

  -- Recursive case: traverse facts in both subject and object directions
  SELECT
    e2.id,
    e2.name AS entity_name,
    e2.entity_type,
    f.id AS fact_id,
    f.predicate,
    f.evidence_status,
    w.hop_depth + 1,
    w.path || e2.id
  FROM walk w
  JOIN loom_facts f
    ON (f.subject_id = w.id OR f.object_id = w.id)
  JOIN loom_entities e2
    ON (CASE WHEN f.subject_id = w.id THEN f.object_id ELSE f.subject_id END = e2.id)
  WHERE w.hop_depth < p_max_hops
    AND f.valid_until IS NULL                       -- only currently-valid facts
    AND f.deleted_at IS NULL                        -- exclude soft-deleted facts
    AND f.namespace = p_namespace                   -- namespace isolation
    AND e2.deleted_at IS NULL                       -- exclude soft-deleted entities
    AND NOT e2.id = ANY(w.path)                     -- cycle prevention
)
-- Return only traversed nodes (exclude the starting node at hop 0)
SELECT * FROM walk WHERE hop_depth > 0
ORDER BY hop_depth, entity_name;
$$ LANGUAGE sql STABLE;
