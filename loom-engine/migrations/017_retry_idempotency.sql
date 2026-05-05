-- 017_retry_idempotency.sql
-- Tighten idempotency keys around namespace boundaries and retryable fact writes.

-- Source event IDs are only unique within a namespace. The original global
-- table constraint made a seed/import replay in namespace B collide with the
-- same source event in namespace A.
ALTER TABLE loom_episodes
  DROP CONSTRAINT IF EXISTS loom_episodes_source_source_event_id_key;

CREATE UNIQUE INDEX IF NOT EXISTS idx_episodes_namespace_source_event
  ON loom_episodes (namespace, source, source_event_id)
  WHERE source_event_id IS NOT NULL;

-- Clean up exact duplicate fact/provenance rows that may have been created by
-- a failed processing attempt before adding the retry idempotency index.
WITH duplicate_facts AS (
  SELECT
    id,
    ROW_NUMBER() OVER (
      PARTITION BY namespace, subject_id, predicate, object_id, source_episodes
      ORDER BY created_at NULLS LAST, id
    ) AS duplicate_rank
  FROM loom_facts
  WHERE deleted_at IS NULL
)
UPDATE loom_facts f
SET deleted_at = NOW()
FROM duplicate_facts d
WHERE f.id = d.id
  AND d.duplicate_rank > 1;

CREATE UNIQUE INDEX IF NOT EXISTS idx_facts_episode_triple_provenance
  ON loom_facts (namespace, subject_id, predicate, object_id, source_episodes)
  WHERE deleted_at IS NULL;
