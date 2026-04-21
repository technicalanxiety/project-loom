# Project Loom

## A PostgreSQL-Native Memory Compiler for AI Workflows

*"Weaving threads of knowledge into fabric."*

Project Loom is an evidence-grounded memory system for AI workflows. It ingests interaction records (episodes) from multiple sources, extracts structured knowledge as entities and facts, and compiles relevant context packages for AI queries. The system emphasizes strict namespace isolation, temporal fact tracking with provenance, and inspectable retrieval decisions.

---

## Table of Contents

- [Architecture](#architecture)
- [Technology Stack](#technology-stack)
- [Key Features](#key-features)
- [Prerequisites](#prerequisites)
- [Quick Start](#quick-start)
- [Configuration](#configuration)
- [MCP Endpoints](#mcp-endpoints)
- [REST API](#rest-api)
- [Dashboard](#dashboard)
- [Development Setup](#development-setup)
- [Testing](#testing)
- [Project Structure](#project-structure)
- [Contributing](#contributing)
- [License](#license)

---

## Architecture

Loom runs as five Docker containers orchestrated via Docker Compose:

```mermaid
graph TB
    subgraph "Client Layer"
        CC[Claude Code]
        Manual[Manual Ingestion]
        GH[GitHub Webhooks]
    end

    subgraph "Caddy Reverse Proxy :443"
        Caddy[Caddy — TLS + Static Files + Routing]
    end

    subgraph "loom-engine :8080"
        subgraph "API Layer"
            MCP[MCP Endpoints<br/>loom_learn · loom_think · loom_recall]
            REST[REST API<br/>/api/learn · /api/health · /api/webhooks/github]
            DashAPI[Dashboard API<br/>/dashboard/api/*]
            Auth[Bearer Token Middleware]
        end

        subgraph "Online Pipeline — Query Serving"
            Classify[Intent Classifier<br/>Gemma 4 E4B]
            Profiles[Retrieval Profiles<br/>tokio::join! parallel]
            Ranker[4-Dimension Ranker]
            Compiler[Context Compiler<br/>XML + JSON output]
        end

        subgraph "Offline Pipeline — tokio Spawned Tasks"
            Embed[Episode Embedder<br/>nomic-embed-text 768d]
            Extract[Entity + Fact Extractor<br/>Gemma 4 26B MoE]
            Resolve[3-Pass Entity Resolver]
            Supersede[Supersession Resolver]
        end
    end

    subgraph "loom-dashboard"
        DashUI[React SPA<br/>Pipeline Health · Graph Explorer<br/>Trace Viewer · Conflict Queue]
    end

    subgraph "Ollama"
        Models[gemma4:26b-a4b-q4<br/>gemma4:e4b<br/>nomic-embed-text]
    end

    subgraph "PostgreSQL 16"
        DB[(pgvector + pgAudit<br/>loom_* tables)]
    end

    CC --> Caddy
    Manual --> Caddy
    GH --> Caddy

    Caddy -->|/mcp/*| MCP
    Caddy -->|/api/*| REST
    Caddy -->|/dashboard/api/*| DashAPI
    Caddy -->|/*| DashUI

    MCP --> Auth
    REST --> Auth
    DashAPI --> Auth

    MCP -->|loom_think| Classify
    Classify --> Profiles
    Profiles --> Ranker
    Ranker --> Compiler

    MCP -->|loom_learn| Embed
    Embed --> Extract
    Extract --> Resolve
    Resolve --> Supersede

    Extract -.-> Models
    Classify -.-> Models
    Embed -.-> Models

    Profiles -.->|Read| DB
    Compiler -.->|Read| DB
    Supersede -.->|Write| DB
```

### Container Overview

| Container | Image | Purpose |
|-----------|-------|---------|
| **loom-engine** | Rust binary (~20MB) | MCP, REST, Dashboard API, offline pipeline, scheduled tasks |
| **loom-dashboard** | Vite + React SPA | Static files served by Caddy |
| **postgres** | pgvector/pgvector:pg16 | Single data store with pgvector + pgAudit |
| **ollama** | ollama/ollama:latest | Local LLM inference (zero cloud dependency) |
| **caddy** | caddy:2-alpine | TLS termination, reverse proxy, static file serving |

### Pipeline Separation

The online and offline pipelines share PostgreSQL but use **separate connection pools** so offline processing never starves query serving:

- **Online pipeline** (loom_think): classify → retrieve → weight → rank → compile. Target < 500ms p95.
- **Offline pipeline** (loom_learn): embed → extract entities → resolve → extract facts → supersede → tier management. Runs as tokio spawned tasks, returns immediately.

## Technology Stack

| Layer | Technology | Purpose |
|-------|-----------|---------|
| **Engine** | Rust (tokio + axum) | Single binary serving all APIs. Compile-time SQL checking via sqlx. |
| **Database** | PostgreSQL 16 + pgvector + pgAudit | Single system of record. Vector similarity, audit logging, graph traversal. |
| **LLM Inference** | Ollama | Gemma 4 26B MoE (extraction), Gemma 4 E4B (classification), nomic-embed-text (embeddings 768d). |
| **Dashboard** | React 18 + Vite 6 + TypeScript | Pipeline health, graph explorer, trace viewer, conflict review, metrics. |
| **Reverse Proxy** | Caddy | Automatic TLS, static file serving, API routing. |
| **Auth** | Bearer token (tower middleware) | Constant-time comparison, applied at router level. |

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
- **GitHub webhook ingestion**: Automatic episode creation from PR and issue comments

## Prerequisites

- [Docker](https://docs.docker.com/get-docker/) and [Docker Compose](https://docs.docker.com/compose/install/) v2+
- **GPU recommended** for Ollama (NVIDIA with CUDA support for Gemma 4 26B MoE)
- 16GB+ RAM recommended (Gemma 4 26B MoE requires significant memory)
- CPU-only mode works but extraction will be significantly slower

## Quick Start

### 1. Clone and configure

```bash
git clone <repository-url> project-loom
cd project-loom
cp .env.example .env
```

Edit `.env` to set a secure bearer token:

```bash
# Generate a random token
LOOM_BEARER_TOKEN=$(openssl rand -hex 32)
echo "LOOM_BEARER_TOKEN=$LOOM_BEARER_TOKEN" >> .env
```

### 2. Start all services

```bash
docker compose up -d
```

This starts all five containers. PostgreSQL migrations run automatically on first boot.

### 3. Pull Ollama models (first run only)

```bash
docker compose exec ollama ollama pull nomic-embed-text
docker compose exec ollama ollama pull gemma4:e4b
docker compose exec ollama ollama pull gemma4:26b-a4b-q4
```

> Pull `nomic-embed-text` first — it's small and needed for basic operation. The larger Gemma models can download in the background.

### 4. Verify health

```bash
curl -s https://localhost/api/health | jq
```

Expected response:

```json
{
  "status": "ok",
  "database": { "ok": true, "latency_ms": 3, "error": null },
  "ollama": { "ok": true, "latency_ms": 12, "error": null },
  "version": "0.1.0"
}
```

### 5. Ingest your first episode

```bash
curl -s -X POST https://localhost/mcp/loom_learn \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOOM_BEARER_TOKEN" \
  -d '{
    "content": "Discussed migrating the auth service from JWT to OAuth2. Team decided to use Azure AD B2C for identity management. Timeline is Q2 2025.",
    "source": "manual",
    "namespace": "my-project"
  }' | jq
```

Expected response:

```json
{
  "episode_id": "a1b2c3d4-...",
  "status": "queued"
}
```

### 6. Query your memory

```bash
curl -s -X POST https://localhost/mcp/loom_think \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOOM_BEARER_TOKEN" \
  -d '{
    "query": "What authentication approach are we using?",
    "namespace": "my-project"
  }' | jq
```

### 7. Open the dashboard

Navigate to `https://localhost` in your browser.

---

## Configuration

All configuration is via environment variables. Copy `.env.example` to `.env` and customize:

### Database

| Variable | Description | Default |
|----------|-------------|---------|
| `DATABASE_URL` | PostgreSQL connection string | `postgres://loom:loom@postgres:5432/loom` |
| `DATABASE_URL_ONLINE` | Online pipeline pool connection | `postgres://loom:loom@postgres:5432/loom` |
| `DATABASE_URL_OFFLINE` | Offline pipeline pool connection | `postgres://loom:loom@postgres:5432/loom` |
| `ONLINE_POOL_MAX` | Max connections for online pool | `10` |
| `OFFLINE_POOL_MAX` | Max connections for offline pool | `5` |

### LLM / Ollama

| Variable | Description | Default |
|----------|-------------|---------|
| `OLLAMA_URL` | Ollama API base URL | `http://ollama:11434` |
| `EXTRACTION_MODEL` | Model for entity/fact extraction | `gemma4:26b-a4b-q4` |
| `CLASSIFICATION_MODEL` | Model for intent classification | `gemma4:e4b` |
| `EMBEDDING_MODEL` | Model for embeddings (768d) | `nomic-embed-text` |

### Azure OpenAI (Fallback)

| Variable | Description | Default |
|----------|-------------|---------|
| `AZURE_OPENAI_URL` | Azure OpenAI endpoint | *(empty — disabled)* |
| `AZURE_OPENAI_KEY` | Azure OpenAI API key | *(empty — disabled)* |

### Server

| Variable | Description | Default |
|----------|-------------|---------|
| `LOOM_BEARER_TOKEN` | API authentication token | `changeme` |
| `LOOM_HOST` | Server bind address | `0.0.0.0` |
| `LOOM_PORT` | Server port | `8080` |
| `RUST_LOG` | Log level filter | `loom_engine=info,tower_http=debug` |

### Test Database

| Variable | Description | Default |
|----------|-------------|---------|
| `DATABASE_URL_TEST` | Test database (docker-compose.test.yml) | `postgres://loom_test:loom_test@localhost:5433/loom_test` |

---

## MCP Endpoints

All MCP endpoints require `Authorization: Bearer <token>` header.

### loom_learn — Ingest an Episode

Stores an episode and queues it for async extraction. Returns immediately.

**Endpoint:** `POST /mcp/loom_learn`

**Request:**

```json
{
  "content": "Discussed migrating auth service to OAuth2...",
  "source": "claude-code",
  "namespace": "my-project",
  "occurred_at": "2025-01-15T10:30:00Z",
  "metadata": { "session_id": "abc123" },
  "participants": ["jason", "claude"],
  "source_event_id": "session-abc123-msg-42"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `content` | string | yes | Raw episode text |
| `source` | string | yes | Source system: `claude-code`, `manual`, `github` |
| `namespace` | string | yes | Isolation boundary for this memory |
| `occurred_at` | ISO 8601 | no | When the interaction happened (defaults to now) |
| `metadata` | object | no | Arbitrary source-specific metadata |
| `participants` | string[] | no | People involved in the interaction |
| `source_event_id` | string | no | Deduplication key within source |

**Response:**

```json
{
  "episode_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "status": "queued"
}
```

| Status | Meaning |
|--------|---------|
| `queued` | Episode stored, extraction will run asynchronously |
| `duplicate` | Episode already exists (content hash or source_event_id match) |

**Idempotency:** Duplicate detection uses content SHA-256 hash and `(source, source_event_id)` unique constraint. Safe to retry.

---

### loom_think — Compile Context Package

Runs the full online pipeline: classify intent → select retrieval profiles → execute in parallel → apply weights → rank → compile package.

**Endpoint:** `POST /mcp/loom_think`

**Request:**

```json
{
  "query": "What authentication approach are we using for the API gateway?",
  "namespace": "my-project",
  "task_class_override": null,
  "target_model": "claude"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `query` | string | yes | The question to compile context for |
| `namespace` | string | yes | Which namespace to search |
| `task_class_override` | string | no | Force a task class: `debug`, `architecture`, `compliance`, `writing`, `chat` |
| `target_model` | string | no | Target AI model (defaults to `claude`). Controls output format. |

**Response:**

```json
{
  "context_package": "<loom model=\"claude\" tokens=\"1847\" namespace=\"my-project\" task=\"architecture\">...</loom>",
  "token_count": 1847,
  "compilation_id": "b2c3d4e5-f6a7-8901-bcde-f23456789012"
}
```

**Output formats:**
- `target_model` containing "claude" → XML structured format (`<loom>` tags)
- Any other value → JSON compact format

**Task class → retrieval profile mapping:**

| Task Class | Retrieval Profiles |
|------------|-------------------|
| `debug` | graph_neighborhood, episode_recall |
| `architecture` | fact_lookup, graph_neighborhood |
| `compliance` | episode_recall, fact_lookup |
| `writing` | fact_lookup |
| `chat` | fact_lookup |

---

### loom_recall — Direct Fact Lookup

Bypasses classification and retrieval profiles. Returns raw facts for named entities.

**Endpoint:** `POST /mcp/loom_recall`

**Request:**

```json
{
  "entity_names": ["APIM", "Azure AD B2C"],
  "namespace": "my-project",
  "include_historical": false
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `entity_names` | string[] | yes | Entity names to look up |
| `namespace` | string | yes | Which namespace to search |
| `include_historical` | boolean | no | Include superseded/deleted facts (default: false) |

**Response:**

```json
{
  "facts": [
    {
      "id": "c3d4e5f6-...",
      "subject_id": "...",
      "predicate": "uses",
      "object_id": "...",
      "namespace": "my-project",
      "evidence_status": "extracted",
      "valid_from": "2025-01-15T10:30:00Z",
      "valid_until": null,
      "source_episodes": ["a1b2c3d4-..."]
    }
  ]
}
```

---

## REST API

### POST /api/learn — Manual Episode Ingestion

Same as `loom_learn` but forces `source = "manual"`. Requires bearer token.

```bash
curl -X POST https://localhost/api/learn \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOOM_BEARER_TOKEN" \
  -d '{
    "content": "Architecture review notes...",
    "source": "ignored",
    "namespace": "my-project"
  }'
```

### POST /api/webhooks/github — GitHub Webhook Ingestion

Accepts `issue_comment` and `pull_request_review_comment` events. Requires bearer token and `X-GitHub-Event` header.

```bash
# Configure in GitHub repo settings:
# Webhook URL: https://your-domain/api/webhooks/github
# Content type: application/json
# Events: Issue comments, Pull request review comments
# Secret: Use LOOM_BEARER_TOKEN as Authorization header
```

### GET /api/health — Health Check

Unauthenticated. Probes PostgreSQL and Ollama connectivity.

```bash
curl https://localhost/api/health
```

Returns `{"status": "ok"}` when all components are healthy, `{"status": "degraded"}` otherwise. Always returns HTTP 200 so Docker health checks can read the body.

---

## Dashboard

The operational dashboard is a React SPA at `https://localhost`. It requires bearer token authentication.

### Dashboard Views

| View | Description |
|------|-------------|
| **Pipeline Health** | Episode counts by source/namespace, entity counts by type, queue depth, model config |
| **Compilation Traces** | Paginated loom_think history with drill-down to per-candidate score breakdowns |
| **Knowledge Graph Explorer** | Entity search, detail view, visual 1-2 hop neighborhood graph |
| **Entity Conflict Queue** | Unresolved resolution conflicts. Actions: merge, keep separate, split |
| **Predicate Candidate Review** | Custom predicates with occurrence counts. Actions: map to canonical, promote to pack |
| **Predicate Pack Browser** | All packs with predicate counts, categories, usage heatmap |
| **Retrieval Quality Metrics** | Precision over time, latency percentiles (p50/p95/p99), classification confidence |
| **Extraction Quality** | Model comparison, resolution method distribution, custom predicate growth |
| **Hot-Tier Utilization** | Per-namespace hot tier entity/fact counts and budget utilization |

### Dashboard API Endpoints

All dashboard endpoints are under `/dashboard/api/` and require bearer token auth.

| Method | Path | Description |
|--------|------|-------------|
| GET | `/dashboard/api/health` | Pipeline health overview |
| GET | `/dashboard/api/namespaces` | Namespace listing with tier budgets |
| GET | `/dashboard/api/compilations` | Compilation trace list (paginated) |
| GET | `/dashboard/api/compilations/:id` | Compilation trace detail |
| GET | `/dashboard/api/entities` | Entity search (filter by namespace, type, name) |
| GET | `/dashboard/api/entities/:id` | Entity detail with facts |
| GET | `/dashboard/api/entities/:id/graph` | Entity graph neighborhood |
| GET | `/dashboard/api/facts` | Fact listing (filter by namespace, predicate, status) |
| GET | `/dashboard/api/conflicts` | Entity conflict queue |
| GET | `/dashboard/api/predicates/candidates` | Predicate candidate queue |
| GET | `/dashboard/api/predicates/packs` | Predicate pack listing |
| GET | `/dashboard/api/predicates/packs/:pack` | Pack detail with predicates |
| GET | `/dashboard/api/predicates/active/:namespace` | Active predicates for namespace |
| GET | `/dashboard/api/metrics/retrieval` | Retrieval quality metrics |
| GET | `/dashboard/api/metrics/extraction` | Extraction pipeline metrics |
| GET | `/dashboard/api/metrics/classification` | Classification confidence distribution |
| GET | `/dashboard/api/metrics/hot-tier` | Hot-tier utilization per namespace |
| POST | `/dashboard/api/conflicts/:id/resolve` | Resolve entity conflict |
| POST | `/dashboard/api/predicates/candidates/:id/resolve` | Resolve predicate candidate |

---

## Development Setup

### Prerequisites

- **Rust** (stable, latest) — [rustup.rs](https://rustup.rs)
- **Node.js** v20+ and npm
- **Docker** and Docker Compose
- **sqlx-cli** — `cargo install sqlx-cli --no-default-features --features postgres`
- **cargo-nextest** — `cargo install cargo-nextest`
- **cargo-watch** (optional) — `cargo install cargo-watch`

### Rust Engine

```bash
cd loom-engine

# Check compilation
cargo check

# Build
cargo build

# Run with hot reload
cargo watch -x run

# Run database migrations
export DATABASE_URL=postgres://loom:loom@localhost:5432/loom
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

# Lint and format
npm run lint:fix
```

### Database Migrations

Migrations are in `loom-engine/migrations/` and run automatically on engine startup. For manual execution:

```bash
cargo install sqlx-cli --no-default-features --features postgres
export DATABASE_URL=postgres://loom:loom@localhost:5432/loom
sqlx migrate run --source loom-engine/migrations/
```

---

## Testing

### Rust Engine Tests

```bash
cd loom-engine

# Unit tests
cargo nextest run

# With output
cargo nextest run --no-capture

# Start test database for integration tests
docker compose -f docker-compose.test.yml up -d postgres-test

# Run integration tests
DATABASE_URL_TEST=postgres://loom_test:loom_test@localhost:5433/loom_test \
  cargo nextest run --profile integration

# Clippy linting
cargo clippy -- -D warnings

# Format check
cargo fmt --check
```

### Dashboard Tests

```bash
cd loom-dashboard

# Run tests
npm test

# Run tests with coverage
npm run test:coverage

# Biome lint check
npx biome check src/
```

---

## Project Structure

```
project-loom/
├── README.md                           # This file
├── CLAUDE.md                           # Claude Code MCP integration guide
├── CONTRIBUTING.md                     # Contribution guidelines
├── SECURITY.md                         # Security policy
├── CHANGELOG.md                        # Release history
├── .env.example                        # Environment variable template
├── docker-compose.yml                  # Production deployment (5 containers)
├── docker-compose.test.yml             # Test database
├── Caddyfile                           # Reverse proxy configuration
│
├── loom-engine/                        # Rust binary
│   ├── Cargo.toml
│   ├── Dockerfile                      # Multi-stage: builder + distroless
│   ├── src/
│   │   ├── main.rs                     # tokio::main, axum router setup
│   │   ├── config.rs                   # AppConfig from env vars
│   │   ├── db/                         # Database layer (sqlx)
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
│   │   ├── llm/                        # LLM client layer
│   │   │   ├── client.rs               # Ollama + Azure OpenAI client
│   │   │   ├── embeddings.rs           # nomic-embed-text (768d)
│   │   │   ├── extraction.rs           # Entity + fact extraction
│   │   │   └── classification.rs       # Intent classification
│   │   ├── pipeline/
│   │   │   ├── offline/                # Async episode processing
│   │   │   │   ├── ingest.rs           # Episode ingestion + dedup
│   │   │   │   ├── extract.rs          # Extraction orchestration
│   │   │   │   ├── resolve.rs          # Three-pass entity resolution
│   │   │   │   ├── supersede.rs        # Fact supersession
│   │   │   │   ├── state.rs            # Tier management
│   │   │   │   └── procedures.rs       # Procedure flagging
│   │   │   └── online/                 # Query serving
│   │   │       ├── classify.rs         # Intent classification
│   │   │       ├── namespace.rs        # Namespace resolution
│   │   │       ├── retrieve.rs         # Retrieval profiles (parallel)
│   │   │       ├── weight.rs           # Memory weight modifiers
│   │   │       ├── rank.rs             # 4-dimension ranking
│   │   │       └── compile.rs          # Context package compilation
│   │   ├── api/
│   │   │   ├── mcp.rs                  # MCP JSON-RPC handlers
│   │   │   ├── rest.rs                 # REST API + GitHub webhooks
│   │   │   ├── dashboard.rs            # Dashboard API handlers
│   │   │   └── auth.rs                 # Bearer token middleware
│   │   ├── worker/
│   │   │   ├── processor.rs            # Background processing
│   │   │   └── scheduler.rs            # Periodic tasks (snapshots, health)
│   │   └── types/                      # Shared data types (serde)
│   │       ├── episode.rs
│   │       ├── entity.rs
│   │       ├── fact.rs
│   │       ├── predicate.rs
│   │       ├── classification.rs
│   │       ├── compilation.rs
│   │       ├── audit.rs
│   │       └── mcp.rs
│   ├── migrations/                     # PostgreSQL migrations (001-013)
│   └── prompts/                        # LLM prompt templates
│       ├── entity_extraction.txt
│       ├── fact_extraction.txt
│       └── classification.txt
│
├── loom-dashboard/                     # React SPA
│   ├── package.json
│   ├── tsconfig.json
│   ├── vite.config.ts
│   ├── Dockerfile
│   └── src/
│       ├── main.tsx
│       ├── App.tsx
│       ├── api/client.ts               # Typed API client
│       └── types/index.ts              # Shared TypeScript types
│
├── docker/
│   └── postgres/
│       └── init-extensions.sql         # pgvector + pgAudit setup
│
├── docs/
│   └── adr/                            # Architecture Decision Records
│       ├── 001-postgresql-single-store.md
│       ├── 002-local-llm-inference.md
│       └── 003-namespace-isolation.md
│
└── .kiro/
    └── specs/                          # Spec-driven development
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development workflow, code conventions, and testing requirements.

## License

Apache 2.0 — See [LICENSE](LICENSE) for details.
