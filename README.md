# Project Loom

## A PostgreSQL-Native Memory Compiler for AI Workflows

*"Weaving threads of knowledge into fabric."*

Project Loom is an evidence-grounded memory system for AI workflows. It ingests interaction records (episodes) from multiple sources, extracts structured knowledge as entities and facts, and compiles relevant context packages for AI queries. The system emphasizes strict namespace isolation, temporal fact tracking with provenance, and inspectable retrieval decisions.

---

## Architecture

Loom runs as five Docker containers orchestrated via Docker Compose:

```
Docker Compose
├── loom-engine (Rust binary, ~20MB image)
│   ├── MCP endpoint (:8080/mcp)
│   ├── REST endpoint (:8080/api)
│   ├── Dashboard API (:8080/dashboard)
│   ├── Background worker (tokio spawned tasks)
│   └── Scheduled tasks (24h snapshots, tier promotion)
│
├── loom-dashboard (Vite + React, static files served by Caddy)
│   ├── Pipeline health
│   ├── Knowledge graph explorer
│   ├── Compilation trace viewer
│   ├── Entity conflict review queue
│   ├── Predicate candidate review
│   ├── Retrieval quality metrics
│   └── Benchmark comparison views
│
├── postgres (PostgreSQL 16)
│   ├── pgvector, pgAudit extensions
│   └── loom_* tables
│
├── ollama (local LLM inference)
│   ├── gemma4:26b-a4b-q4 (extraction)
│   ├── gemma4:e4b (classification)
│   └── nomic-embed-text (embeddings)
│
└── caddy (reverse proxy + TLS + static file serving)
    ├── /api/* → loom-engine:8080
    ├── /mcp/* → loom-engine:8080
    ├── /dashboard/api/* → loom-engine:8080
    └── /* → loom-dashboard static files
```

## Technology Stack

| Layer | Technology | Purpose |
|-------|-----------|---------|
| **Engine** | Rust (tokio + axum) | Single binary serving MCP, REST, and dashboard APIs. Compile-time SQL checking via sqlx. |
| **Database** | PostgreSQL 16 + pgvector | Single system of record. Vector similarity, audit logging, recursive CTEs for graph traversal. |
| **LLM Inference** | Ollama | Gemma 4 26B MoE for extraction, Gemma 4 E4B for classification, nomic-embed-text for embeddings. |
| **Dashboard** | React + Vite + TypeScript | Interactive pipeline health, graph explorer, trace viewer, conflict review, metrics. |
| **Reverse Proxy** | Caddy | TLS termination, static file serving, API routing. |

## Key Features

- **Two-pipeline architecture**: Online pipeline for low-latency query serving, offline pipeline for async episode processing
- **Three memory types**: Episodic (raw interactions), semantic (extracted facts), procedural (behavioral patterns)
- **Three-pass entity resolution**: Exact match → alias match → semantic similarity (prefers fragmentation over collision)
- **Pack-aware predicate system**: Canonical predicate registry with domain-specific packs (core, GRC, etc.)
- **Four-dimension ranking**: Relevance (0.40), recency (0.25), stability (0.20), provenance (0.15)
- **Intent classification**: Five task classes (debug, architecture, compliance, writing, chat) drive retrieval strategy
- **Temporal fact tracking**: Facts have valid_from/valid_until with supersession chains
- **Hot/warm tier management**: Configurable per-namespace token budgets with automatic promotion/demotion
- **Comprehensive audit logging**: Every compilation decision is traced and inspectable
- **Dual output formats**: XML structured (for Claude) and JSON compact (for local models)
- **Strict namespace isolation**: Hard isolation by default, no cross-namespace leakage

## Prerequisites

