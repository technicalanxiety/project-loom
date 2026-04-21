-- 015_ingestion_mode.sql
-- Episode provenance classification.
--
-- Adds ingestion_mode to loom_episodes and the parser-metadata columns that
-- accompany vendor_import mode. Enforces the three-mode invariant via CHECK:
-- every episode enters through exactly one of user_authored_seed,
-- vendor_import, or live_mcp_capture. There is no llm_reconstruction mode —
-- that is rejected architecturally per docs/adr/004-ingestion-modes.md.
--
-- The database has no rows yet at the point this migration applies, so
-- ingestion_mode is declared NOT NULL without a transitional DEFAULT.
-- Every writer must supply the mode explicitly.

ALTER TABLE loom_episodes
  ADD COLUMN ingestion_mode TEXT NOT NULL
    CHECK (ingestion_mode IN (
      'user_authored_seed',
      'vendor_import',
      'live_mcp_capture'
    ));

-- Parser lineage fields, populated only for vendor_import mode.
-- parser_version:        semantic version of the parser, e.g. 'claude_ai_parser@0.3.1'
-- parser_source_schema:  vendor export schema version asserted against, e.g. 'claude_ai_export_v2'
ALTER TABLE loom_episodes
  ADD COLUMN parser_version       TEXT,
  ADD COLUMN parser_source_schema TEXT;

-- vendor_import requires both parser fields; other modes must leave them NULL.
ALTER TABLE loom_episodes
  ADD CONSTRAINT chk_parser_fields_vendor_import
    CHECK (
      (ingestion_mode =  'vendor_import' AND parser_version       IS NOT NULL
                                         AND parser_source_schema IS NOT NULL)
      OR
      (ingestion_mode <> 'vendor_import' AND parser_version       IS NULL
                                         AND parser_source_schema IS NULL)
    );

CREATE INDEX idx_episodes_ingestion_mode ON loom_episodes (ingestion_mode)
  WHERE deleted_at IS NULL;

-- Parser health view support: narrow index on (parser_version, ingested_at)
-- for the dashboard query that reports last-successful-run per parser.
CREATE INDEX idx_episodes_parser_version ON loom_episodes (parser_version, ingested_at DESC)
  WHERE deleted_at IS NULL AND parser_version IS NOT NULL;
