---
description: High-level workspace context covering Project Loom architecture, tech stack, and repository layout
inclusion: auto
---

# Project Loom — Workspace Context

## What This Is

Project Loom is a PostgreSQL-native memory compiler for AI workflows. It ingests conversation
episodes, extracts entities and facts into a shallow knowledge graph, and compiles scoped
context packages for LLM consumption via MCP, REST, and a dashboard UI.

**Stage**: Personal infrastructure (single-operator, MIT-licensed for forks).
**License**: MIT
**Stance**: Published under MIT in case the architecture is useful as a starting point. Not maintained as a product — no PRs reviewed, no issues answered. See `PROJECT-STANCE.md`.

## Architecture Summary

- **loom-engine** (Rust/axum/tokio): Core service — MCP JSON-RPC dispatcher (ADR 008), REST API,
  dashboard API, SSE telemetry stream (ADR 010), offline workers, scheduled tasks. Single binary.
- **loom-dashboard** (TypeScript/React/Vite): 13-page dashboard — Runtime (live SSE-driven),
  Pipeline Health, Compilations, Entities, Predicates, Metrics, Benchmarks, Conflicts,
  Parser Health, Ingestion Distribution.
- **PostgreSQL 17** + pgvector (pgAudit optional): Single system of record. No external vector
  store, no graph database.
- **Ollama** (native by default — ADR 002 amendment): Local LLM inference. Extraction model
  follows hardware tier per ADR 009 — `qwen2.5:14b` for iGPU/APU/CPU, `gemma4:26b` for discrete
  GPU, `gemma4:e4b` for very tight memory. Classification: `gemma4:e4b`. Embeddings:
  `nomic-embed-text` (768d). Extraction calls use `response_format: json_schema` for
  guaranteed-parseable output (ADR 011).
- **Caddy**: Reverse proxy, TLS termination, static file serving, bearer-token injection on
  `/dashboard/api/*` (ADR 006).

## Two Pipelines (Strictly Separated)

- **Online** (latency-sensitive): classify → namespace → retrieve → weight → rank → compile → audit
- **Offline** (async, never blocks queries): ingest → dedup → embed → extract entities → resolve → extract facts → supersede → state → procedures → snapshot. Each episode moves through a `pending → processing → completed | failed` state machine; the worker applies exponential backoff between retries and parks episodes in `failed` after `EPISODE_MAX_ATTEMPTS` so poison-pill inputs can't generate infinite LLM load (ADR 007). Embedding inputs are bounded at 16K characters and extraction output is schema-constrained (ADR 011) so neither stage produces routine poison pills.

## Local Development

```bash
docker compose up -d
```

Services: postgres:5432, loom-engine:8080, caddy:443/80. Ollama runs natively on the host
by default (use `--profile with-docker-ollama` for Linux+CUDA opt-in).

## Key Design Principles

- PostgreSQL is the single system of record — no second database (ADR 001)
- Hard namespace isolation — no cross-namespace queries (ADR 003)
- Three-mode ingestion taxonomy with verbatim content invariant (ADR 004, ADR 005)
- Authority hierarchy: Episodes > Facts > Procedures
- Two tiers only (hot + warm), cold deferred
- Canonical predicate registry with domain packs
- All SQL compile-time checked via sqlx
- No backwards-compat shims; single-operator infrastructure

## Reference

ADR index: `docs/adr/000-template.md` through `011-bounded-inputs-constrained-outputs.md`.
Per-client integration guides: `docs/clients/`.
