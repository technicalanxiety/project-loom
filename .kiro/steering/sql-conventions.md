---
description: SQL migration naming, schema rules, and conventions for Project Loom
inclusion: fileMatch
fileMatchPattern: "loom-engine/migrations/**/*.sql"
---

# SQL Migration Conventions — Project Loom

## Naming

- Sequential numbering: `NNN_description.sql` (e.g., `014_add_foo_table.sql`)
- Use lowercase snake_case for all identifiers.
- All Loom tables prefixed with `loom_`.

## Schema Rules

- Every table must clearly separate **canonical** (source of truth) from **derived** (recomputable) columns.
- Use `UUID` primary keys with `DEFAULT gen_random_uuid()`.
- Use `TIMESTAMPTZ` for all timestamps (never `TIMESTAMP`).
- Soft deletes via `deleted_at TIMESTAMPTZ` column — never hard delete data.
- All foreign keys must reference existing tables.
- Add appropriate indexes — especially partial indexes with `WHERE deleted_at IS NULL`.

## Conventions

- `CHECK` constraints for enum-like columns (don't rely on application-level validation alone).
- `JSONB` for flexible/extensible properties, but prefer typed columns for frequently queried fields.
- Comments in migrations explaining *why* a table/column exists.
- Seed data in separate migration files from schema (e.g., `012_seed_core_predicates.sql`).

## Extensions

- `pgvector` for embeddings (`vector(768)` type)
- `pgAudit` for audit logging
- IVFFlat indexes for vector similarity search
