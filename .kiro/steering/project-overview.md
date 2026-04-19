---
description: High-level workspace context covering Project Loom architecture, tech stack, and repository layout
inclusion: auto
---

# Project Loom — Workspace Context

## What This Is

Project Loom is a PostgreSQL-native memory compiler for AI workflows. It ingests conversation
episodes, extracts entities and facts into a shallow knowledge graph, and compiles scoped
context packages for LLM consumption via MCP, REST, and a dashboard UI.

**Stage**: Early prototype / MVP
**License**: Apache-2.0
**Goal**: Open-source release

## Architecture Summary

- **loom-engine** (Rust/axum/tokio): Core service — MCP endpoint, REST API, dashboard API,
  background workers, scheduled tasks. Single binary, ~20MB Docker image.
- **loom-dashboard** (TypeScript/React/Vite): Interactive dashboard — pipeline health,
  knowledge graph explorer, compilation trace viewer, conflict review queue.
- **PostgreSQL 16** + pgvector + pgAudit: Single system of record. No external vector store,
  no graph database.
- **Ollama**: Local LLM inference (Gemma 4 for extraction/classification, nomic-embed-text
  for embeddings).
- **Caddy**: Reverse proxy, TLS termination, static file serving.

## Two Pipelines (Strictly Separated)

- **Online** (latency-sensitive): classify → namespace → retrieve → weight → rank → compile → audit
- **Offline** (async, never blocks queries): ingest → dedup → extract entities → resolve → extract facts → supersede → state → procedures → snapshot

## Local Development

```bash
docker-compose up
```

Services: postgres:5432, ollama:11434, loom-engine:8080, caddy:443/80

## Key Design Principles

- PostgreSQL is the single system of record — no second database
- Hard namespace isolation — no cross-namespace queries in MVP
- Authority hierarchy: Episodes > Facts > Procedures
- Two tiers only (hot + warm), cold deferred
- Canonical predicate registry with domain packs
- All queries compile-time checked via sqlx

## Reference

Full specification: #[[file:Project_Loom_v3.md]]
