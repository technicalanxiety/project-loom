# Changelog

All notable changes to Project Loom will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

#### Episode processing state machine with bounded retries (ADR 007)

- Migration 016 adds `processing_status` (`pending` / `processing` /
  `completed` / `failed`), `processing_attempts`,
  `processing_last_attempt`, `processing_last_error` to `loom_episodes`.
  Partial indexes on `processing_status='pending'` (for the poll path)
  and `processing_status='failed'` (for the dashboard).
- Worker atomically claims rows via conditional UPDATE before running
  the pipeline — safe for multiple replicas even though deployment
  currently runs one.
- Exponential backoff between retries: delay for attempt N is
  `EPISODE_BACKOFF_BASE_SECS * 2^N`. After `EPISODE_MAX_ATTEMPTS`
  (default 5, base 30s) the episode transitions to `failed` and stops
  consuming worker cycles. Fixes the poison-pill retry loop where a
  deterministically-unprocessable episode (e.g. content exceeding
  nomic-embed-text's context window) generated infinite Ollama load.
- New env vars: `EPISODE_MAX_ATTEMPTS`, `EPISODE_BACKOFF_BASE_SECS`.
- New dashboard endpoints: `GET /dashboard/api/episodes/failed`
  (operator triage queue with per-episode error context),
  `POST /dashboard/api/episodes/{id}/requeue` (reset to `pending`,
  clear attempts — used after fixing the root cause).
- `PipelineHealthResponse` gains `failed_episode_count`. `queue_depth`
  is now pending-only — failed episodes are counted separately so the
  dashboard can distinguish "waiting on worker" from "waiting on
  operator."
- Truncates persisted error messages at 2 KiB so long Ollama 400
  bodies don't balloon the failed-episodes table.
- ADR 007 documents the design rationale, alternatives, and operator
  workflow.

#### Initial build (scaffolding through task 25 completion)

- PostgreSQL schema: episodes, entities, facts, predicates, procedures, resolution conflicts,
  namespace config, audit log, snapshots, benchmarks (migrations 001-014)
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
- Benchmark evaluation infrastructure (A/B/C conditions, Week 8 gate)

#### Ingestion-model amendment (ADR 004 + 005)

- Three-mode ingestion taxonomy enforced at schema and API:
  `user_authored_seed`, `vendor_import`, `live_mcp_capture` (migration 015)
- `parser_version` + `parser_source_schema` required for vendor_import,
  forbidden otherwise, via `chk_parser_fields_vendor_import` CHECK constraint
- Verbatim episode content invariant (ADR 005) — `content` must be
  transcript / vendor-export excerpt / user-authored prose; never LLM
  summarization output
- MCP `loom_learn` hardcodes `ingestion_mode = live_mcp_capture` at the
  server boundary; REST `/api/learn` requires explicit mode (HTTP 400 on
  missing)
- Stage 5 ranking: provenance coefficient lookup
  (live_mcp_capture 1.0, user_authored_seed 0.8, vendor_import 0.6) with
  MAX-across-source-episodes for fact candidates
- Stage 6 compilation: `sole_source` flag on facts, `mode="..."` attribute
  on episodes in both structured XML and compact JSON outputs
- Client templates shipped at `templates/`: Claude Code CLAUDE.md,
  Claude Desktop Projects instructions, MCP config example,
  PostSession capture hook (`loom-capture.sh`)
- Bootstrap parser scaffolding at `bootstrap/` with fail-loud schema
  assertion helper and Claude Code JSONL reference parser
- Mode 1 CLI seed tool at `cli/loom-seed.py`
- Dashboard: Parser Health view + Ingestion Mode Distribution view with
  seed-only-namespace warning list
- `.kiro/specs/loom-memory-compiler/{requirements,design,tasks}.md`
  amended with Requirements 53–59 and tasks 26–38
- ADR 004 (three-mode taxonomy) and ADR 005 (verbatim content invariant)
- PROJECT-STANCE.md at repo root (personal-infrastructure scope)
- License reconciled Apache-2.0 → MIT; README, CONTRIBUTING, SECURITY
  rewritten to reflect the "no PRs / no issues / no support" stance

### Changed

#### Dependency refresh (Phase 1–3)

- **Rust**: axum 0.7 → 0.8, reqwest 0.12 → 0.13, fake 3 → 5,
  sqlx trimmed to `default-features = false` with explicit feature list
  (drops the `any` feature and its sqlx-mysql/sqlx-sqlite dependencies
  from the runtime surface)
- **Dashboard**: React 18 → 19, react-router-dom 6 → 7, Vite 6 → 8,
  @vitejs/plugin-react 4 → 6, TypeScript 5.9 → 6.0, vitest 2 → 4,
  @vitest/coverage-v8 2 → 4, @biomejs/biome 1.9 → 2.4, jsdom 25 → 29
- CHECK constraint path notation updated to axum 0.8 `{segment}` syntax
  across 23 route declarations in `src/main.rs`, `src/api/dashboard.rs`,
  and integration test fixtures

#### Docker build pipeline

- Engine base image: musl static build → debian:bookworm-slim glibc
  (musl-gcc does not support `-m64` required by `ring` / `aws-lc`)
- reqwest compiled with `rustls-no-provider` + `ring` as the default
  rustls crypto provider — keeps `aws-lc-sys` out of the dependency
  graph entirely (it cannot cross-compile to musl even if we wanted to)
- `ensure_crypto_provider()` helper in `src/crypto.rs` — idempotent via
  `std::sync::Once`; called from both `main()` and `LlmClient::new()`
  so the binary and test harness install the provider on the same path
- PostgreSQL image: `pgvector/pgvector:pg16` → `pg17`
- Ollama: dropped the nvidia GPU hard requirement from the service
  definition (enable GPU reservation manually if your host has one)

#### Operational changes

- `pgaudit` extension is optional. `pgvector/pgvector` images do not
  ship it, so `validate_extensions` now warns and continues rather than
  halting startup. Application-level audit via `loom_audit_log` is
  unaffected; pgaudit (if enabled separately) provides DB-level audit
  as defense-in-depth.
- Caddy injects `Authorization: Bearer $LOOM_BEARER_TOKEN` on
  `/dashboard/api/*` requests so the in-browser SPA doesn't have to
  manage a token. `/mcp/*` and `/api/*` paths still require the caller
  to supply their own token. See ADR 006 for the trade-off.

### Security

- cargo-audit ignores `RUSTSEC-2023-0071` (Marvin Attack in `rsa` 0.9)
  via `loom-engine/.cargo/audit.toml`. The `rsa` crate is pulled in by
  `sqlx-macros-core → sqlx-mysql` for macro-expansion-time analysis; it
  is not linked into the runtime binary on our target (`cargo tree -i
  sqlx-mysql` returns nothing). Revisit when upstream ships a fix or
  sqlx restructures its macro crate.
- Resolved two moderate Dependabot alerts on `esbuild` (transitively
  fixed by the vitest 2 → 4 bump).
