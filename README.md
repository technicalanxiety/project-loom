# Project Loom

## A PostgreSQL-Native Memory Compiler for AI Workflows

*"Weaving threads of knowledge into fabric."*

Project Loom is personal memory infrastructure. I am building it for my own use and publishing it under MIT in case the architecture is useful to someone else as a starting point. **It is not a product and is not maintained as one** — no PRs reviewed, no issues answered, no support offered. Read [PROJECT-STANCE.md](PROJECT-STANCE.md) before using it. If what you want is a maintained product, this is not it.

Loom is an evidence-grounded memory system for AI workflows. It ingests interaction records (episodes) from multiple sources, extracts structured knowledge as entities and facts, and compiles relevant context packages for AI queries. The system emphasizes strict namespace isolation, temporal fact tracking with provenance, inspectable retrieval decisions, and — critically — a three-mode ingestion taxonomy that keeps LLM-generated content out of the authority hierarchy (see [ADR-004](docs/adr/004-ingestion-modes.md) and [ADR-005](docs/adr/005-verbatim-content-invariant.md)).

### The three ingestion modes at a glance

| Mode | How it enters | Ranking coefficient |
|------|----------------|---------------------|
| `user_authored_seed` (Mode 1) | `cli/loom-seed.py` posts markdown you wrote | 0.8 |
| `vendor_import` (Mode 2) | `bootstrap/*.py` parsers POST vendor export excerpts | 0.6 |
| `live_mcp_capture` (Mode 3) | MCP `loom_learn` or `templates/loom-capture.sh` PostSession hook | 1.0 |

The `llm_reconstruction` mode does not exist. LLM summaries of past conversations are not a valid ingestion path — passing one through `loom_learn` is a user-discipline violation that Loom cannot detect at runtime but actively discourages via the shipped templates and documentation.

---

## Table of Contents

