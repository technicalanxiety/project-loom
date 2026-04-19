-- 001_episodes.sql
-- Episodes: Immutable evidence layer.
-- Each episode represents a raw interaction record from a source system (claude-code, manual, github).
-- Episodes are the foundational evidence that all extracted knowledge traces back to.
-- Canonical columns are the source of truth; derived columns (embedding, tags, processed) are recomputable.

CREATE TABLE loom_episodes (
  -- CANONICAL: Source of truth, never recomputed
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  source          TEXT NOT NULL,                    -- e.g. claude-code, manual, github
  source_id       TEXT,                             -- external system identifier
  source_event_id TEXT,                             -- dedup key within source
  content         TEXT NOT NULL,                    -- raw episode text
  content_hash    TEXT NOT NULL,                    -- SHA-256 via sha2 crate for dedup
  occurred_at     TIMESTAMPTZ NOT NULL,             -- when the interaction happened
  ingested_at     TIMESTAMPTZ NOT NULL DEFAULT now(), -- when we received it
  namespace       TEXT NOT NULL,                    -- isolation boundary
  metadata        JSONB DEFAULT '{}',               -- flexible source-specific metadata
  participants    TEXT[],                           -- people involved in the interaction

  -- EXTRACTION LINEAGE: Tracks which models processed this episode
  extraction_model     TEXT,                        -- e.g. "gemma4:26b-a4b-q4", "gpt-4.1-mini"
  classification_model TEXT,                        -- e.g. "gemma4:e4b"
  extraction_metrics   JSONB,                       -- ExtractionMetrics struct serialized per ingestion

  -- DERIVED: Recomputable from canonical data
  embedding       vector(768),                      -- nomic-embed-text via Ollama
  tags            TEXT[],
  processed       BOOLEAN DEFAULT false,            -- true after extraction pipeline completes

  -- DELETION SEMANTICS: Soft delete, never hard delete
  deleted_at      TIMESTAMPTZ,
  deletion_reason TEXT,

  -- Idempotency: prevent duplicate ingestion from the same source event
  UNIQUE(source, source_event_id)
);

-- IVFFlat cosine index for vector similarity search on episode embeddings.
-- Partial index excludes soft-deleted episodes to keep the index lean.
-- lists=100 is a reasonable starting point; tune based on table size.
CREATE INDEX idx_episodes_embedding ON loom_episodes
  USING ivfflat (embedding vector_cosine_ops)
  WITH (lists = 100)
  WHERE deleted_at IS NULL;

-- Namespace + occurred_at for time-ordered retrieval within a namespace.
-- DESC ordering supports "most recent first" queries efficiently.
CREATE INDEX idx_episodes_ns_occurred ON loom_episodes (namespace, occurred_at DESC)
  WHERE deleted_at IS NULL;

-- Content hash index for fast deduplication checks during ingestion.
CREATE INDEX idx_episodes_hash ON loom_episodes (content_hash);

-- Unprocessed episodes index for the background worker polling loop.
-- Only indexes episodes that haven't been processed yet and aren't deleted.
CREATE INDEX idx_episodes_unprocessed ON loom_episodes (ingested_at)
  WHERE processed = false AND deleted_at IS NULL;
