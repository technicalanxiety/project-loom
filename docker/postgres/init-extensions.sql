-- Enable required PostgreSQL extensions for Project Loom.
-- This script runs automatically on first database initialization
-- via /docker-entrypoint-initdb.d/.

CREATE EXTENSION IF NOT EXISTS vector;
CREATE EXTENSION IF NOT EXISTS pgaudit;