- [Docker](https://docs.docker.com/get-docker/) and [Docker Compose](https://docs.docker.com/compose/install/) v2+
- GPU recommended for Ollama (NVIDIA with CUDA support)
- 16GB+ RAM recommended (Gemma 4 26B MoE requires significant memory)

## Quick Start

1. **Clone and configure**

```bash
git clone <repository-url> project-loom
cd project-loom
cp .env.example .env
```

2. **Start all services**

```bash
docker compose up -d
```

3. **Pull Ollama models** (first run only)

```bash
docker compose exec ollama ollama pull gemma4:26b-a4b-q4
docker compose exec ollama ollama pull gemma4:e4b
docker compose exec ollama ollama pull nomic-embed-text
```

4. **Verify health**

```bash
curl https://localhost/api/health
# Expected: "ok"
```

5. **Open the dashboard**

Navigate to `https://localhost` in your browser.

## Configuration

All configuration is via environment variables. See [`.env.example`](.env.example) for the full list:

| Variable | Description | Default |
|----------|-------------|---------|
| `DATABASE_URL` | PostgreSQL connection string | `postgres://loom:loom@postgres:5432/loom` |
| `OLLAMA_URL` | Ollama API base URL | `http://ollama:11434` |
| `EXTRACTION_MODEL` | Model for entity/fact extraction | `gemma4:26b-a4b-q4` |
| `CLASSIFICATION_MODEL` | Model for intent classification | `gemma4:e4b` |
| `EMBEDDING_MODEL` | Model for embeddings (768d) | `nomic-embed-text` |
| `LOOM_BEARER_TOKEN` | API authentication token | `changeme` |
| `LOOM_HOST` | Server bind address | `0.0.0.0` |
| `LOOM_PORT` | Server port | `8080` |
| `RUST_LOG` | Log level filter | `loom_engine=info,tower_http=debug` |

## MCP Integration

### Claude Code Setup

```bash
claude mcp add loom-memory -- curl -s -X POST https://localhost/mcp/ \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOOM_BEARER_TOKEN"
```

### Available MCP Tools

| Tool | Description |
|------|-------------|
| `loom_learn` | Ingest a new episode (async processing) |
| `loom_think` | Compile a context package for a query |
| `loom_recall` | Direct fact lookup for specific entities |

## API Endpoints

| Method | Path | Description |
|--------|------|-------------|
| POST | `/mcp/` | MCP JSON-RPC endpoint (loom_learn, loom_think, loom_recall) |
| POST | `/api/learn` | REST episode ingestion |
| GET | `/api/health` | Health check |
| GET | `/dashboard/api/health` | Pipeline health data |
| GET | `/dashboard/api/entities` | Entity listing |
| GET | `/dashboard/api/facts` | Fact listing |
| GET | `/dashboard/api/traces` | Compilation trace listing |
| GET | `/dashboard/api/conflicts` | Entity conflict queue |
| GET | `/dashboard/api/candidates` | Predicate candidate queue |
| GET | `/dashboard/api/metrics` | Retrieval quality metrics |
| POST | `/dashboard/api/conflicts/:id/resolve` | Resolve entity conflict |
| POST | `/dashboard/api/candidates/:id/resolve` | Resolve predicate candidate |

## Project Structure

```
project-loom/
├── README.md
├── .gitignore
├── .env.example
├── docker-compose.yml
├── Caddyfile
│
├── loom-engine/                        # Rust binary
│   ├── Cargo.toml
│   ├── Dockerfile
│   ├── src/
│   │   ├── main.rs                     # tokio::main, axum router setup
│   │   ├── config.rs                   # AppConfig, LlmConfig
│   │   ├── db/
│   │   │   ├── mod.rs
│   │   │   ├── pool.rs                 # Online + offline connection pools
│   │   │   ├── episodes.rs             # Episode CRUD
│   │   │   ├── entities.rs             # Entity CRUD + resolution
│   │   │   ├── facts.rs                # Fact CRUD + supersession
│   │   │   ├── predicates.rs           # Predicate registry + packs
│   │   │   ├── procedures.rs           # Procedure queries
│   │   │   ├── audit.rs                # Audit log writes
│   │   │   ├── snapshots.rs            # Hot-tier snapshots
│   │   │   ├── traverse.rs             # Graph traversal (loom_traverse)
│   │   │   └── dashboard.rs            # Dashboard data queries
│   │   ├── llm/
│   │   │   ├── mod.rs
│   │   │   ├── client.rs               # Ollama + Azure OpenAI client
│   │   │   ├── embeddings.rs           # nomic-embed-text (768d)
│   │   │   ├── extraction.rs           # Entity + fact extraction
│   │   │   └── classification.rs       # Intent classification
│   │   ├── pipeline/
│   │   │   ├── mod.rs
│   │   │   ├── offline/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── ingest.rs           # Episode ingestion + dedup
│   │   │   │   ├── extract.rs          # Extraction orchestration
│   │   │   │   ├── resolve.rs          # Three-pass entity resolution
│   │   │   │   ├── supersede.rs        # Fact supersession
│   │   │   │   ├── state.rs            # Tier management
│   │   │   │   └── procedures.rs       # Procedure flagging
│   │   │   └── online/
│   │   │       ├── mod.rs
│   │   │       ├── classify.rs         # Intent classification
│   │   │       ├── namespace.rs        # Namespace resolution
│   │   │       ├── retrieve.rs         # Retrieval profiles (parallel)
│   │   │       ├── weight.rs           # Memory weight modifiers
│   │   │       ├── rank.rs             # 4-dimension ranking
│   │   │       └── compile.rs          # Context package compilation
│   │   ├── api/
│   │   │   ├── mod.rs
│   │   │   ├── mcp.rs                  # MCP JSON-RPC
│   │   │   ├── rest.rs                 # REST API
│   │   │   ├── dashboard.rs            # Dashboard API
│   │   │   └── auth.rs                 # Bearer token middleware
│   │   ├── worker/
│   │   │   ├── mod.rs
│   │   │   ├── processor.rs            # Background processing
│   │   │   └── scheduler.rs            # Periodic tasks
│   │   └── types/
│   │       ├── mod.rs
│   │       ├── episode.rs
│   │       ├── entity.rs
│   │       ├── fact.rs
│   │       ├── predicate.rs
│   │       ├── classification.rs
│   │       ├── compilation.rs
│   │       ├── audit.rs
│   │       └── mcp.rs
│   ├── migrations/
│   │   ├── 001_episodes.sql
│   │   ├── 002_entities.sql
│   │   ├── 003_predicate_packs.sql
│   │   ├── 004_predicates.sql
│   │   ├── 005_facts.sql
│   │   ├── 006_procedures.sql
│   │   ├── 007_resolution_conflicts.sql
│   │   ├── 008_namespace_config.sql
│   │   ├── 009_audit_log.sql
│   │   ├── 010_snapshots.sql
│   │   ├── 011_traverse_function.sql
│   │   ├── 012_seed_core_predicates.sql
│   │   └── 013_seed_grc_pack.sql
│   └── prompts/
│       ├── entity_extraction.txt
│       ├── fact_extraction.txt
│       └── classification.txt
│
├── loom-dashboard/                     # React SPA
│   ├── package.json
│   ├── tsconfig.json
│   ├── vite.config.ts
│   ├── index.html
│   ├── Dockerfile
│   └── src/
│       ├── main.tsx
│       ├── App.tsx
│       ├── api/
│       │   └── client.ts
│       └── types/
│           └── index.ts
│
└── .kiro/
    └── specs/                          # Spec-driven development
```

## Development

### Rust Engine

```bash
cd loom-engine

# Build
cargo build

# Run tests
cargo test

# Run with hot reload (requires cargo-watch)
cargo watch -x run

# Run database migrations (requires sqlx-cli)
sqlx migrate run --source migrations/
```

### Dashboard

```bash
cd loom-dashboard

# Install dependencies
npm install

# Development server
npm run dev

# Production build
npm run build
```

### Database Migrations

Migrations are in `loom-engine/migrations/` and run in order. Use sqlx-cli:

```bash
cargo install sqlx-cli --no-default-features --features postgres
export DATABASE_URL=postgres://loom:loom@localhost:5432/loom
sqlx migrate run --source loom-engine/migrations/
```

## Spec-Driven Development

This project uses spec-driven development via Kiro. Design documents, requirements, and task breakdowns are in `.kiro/specs/`. Refer to those files for detailed implementation guidance on each component.

## License

Apache 2.0 — See [LICENSE](LICENSE) for details.
