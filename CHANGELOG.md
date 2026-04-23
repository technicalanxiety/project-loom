# Changelog

All notable changes to Project Loom will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

#### Extraction-model guidance for iGPU / shared-memory hosts (ADR 009)

- ADR 009 introduces a third `EXTRACTION_MODEL` tier for hosts whose
  memory bandwidth — not capacity — is the bottleneck (AMD APUs,
  ARM SoCs, CPU-only). `qwen2.5:14b` is the new recommended model for
  that class: reliable structured-JSON output at ~30-60 s per episode
  on a Beelink SER5-class Ryzen 5 / Vega iGPU / 32 GB DDR4. `gemma4:26b`
  is retained for discrete-GPU hosts; `gemma4:e4b` remains the default
  classifier everywhere and a last-resort extractor on very tight
  hardware.
- Benchmark table and hardware-tier framing documented in the ADR so
  future hardware classes (Strix Halo, Snapdragon X, etc.) map onto
  whichever tier their memory path resembles.

#### GHCR image publishing on every push to main

- New `.github/workflows/docker-publish.yml` builds `loom-engine` and
  `loom-dashboard` images in parallel and pushes to
  `ghcr.io/technicalanxiety/project-loom/{engine,dashboard}` with both
  `:latest` and `:<sha>` tags on every push to `main`. Separate GHA
  cache scopes per image keep incremental rebuilds fast.
- `docker-compose.yml` switched to `pull_policy: always` on both images
  so `docker compose up -d` always lands the newest main build. The
  dashboard runs as a `busybox` copy-then-exit container that refreshes
  the shared `dashboard_dist` named volume on every start — without
  this, Docker only populates named volumes on first creation and
  updates would be invisible.
- Unblocks the remote dev node (Beelink SER5) from needing a local
  Rust/npm toolchain to deploy.

### Fixed

#### LLM pipeline on memory-bandwidth-bound hardware

- `LlmClient::REQUEST_TIMEOUT` bumped from 30 s to 300 s
  ([loom-engine/src/llm/client.rs](loom-engine/src/llm/client.rs)).
  The previous value silently failed every extraction request on iGPU
  hosts because `gemma4:26b` takes several minutes per prompt there.
  Embeddings always return in under a second, so the long timeout has
  no practical cost on that path. Not configurable — a single constant
  keeps the operating envelope honest. See ADR 009.
- `generate_episode_embedding` now truncates content at 30,000
  characters before sending to `nomic-embed-text`
  ([loom-engine/src/llm/embeddings.rs:18](loom-engine/src/llm/embeddings.rs)).
  `nomic-embed-text`'s context window is ~32 K characters; episodes
  exceeding it previously became poison pills that consumed every
  retry attempt. Paired with the retry/backoff state machine (ADR 007)
  so genuinely unprocessable episodes still bound out, but normal
  oversized Claude Code session transcripts no longer fail.
- `WORKER_CONCURRENCY` env var (default 4, runtime-validated ≥1) lets
  operators on iGPU hosts set `WORKER_CONCURRENCY=1` to serialize
  inference. Parallel extraction on a bandwidth-bound iGPU thrashes
  the shared memory bus and ends up *slower* than serial; one-in-flight
  is the correct default for that hardware. Discrete-GPU hosts can
  leave the default.

#### Offline pipeline no longer clobbers `classification_model`

- `UPDATE loom_episodes SET classification_model = $3` in
  [loom-engine/src/db/episodes.rs](loom-engine/src/db/episodes.rs)
  now uses `COALESCE(NULLIF($3, ''), classification_model)` so an
  empty string preserves the existing value rather than overwriting
  it. The offline extraction pipeline always passed `""` (classification
  is an online-stage concern), which silently erased the classifier
  lineage every time an episode was (re)processed. The dashboard's
  "configured classification model" tile reads the most recent
  non-null value, so after a bulk reprocessing run it went blank. One-
  time backfill for affected deployments:
  `UPDATE loom_episodes SET classification_model = '<configured value>'
  WHERE classification_model = '' OR classification_model IS NULL`.

#### MCP wire protocol at `POST /mcp` (ADR 008)

- New JSON-RPC 2.0 dispatcher at `POST /mcp` speaking the MCP wire
  protocol every real MCP client expects. Handles `initialize`,
  `notifications/initialized`, `ping`, `tools/list`, `tools/call`.
  Previously Loom advertised itself as an MCP server but only
  implemented per-tool REST endpoints under `/mcp/`, which real
  clients (Claude Desktop via `mcp-remote`, ChatGPT Developer Mode,
  GitHub Copilot Agent mode, M365 Copilot declarative agents, Claude
  Code) cannot connect to. This is the unblocker for every client in
  `docs/clients/`.
- `tools/list` advertises the three Loom tools with full JSON Schema
  input schemas. The `loom_learn` description carries the
  verbatim-content invariant (ADR 005) so MCP clients surface it to
  the model at the point of tool choice — a third line of defense
  alongside the per-client discipline templates and DB-level
  enforcement.
- The per-tool REST endpoints (`POST /mcp/loom_learn` etc.) remain
  mounted for direct curl testing and integration tests. Both
  surfaces share handler code so validation, dedup, and the
  `ingestion_mode = live_mcp_capture` hardcoding apply regardless of
  transport.
- Caddyfile matcher updated from `handle /mcp/*` (which does NOT
  match bare `/mcp`) to a named matcher covering both paths.
- 14 unit tests + 13 integration tests in `tests/mcp_rpc.rs`
  exercising the dispatcher end-to-end through the real axum router.
