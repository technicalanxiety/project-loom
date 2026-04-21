-- Enable required PostgreSQL extensions for Project Loom.
-- This script runs automatically on first database initialization
-- via /docker-entrypoint-initdb.d/.
--
-- `vector` is required (pgvector similarity search).
-- `pgaudit` is optional defense-in-depth DB-level audit logging; the
-- standard pgvector/pgvector image does not ship pgaudit binaries, so the
-- enable below is guarded by a pg_available_extensions check and is a
-- no-op when the package isn't installed. See
-- loom-engine/src/db/pool.rs::validate_extensions for the runtime contract.

CREATE EXTENSION IF NOT EXISTS vector;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_available_extensions WHERE name = 'pgaudit') THEN
        CREATE EXTENSION IF NOT EXISTS pgaudit;
    ELSE
        RAISE NOTICE 'pgaudit not available on this image — skipping (optional extension)';
    END IF;
END $$;
