# Changelog

All notable changes to Project Loom will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- PostgreSQL schema: episodes, entities, facts, predicates, procedures, resolution conflicts,
  namespace config, audit log, snapshots (migrations 001-013)
- Canonical predicate registry with core and GRC predicate packs
- Graph traversal SQL function (`loom_traverse`)
- Rust project scaffolding (axum, tokio, sqlx, serde, tracing)
- Dashboard scaffolding (React 18, Vite 6, TypeScript, react-router-dom)
- Docker Compose setup (Postgres 16 + pgvector, Ollama, Caddy reverse proxy)
- MCP, REST, and Dashboard API route structure
- Online pipeline stages: classify, namespace, retrieve, weight, rank, compile
- Offline pipeline stages: ingest, extract, resolve, supersede, state, procedures
- Background worker and scheduler scaffolding
- LLM client for Ollama (extraction, classification, embeddings)
- Bearer token authentication middleware
- Biome linting and formatting for dashboard
- Clippy and rustfmt configuration for engine
- Vitest + React Testing Library setup for dashboard
- cargo-nextest configuration with default, ci, and integration profiles
- Test database infrastructure (docker-compose.test.yml)