- ADR 008 documents the method surface, alternatives considered
  (rmcp crate, stdio-only shim, SSE fallback, dropping the REST
  endpoints), and the protocol-version negotiation strategy.

#### Multi-client integration (five first-class clients)

- `docs/clients/` folder with one guide per supported client —
  [claude-code.md](docs/clients/claude-code.md),
  [claude-desktop.md](docs/clients/claude-desktop.md),
  [chatgpt-desktop.md](docs/clients/chatgpt-desktop.md),
  [github-copilot.md](docs/clients/github-copilot.md),
  [m365-copilot.md](docs/clients/m365-copilot.md) — plus an index
  README. Each guide covers MCP registration, discipline template,
  vendor-import path (where an export exists), and known gaps. Equal
  billing across all five; there is no "primary" target.
- Discipline templates under `templates/` for ChatGPT Developer Mode
  custom instructions, GitHub Copilot `.github/copilot-instructions.md`,
  M365 Copilot declarative agent manifest + instructions, and an
  updated Claude Desktop config using the `mcp-remote` stdio bridge.
- Bootstrap parsers under `bootstrap/` for Claude.ai / Claude Desktop
  account export (`claude_ai_export_v2`), ChatGPT Data Controls
  export (`chatgpt_export_v1`), and M365 Copilot Purview audit
  export (`m365_copilot_audit_v1`). Plus a stub for GitHub Copilot
  Chat that exits non-zero pointing at the live-capture path (GitHub
  does not publish a conversation export at time of writing).
- Top-level `CLAUDE.md` slimmed to project-context-for-Claude-Code
  sessions; the Claude Code MCP integration details moved to
  `docs/clients/claude-code.md`.

#### Background workers actually start

- `start_processing_loop` (offline extraction) and `start_scheduler`
  (daily/weekly periodic jobs) were defined in `worker/processor.rs`
  and `worker/scheduler.rs` but never called from `main()`. Wired
  both into the runtime alongside `axum::serve`, with graceful
  shutdown on Ctrl-C via a shared `CancellationToken`. Before this
  fix, episodes landed in Postgres with `processed=false` forever —
  the pipeline claims of "async extraction runs in the background"
  were true in intent but unimplemented in practice.

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

#### Native Ollama is now the default (ADR 002 amendment)

- `.env.example` default `OLLAMA_URL=http://host.docker.internal:11434`
  (was `http://ollama:11434`). The engine now talks to a native
  Ollama on the Docker host by default. This recovers Metal / MPS
  acceleration on Apple Silicon (Docker cannot pass it through) and
  CUDA on Linux hosts without Docker GPU setup. Required for
  practical use on 16 GB M-series Macs where Docker Ollama was
  CPU-only and OOM'd on even the classifier model.
- `docker-compose.yml` Ollama service gated behind the
  `with-docker-ollama` Compose profile — invoked with
  `docker compose --profile with-docker-ollama up -d` for Linux +
  CUDA hosts that prefer the containerized path.
- `loom-engine` no longer declares `depends_on: ollama`. The LLM
  client already has retry + Azure OpenAI fallback, and
  `/api/health` reports Ollama as `degraded` rather than blocking
  engine startup.
- `EXTRACTION_MODEL` default changed to `gemma4:e4b` for 16 GB
  hosts (same compact model handles classification and extraction,
  lower quality but actually runs). Larger-memory hosts can set
  `gemma4:26b` for the full MoE extractor.
- ADR 002 amended with the native-vs-docker decision and Apple
  Silicon rationale. README architecture diagram, Container
  Overview table, and Quick Start "Pull Ollama models" section all
  lead with the native path.

#### Bootstrap ingestion plumbing

- `DefaultBodyLimit` raised from axum's 2 MiB default to 64 MiB on
  `/api/learn` and `/mcp` routes. Bootstrap parsers regularly POST
  multi-MB session transcripts; the default was rejecting them with
  413 before the handler saw the request.
- `bootstrap/claude_code_parser.py` now chunks Claude Code JSONL
  sessions at record boundaries with a 4 KiB per-episode cap (was:
  one session = one episode). Raw Claude Code session files run
  3-5 MB and blow past nomic-embed-text's 8192-token context
  window; even at 12 KiB chunks, JSON-dense records (escaped code
  diffs, tool-output blobs) tokenize at ~1 char/token and exceed
  the window. 4 KiB is the empirical floor that keeps worst-case
  chunks under 8 K tokens. Schema version stays
  `claude_code_jsonl_v2`; parser version bumps to 0.3.0 then 0.4.0.
- Parser envelope relaxed from requiring `type + timestamp` to
  `type + sessionId`. Claude Code's actual JSONL has two record
  types (`ai-title`, `last-prompt`) that never carry `timestamp` on
  the envelope; the parser now walks until it finds the first
  timestamp-bearing record per chunk for `occurred_at`.
- New `LOOM_TLS_INSECURE` env var (via shared
  `bootstrap/loom_http.py`) honored by all bootstrap parsers and
  `cli/loom-seed.py` so local development against Caddy's
  self-signed localhost cert works without disabling TLS
  verification globally. Unset in production leaves urllib's
  default (system trust store) verification in place.
- Claude Desktop template switched to the `mcp-remote` stdio
  bridge. Claude Desktop is stdio-only in shipped versions;
  `mcp-remote` wraps the HTTP MCP endpoint into stdio. Template
  documents the `NODE_TLS_REJECT_UNAUTHORIZED=0` env for localhost
  self-signed certs and the `${LOOM_TOKEN}` interpolation gotcha
  (Claude Desktop does not interpolate env vars in this file).

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