- [Architecture](#architecture)
- [Supported Clients](#supported-clients)
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
    subgraph "MCP Clients — live_mcp_capture"
        CC[Claude Code]
        CD[Claude Desktop]
        CGPT[ChatGPT Desktop]
        GHC[GitHub Copilot]
        M365[M365 Copilot]
    end

    subgraph "Other Ingestion"
        Boot[Bootstrap parsers<br/>vendor_import]
        Seed[loom-seed CLI<br/>user_authored_seed]
        GH[GitHub Webhooks<br/>live_mcp_capture]
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
            Classify[Intent Classifier<br/>gemma4:e4b]
            Profiles[Retrieval Profiles<br/>tokio::join! parallel]
            Ranker[4-Dimension Ranker]
            Compiler[Context Compiler<br/>XML + JSON output]
        end

        subgraph "Offline Pipeline — tokio Spawned Tasks"
            Embed[Episode Embedder<br/>nomic-embed-text 768d]
            Extract[Entity + Fact Extractor<br/>qwen2.5:14b · gemma4:26b · gemma4:e4b<br/>schema-constrained output]
            Resolve[3-Pass Entity Resolver]
            Supersede[Supersession Resolver]
        end

        subgraph "Telemetry"
            Tele[Sampler + Ring Buffer<br/>SSE @ /dashboard/api/stream/telemetry]
        end
    end

    subgraph "loom-dashboard"
        DashUI[React SPA — 13 pages<br/>Runtime · Pipeline Health · Compilations<br/>Entities · Predicates · Metrics · Benchmarks<br/>Conflicts · Parser Health · Distribution]
    end

    subgraph "Ollama (native by default, docker opt-in)"
        Models[Extraction tier per ADR-009:<br/>qwen2.5:14b iGPU/APU · gemma4:26b dGPU<br/>gemma4:e4b classifier · nomic-embed-text]
    end

    subgraph "PostgreSQL 17"
        DB[(pgvector + optional pgAudit<br/>loom_* tables)]
    end

    CC --> Caddy
    CD --> Caddy
    CGPT --> Caddy
    GHC --> Caddy
    M365 --> Caddy
    Boot --> Caddy
    Seed --> Caddy
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

    Tele -.->|Read| DB
    Tele -.->|Probe| Models
    DashAPI -.->|SSE| Tele

    Profiles -.->|Read| DB
    Compiler -.->|Read| DB
    Supersede -.->|Write| DB
```

### Container Overview

| Container | Image | Purpose |
|-----------|-------|---------|
| **loom-engine** | Rust binary on debian:bookworm-slim | MCP, REST, Dashboard API, offline pipeline, scheduled tasks |
| **loom-dashboard** | Build-only (node:22 → busybox) | React bundle populated into `dashboard_dist` volume, container exits |
| **postgres** | pgvector/pgvector:pg17 | Single data store with pgvector (pgAudit optional — image doesn't ship it) |
| **ollama** *(opt-in)* | ollama/ollama:latest | Local LLM inference. Native on the Docker host is the default (required on Apple Silicon — Docker cannot pass through Metal). Dockered Ollama lives behind the `with-docker-ollama` Compose profile for Linux + CUDA hosts. See [ADR-002](docs/adr/002-local-llm-inference.md). |
| **caddy** | caddy:2-alpine | TLS termination, reverse proxy, static file serving, bearer-token injection for dashboard API |

### Pipeline Separation

The online and offline pipelines share PostgreSQL but use **separate connection pools** so offline processing never starves query serving:

- **Online pipeline** (loom_think): classify → retrieve → weight → rank → compile. Target < 500ms p95.
- **Offline pipeline** (loom_learn): embed → extract entities → resolve → extract facts → supersede → tier management. Runs as tokio spawned tasks, returns immediately. Each episode moves through a `pending → processing → completed | failed` state machine with exponential backoff on failure (see [ADR-007](docs/adr/007-episode-processing-retry-backoff.md)); permanently-unprocessable episodes surface on the dashboard for operator triage rather than retrying forever. Embedding inputs are bounded at 16K characters and extraction output is constrained to a JSON Schema via Ollama's `response_format` so neither stage produces routine poison pills (see [ADR-011](docs/adr/011-bounded-inputs-constrained-outputs.md)).
- **Telemetry sampler**: a 1 Hz background task samples host CPU/memory, Ollama state, pipeline-stage p50 latency, queue counters, and recent failures into an in-process ring buffer. The dashboard subscribes via SSE at `/dashboard/api/stream/telemetry` (see [ADR-010](docs/adr/010-streaming-telemetry.md)). No new tables, no time-series store.

## Supported Clients

Loom is transport-agnostic. Any surface that can speak HTTP to `/mcp/*`
or `/api/learn` can drive it. Five clients are first-class — each ships
with an integration guide, a discipline template, and (where the client
publishes an export) a bootstrap parser. All five get equal billing —
there is no "primary" target.

| Client | MCP live capture | Vendor import | Guide |
|--------|------------------|---------------|-------|
| Claude Code | Yes — HTTP transport + PostSession hook | Yes — local JSONL | [docs/clients/claude-code.md](docs/clients/claude-code.md) |
| Claude Desktop | Yes — HTTP transport | Yes — Claude.ai export | [docs/clients/claude-desktop.md](docs/clients/claude-desktop.md) |
| ChatGPT Desktop | Yes — Developer Mode apps (Business / Enterprise / Edu) | Yes — Data Controls export | [docs/clients/chatgpt-desktop.md](docs/clients/chatgpt-desktop.md) |
| GitHub Copilot (VS Code) | Yes — `.vscode/mcp.json`, Agent mode | No — no Copilot Chat export published | [docs/clients/github-copilot.md](docs/clients/github-copilot.md) |
| M365 Copilot | Yes — Copilot Studio declarative agent | Yes — Purview audit export | [docs/clients/m365-copilot.md](docs/clients/m365-copilot.md) |

Every MCP transport converges on the same server handler, which
hardcodes `ingestion_mode = live_mcp_capture` — clients cannot forge
the mode through MCP. Vendor-import parsers explicitly set
`vendor_import` and carry `parser_version` + `parser_source_schema`
so the Stage 5 ranker can apply the correct provenance coefficient.

See [docs/clients/README.md](docs/clients/README.md) for the per-client
guide index and the checklist for wiring a new client.

## Technology Stack

| Layer | Technology | Purpose |
|-------|-----------|---------|
| **Engine** | Rust (tokio + axum 0.8 + sqlx 0.8) | Single binary serving all APIs. `ring` is the rustls crypto provider (keeps `aws-lc-sys` out of the build). |
| **Database** | PostgreSQL 17 + pgvector (pgAudit optional) | Single system of record. Vector similarity, graph traversal, application-level audit logging via `loom_audit_log`. |
| **LLM Inference** | Ollama (native by default) | Hardware-tier extraction model — `qwen2.5:14b` for iGPU/APU, `gemma4:26b` for discrete GPU, `gemma4:e4b` for tight memory (see [ADR-009](docs/adr/009-extraction-model-for-igpu.md)). `gemma4:e4b` for classification, `nomic-embed-text` for embeddings (768d). Extraction calls use `response_format: json_schema` for guaranteed-parseable output ([ADR-011](docs/adr/011-bounded-inputs-constrained-outputs.md)). |
| **Dashboard** | React 19 + Vite 8 + TypeScript 6 + vitest 4 + biome 2 | 13 pages: Runtime (SSE-driven live status), Pipeline Health, Compilations + detail, Entities + detail, Predicates + Pack detail, Metrics, Benchmarks, Conflicts, Parser Health, Ingestion Distribution. |
| **Reverse Proxy** | Caddy | Automatic TLS, static file serving, API routing, bearer-token injection on `/dashboard/api/*`. |
| **Auth** | Bearer token (tower middleware) | Constant-time comparison, applied at router level. External clients (MCP / REST) carry their own token; Caddy injects on behalf of the in-browser SPA. |

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
- 16 GB+ RAM. Extraction-model choice follows hardware tier per [ADR-009](docs/adr/009-extraction-model-for-igpu.md):
  - **Discrete GPU (≥16 GB VRAM)**: `gemma4:26b` — full MoE extractor.
  - **iGPU / APU / CPU-only with ≥16 GB system RAM**: `qwen2.5:14b` — best quality/speed for shared-memory hosts (~30–60 s per episode on a Beelink SER5-class Ryzen).
  - **Very tight memory (≤16 GB total, no discrete GPU)**: `gemma4:e4b` accepting lower extraction quality, or run extraction off-host via Azure OpenAI fallback.

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

Ollama runs natively on the host by default (required on Apple Silicon —
Docker cannot pass through Metal). Install via `brew install ollama`
or the Ollama desktop app, start it (`ollama serve` or the menu-bar
app), then:

```bash
ollama pull nomic-embed-text
ollama pull gemma4:e4b        # always — used for classification on every host

# Pick ONE extraction-tier model per ADR-009:
ollama pull qwen2.5:14b       # iGPU / APU / CPU-only with ≥16 GB system RAM
# ollama pull gemma4:26b      # discrete GPU with ≥16 GB VRAM
# (very tight memory) reuse gemma4:e4b — set EXTRACTION_MODEL=gemma4:e4b
```

If you are on Linux with CUDA and prefer to run Ollama in a container,
enable the `with-docker-ollama` Compose profile instead — see the
comment in `docker-compose.yml` and set
`OLLAMA_URL=http://ollama:11434` in `.env`.

> Pull `nomic-embed-text` first — it's small and needed for basic operation. The larger Gemma models can download in the background.

### 4. Verify health

```bash
curl -s https://localhost/api/health -k | jq
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

The MCP endpoint hardcodes `ingestion_mode=live_mcp_capture` regardless of what the client sends. The REST endpoint requires it explicitly.

```bash
# Via MCP (client does not set ingestion_mode; server hardcodes live_mcp_capture)
curl -s -X POST https://localhost/mcp/loom_learn \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOOM_BEARER_TOKEN" \
  -d '{
    "content": "Discussed migrating the auth service from JWT to OAuth2. Team decided to use Azure AD B2C for identity management. Timeline is Q2 2025.",
    "source": "claude-code",
    "namespace": "my-project"
  }' | jq

# Or via REST — any of the three modes, explicit
curl -s -X POST https://localhost/api/learn \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOOM_BEARER_TOKEN" \
  -d '{
    "content": "(verbatim meeting note, not an LLM summary)",
    "source": "manual",
    "namespace": "my-project",
    "ingestion_mode": "user_authored_seed"
  }' | jq
```

For Mode 1 seed documents in bulk, use the CLI tool:

```bash
LOOM_URL=https://localhost LOOM_TOKEN="$LOOM_BEARER_TOKEN" \
  python3 cli/loom-seed.py --namespace my-project path/to/seeds/
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
| `OLLAMA_URL` | Ollama API base URL | `http://host.docker.internal:11434` (native Ollama on the Docker host — see [ADR-002](docs/adr/002-local-llm-inference.md) amendment) |
| `EXTRACTION_MODEL` | Model for entity/fact extraction. Tier guidance per [ADR-009](docs/adr/009-extraction-model-for-igpu.md) — `gemma4:26b` (discrete GPU), `qwen2.5:14b` (iGPU/APU/CPU), or `gemma4:e4b` (very tight memory, lower quality). | `gemma4:e4b` |
| `CLASSIFICATION_MODEL` | Model for intent classification | `gemma4:e4b` |
| `EMBEDDING_MODEL` | Model for embeddings (768d) | `nomic-embed-text` |
| `WORKER_CONCURRENCY` | Parallel offline-extraction workers. Set to `1` on iGPU/APU hosts to serialize inference — parallel runs thrash shared memory bandwidth and end up *slower*. Discrete-GPU hosts can leave the default. | `4` |

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

### Episode Processing Worker

| Variable | Description | Default |
|----------|-------------|---------|
| `EPISODE_MAX_ATTEMPTS` | Max retry attempts before an episode is marked `failed` | `5` |
| `EPISODE_BACKOFF_BASE_SECS` | Base backoff in seconds; actual delay is `base * 2^attempts` | `30` |

After `EPISODE_MAX_ATTEMPTS` failures the episode transitions to `processing_status='failed'` and the worker stops retrying. The dashboard surfaces these at `/dashboard/api/episodes/failed` for triage; once the root cause is fixed, `POST /dashboard/api/episodes/{id}/requeue` resets the counter. See [ADR-007](docs/adr/007-episode-processing-retry-backoff.md).

### Test Database

| Variable | Description | Default |
|----------|-------------|---------|
| `DATABASE_URL_TEST` | Test database (docker-compose.test.yml) | `postgres://loom_test:loom_test@localhost:5433/loom_test` |

---

## MCP Endpoints

Loom exposes its three tools (`loom_learn`, `loom_think`, `loom_recall`)
on two HTTP surfaces. Both require `Authorization: Bearer <token>`.

- **`POST /mcp`** — MCP JSON-RPC 2.0 dispatcher. This is what real MCP
  clients hit after registering `https://<your-loom>/mcp` as an MCP
  server (Claude Desktop via `mcp-remote`, ChatGPT Developer Mode apps,
  GitHub Copilot Agent mode, M365 Copilot declarative agents, Claude
  Code `claude mcp add`). The dispatcher speaks the methods every MCP
  client expects: `initialize`, `notifications/initialized`, `ping`,
  `tools/list`, `tools/call`. See [ADR-008](docs/adr/008-mcp-wire-protocol.md)
  for the decision rationale and the set of methods supported.

- **`POST /mcp/loom_learn`, `POST /mcp/loom_think`, `POST /mcp/loom_recall`** —
  per-tool REST endpoints. These predate the JSON-RPC dispatcher and
  remain mounted for direct curl testing, integration tests, and
  callers that don't want to speak the MCP wire protocol. They are
  *not* what MCP clients connect to; use them for scripts, smoke tests,
  and one-off requests.

The two surfaces share the same handler code — whether a call arrives
via JSON-RPC `tools/call` or a direct `POST /mcp/loom_*`, it runs the
same validation, the same dedup logic, and the same pipeline. The MCP
server hardcodes `ingestion_mode = live_mcp_capture` at both entry
points; clients cannot override it through either transport.

### JSON-RPC dispatcher request shape

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "loom_learn",
    "arguments": {
      "content": "Discussed migrating auth service to OAuth2...",
      "source": "claude-desktop",
      "namespace": "my-project"
    }
  }
}
```

Responses follow the MCP `tools/call` convention:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "content": [
      { "type": "text", "text": "{\n  \"episode_id\": \"...\",\n  \"status\": \"queued\"\n}" }
    ]
  }
}
```

On tool-level failure the result carries `"isError": true` plus a text
block describing the failure. Transport-level errors (malformed
envelope, unknown method) surface as standard JSON-RPC error objects
with the canonical `-32600`/`-32601`/`-32602` codes.

The tool argument schemas below describe the shape that goes inside
`params.arguments` for JSON-RPC callers and the full body for direct
REST callers.

### loom_learn — Ingest an Episode

Stores an episode and queues it for async extraction. Returns immediately.

**Endpoints:** `POST /mcp/loom_learn` (REST) or `POST /mcp` with
`method: "tools/call"` + `params.name: "loom_learn"` (JSON-RPC).

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
| `source` | string | yes | Free-form source identifier (e.g. `claude-code`, `claude-desktop`, `chatgpt`, `github-copilot`, `m365-copilot`, `manual`, `github`) |
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

**Endpoints:** `POST /mcp/loom_think` (REST) or `POST /mcp` with
`method: "tools/call"` + `params.name: "loom_think"` (JSON-RPC).

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

**Endpoints:** `POST /mcp/loom_recall` (REST) or `POST /mcp` with
`method: "tools/call"` + `params.name: "loom_recall"` (JSON-RPC).

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

### POST /api/learn — Direct Episode Ingestion

General-purpose ingestion endpoint. Bootstrap parsers (per-client vendor exports), the `loom-seed` CLI, and live-capture hooks (like Claude Code's PostSession hook) all POST here. The caller specifies `ingestion_mode`; the server validates the enum and the parser-metadata coupling.

```bash
curl -X POST https://localhost/api/learn \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOOM_BEARER_TOKEN" \
  -d '{
    "content": "Architecture review notes...",
    "source": "manual",
    "namespace": "my-project",
    "ingestion_mode": "user_authored_seed"
  }'
```

**Validation rules:**

| Condition | Response |
|-----------|----------|
| `ingestion_mode` missing | HTTP 400 |
| `ingestion_mode` not one of the three valid values | HTTP 400 |
| `ingestion_mode = vendor_import` without `parser_version` + `parser_source_schema` | HTTP 400 |
| `ingestion_mode` is `user_authored_seed` or `live_mcp_capture` but parser fields present | HTTP 400 |

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

The operational dashboard is a React SPA at `https://localhost`. In the browser you don't supply a token — Caddy injects `Authorization: Bearer $LOOM_BEARER_TOKEN` on every `/dashboard/api/*` request on the SPA's behalf. Just open the URL.

The token still lives in `.env` (shared with the engine). Caddy reads it via `${LOOM_BEARER_TOKEN}` expansion at startup. See [ADR-006](docs/adr/006-dashboard-auth-injection.md) for the security trade-off (CSRF exposure on `/dashboard/api/*` in exchange for skipping a login flow — acceptable for localhost / trusted-network deployment, not for hostile networks).

External clients calling `/mcp/*` or `/api/*` still carry their own bearer token — Caddy does not inject on those paths.

### Dashboard Views

| View | Description |
|------|-------------|
| **Runtime** | Live SSE-driven status: host CPU/memory, Ollama model + GPU/CPU badge, per-stage p50 latency, ingestion queue counters, recent failures table, **Retry failed** bulk-requeue button. Connection pulses at 1 Hz. See [ADR-010](docs/adr/010-streaming-telemetry.md). |
| **Pipeline Health** | Episode counts by source/namespace, entity counts by type, pending queue depth, failed-episode count, model config |
| **Compilations** | Paginated loom_think history with drill-down to per-candidate score breakdowns |
| **Entities** | Entity search and detail view with 1-2 hop neighborhood graph, tier pills, salience bars |
| **Predicates** | Custom predicate candidates (map to canonical / promote to pack drawers) plus a pack browser with usage heatmap |
| **Conflicts** | Entity resolution conflict queue. Actions: merge, keep separate, split |
| **Metrics** | Retrieval precision over time, latency percentiles (p50/p95/p99), classification confidence, extraction quality, hot-tier utilization |
| **Benchmarks** | A/B/C condition runs, per-task precision/latency, winner card |
| **Parser Health** | Per-bootstrap-parser episode counts, parser versions, freshness pills, last-ingested timestamps |
| **Ingestion Mode Distribution** | Per-namespace Mode 1/2/3 breakdown with a seed-only warning list |

### Dashboard API Endpoints

All dashboard endpoints are under `/dashboard/api/`. Requests flow through Caddy, which injects the bearer token transparently; the engine itself still requires `Authorization: Bearer <token>` on every hit, so bypassing Caddy (e.g. calling the engine's port directly) requires supplying the header manually.

| Method | Path | Description |
|--------|------|-------------|
| GET | `/dashboard/api/health` | Pipeline health overview |
| GET | `/dashboard/api/namespaces` | Namespace listing with tier budgets |
| GET | `/dashboard/api/compilations` | Compilation trace list (paginated) |
| GET | `/dashboard/api/compilations/{id}` | Compilation trace detail |
| GET | `/dashboard/api/entities` | Entity search (filter by namespace, type, name) |
| GET | `/dashboard/api/entities/{id}` | Entity detail with facts |
| GET | `/dashboard/api/entities/{id}/graph` | Entity graph neighborhood |
| GET | `/dashboard/api/facts` | Fact listing (filter by namespace, predicate, status) |
| GET | `/dashboard/api/conflicts` | Entity conflict queue |
| GET | `/dashboard/api/predicates/candidates` | Predicate candidate queue |
| GET | `/dashboard/api/predicates/packs` | Predicate pack listing |
| GET | `/dashboard/api/predicates/packs/{pack}` | Pack detail with predicates |
| GET | `/dashboard/api/predicates/active/{namespace}` | Active predicates for namespace |
| GET | `/dashboard/api/metrics/retrieval` | Retrieval quality metrics |
| GET | `/dashboard/api/metrics/extraction` | Extraction pipeline metrics |
| GET | `/dashboard/api/metrics/classification` | Classification confidence distribution |
| GET | `/dashboard/api/metrics/hot-tier` | Hot-tier utilization per namespace |
| GET | `/dashboard/api/metrics/parser-health` | Per-bootstrap-parser rows: version, schema, episode count, last ingested |
| GET | `/dashboard/api/metrics/ingestion-distribution` | Per-namespace Mode 1/2/3 counts + seed-only warning list |
| POST | `/dashboard/api/conflicts/{id}/resolve` | Resolve entity conflict |
| POST | `/dashboard/api/predicates/candidates/{id}/resolve` | Resolve predicate candidate |
| GET | `/dashboard/api/episodes/failed` | List episodes that exhausted retries (operator triage queue) |
| POST | `/dashboard/api/episodes/{id}/requeue` | Reset a single episode's processing state to `pending` (after fixing the root cause) |
| POST | `/dashboard/api/episodes/failed/requeue-all` | Bulk-reset every episode in `failed` state. Backs the Runtime page's "Retry failed" button. Returns `{requeued: <count>}`. |
| GET | `/dashboard/api/benchmarks` | List benchmark runs (most recent first) |
| GET | `/dashboard/api/benchmarks/{id}` | Benchmark comparison detail (A/B/C conditions per task) |
| POST | `/dashboard/api/benchmarks/run` | Trigger a new benchmark run |
| GET | `/dashboard/api/stream/telemetry` | Server-Sent Events stream — `TelemetrySnapshot` every 1 s. Powers the Runtime page. |

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
├── CLAUDE.md                           # Project context auto-loaded by Claude Code sessions
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
│   │   │   ├── mcp.rs                  # Per-tool REST handlers for /mcp/loom_*
│   │   │   ├── mcp_rpc.rs              # MCP JSON-RPC 2.0 dispatcher at POST /mcp (ADR-008)
│   │   │   ├── rest.rs                 # REST API + GitHub webhooks
│   │   │   ├── dashboard.rs            # Dashboard API handlers
│   │   │   └── auth.rs                 # Bearer token middleware
│   │   ├── worker/
│   │   │   ├── processor.rs            # Background processing (claim, extract, retry/backoff per ADR-007)
│   │   │   └── scheduler.rs            # Periodic tasks (snapshots, health)
│   │   ├── telemetry/                  # SSE telemetry sampler + ring buffers (ADR-010)
│   │   │   ├── state.rs                # In-process state, sparkline ring buffers, error tail
│   │   │   └── sampler.rs              # 1 Hz / 5 s background sampler
│   │   └── types/                      # Shared data types (serde)
│   │       ├── episode.rs
│   │       ├── entity.rs
│   │       ├── fact.rs
│   │       ├── predicate.rs
│   │       ├── classification.rs
│   │       ├── compilation.rs
│   │       ├── audit.rs
│   │       └── mcp.rs
│   ├── migrations/                     # PostgreSQL migrations (001-016)
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
│   ├── adr/                            # Architecture Decision Records
│   │   ├── 000-template.md
│   │   ├── 001-postgresql-single-store.md
│   │   ├── 002-local-llm-inference.md          # + native-Ollama amendment
│   │   ├── 003-namespace-isolation.md
│   │   ├── 004-ingestion-modes.md
│   │   ├── 005-verbatim-content-invariant.md
│   │   ├── 006-dashboard-auth-injection.md
│   │   ├── 007-episode-processing-retry-backoff.md   # poison-pill bounded retries
│   │   ├── 008-mcp-wire-protocol.md                  # JSON-RPC 2.0 dispatcher at POST /mcp
│   │   ├── 009-extraction-model-for-igpu.md          # qwen2.5:14b for shared-memory hosts
│   │   ├── 010-streaming-telemetry.md                # SSE Runtime page
│   │   └── 011-bounded-inputs-constrained-outputs.md # 16K embed cap + json_schema extraction
│   └── clients/                        # Per-client integration guides
│       ├── README.md                   # Index + new-client checklist
│       ├── claude-code.md
│       ├── claude-desktop.md
│       ├── chatgpt-desktop.md
│       ├── github-copilot.md
│       └── m365-copilot.md
│
├── bootstrap/                          # Vendor-export parsers (Mode 2)
│   ├── README.md
│   ├── schema_assertions.py            # Shared strict-schema helper
│   ├── claude_code_parser.py           # Reference implementation
│   ├── claude_ai_parser.py             # Claude.ai / Claude Desktop export
│   ├── chatgpt_parser.py               # ChatGPT Data Controls export
│   ├── github_copilot_parser.py        # Stub — no export available
│   └── m365_copilot_parser.py          # M365 Purview audit export
│
├── templates/                          # Client-side configs + discipline blocks
│   ├── README.md
│   ├── CLAUDE.md                       # Claude Code project CLAUDE.md block
│   ├── loom-capture.sh                 # Claude Code PostSession hook
│   ├── claude_desktop_config.example.json
│   ├── claude_desktop_projects_instructions.md
│   ├── chatgpt_custom_instructions.md
│   ├── vscode_mcp.example.json
│   ├── github_copilot_instructions.md
│   ├── m365_copilot_agent_manifest.example.json
│   └── m365_copilot_instructions.md
│
├── cli/
│   └── loom-seed.py                    # Mode 1 user_authored_seed ingestion
│
└── .kiro/
    └── specs/                          # Spec-driven development
```

## Forking

Loom is personal infrastructure — PRs are not reviewed and issues are not answered. See [PROJECT-STANCE.md](PROJECT-STANCE.md) for the full rationale. If you are forking, [CONTRIBUTING.md](CONTRIBUTING.md) documents the conventions I follow internally so your fork can stay consistent if that helps you.

## License

MIT — see [LICENSE](LICENSE). The license is MIT specifically so forks can evolve in whatever direction their maintainers need without negotiating with me.
