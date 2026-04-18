# Project Loom
## A PostgreSQL-Native Memory Compiler for AI Workflows
### Evidence-Grounded Memory · Strict Scoping · Shallow Graph · Inspectable Context Assembly
### *"Weaving threads of knowledge into fabric."*

---

## What This Document Is

This is the implementation specification for the Loom MVP. It replaces all prior design documents. Every decision here is scoped to what ships first, with later-phase capabilities explicitly deferred and marked as such.

The central principle:

> **Keep the architecture. Narrow the build. Prove retrieval quality before adding memory sophistication.**

---

## Technology Stack

### Application Platform

| Layer | Technology | Rationale |
|-------|-----------|-----------|
| **Engine** | Rust (tokio async runtime) | True concurrency without GIL. Compile-time SQL checking via sqlx. Strict JSON deserialization via serde. Single static binary (~20MB Docker image). Predictable latency for the online pipeline. |
| **Dashboard UI** | TypeScript + React (Vite) | Interactive drill-down views, real-time updates, graph visualization. Deployed as static files. |
| **Database** | PostgreSQL 16 + pgvector + pgAudit | Single system of record. Vector similarity, audit logging, recursive CTEs for graph traversal. |
| **LLM Inference** | Ollama (local) | Gemma 4 26B MoE for extraction, Gemma 4 E4B for classification, nomic-embed-text for embeddings. Zero cloud dependency for inference. |
| **Bootstrap Scripts** | Python | Run-once data transformation. Parsing various export formats (Claude.ai, ChatGPT, Codex CLI). Not a long-running service. |
| **Build Tooling** | Claude Code + Kiro | Claude Code for implementation. Kiro for spec refinement and task breakdown. |

### Key Rust Crates

| Crate | Purpose |
|-------|---------|
| `axum` | HTTP framework (MCP, REST, dashboard API) |
| `tokio` | Async runtime (server, background workers, scheduled tasks) |
| `sqlx` | Compile-time checked PostgreSQL queries with async support |
| `pgvector` | Vector type support for sqlx |
| `serde` / `serde_json` | Strict JSON serialization/deserialization for LLM responses, MCP protocol, audit JSONB |
| `reqwest` | HTTP client for Ollama API calls |
| `sha2` | SHA-256 content hashing for episode dedup |
| `uuid` | UUID generation for primary keys |
| `chrono` | Timestamp handling |
| `tracing` | Structured logging with span-based instrumentation |
| `tower` | Middleware (auth, request tracing, CORS) |

### Deployment Topology

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

---

## Scope: What Ships in MVP

| Ships | Does Not Ship |
|-------|--------------|
| PostgreSQL schema (episodes, entities, facts, predicates, resolution tracking) | Advanced procedural mining |
| pgvector embeddings (episodes only initially) | Broad connector network (7+ sources) |
| Temporal facts with provenance | Sophisticated memify/compaction |
| Canonical predicate registry with predicate packs and candidate tracking | Many retrieval modes |
| Three-pass entity resolution with conflict detection | Deep graph traversal (3+ hops) |
| Strict namespace isolation | Elaborate model packaging matrix |
| Hot + warm tiers with configurable per-namespace budgets | Git-style memory version-control |
| 4-stage context compiler with dual-profile retrieval | Restorable compression in output |
| Memory weight modifiers per task class | Advanced salience self-improvement |
| Extraction quality evaluation protocol (50-episode gate) | Sleep-time memify scheduler |
| Operational dashboard (trace viewer, conflict queue, metrics) | ChatGPT / Copilot / Azure DevOps connectors |
| MCP interface (loom_think, loom_learn, loom_recall) | Browser extensions |
| Audit log with per-request trace, classification confidence, profile execution | |
| Claude Code integration (primary) | |
| Manual ingestion path | |
| Local LLM inference via Ollama (Gemma 4) | |

---

## Architecture

### System of Record

PostgreSQL is the single system of record. There is no second database. There is no external vector store. There is no graph database.

- Graph traversal: recursive CTEs, constrained to 1-2 hops
- Embeddings: pgvector extension
- Audit: pgAudit extension
- Compliance: existing SOX patterns

Auxiliary caches, specialized graph infrastructure, or external vector stores are future escape hatches, not part of this plan. They are introduced only if a measured bottleneck appears first.

### Two Pipelines (Strictly Separated)

**Online pipeline** (serves user queries, latency-sensitive):
1. Intent classification (primary + secondary class with confidence)
2. Namespace resolution
3. Determine retrieval profiles (1-3, merged from primary + secondary class)
4. Run profiles in parallel via tokio::join!
5. Apply memory weight modifiers per task class
6. Rank and trim on four scored dimensions
7. Compile package
8. Audit log

**Offline pipeline** (learns from episodes, runs asynchronously via tokio spawned tasks, never blocks queries):
1. Ingest episode
2. Enforce idempotency / deduplicate
3. Extract entities (structured prompt, constrained types)
4. Resolve entities (three-pass: exact → alias → semantic)
5. Extract candidate facts (canonical predicate registry)
6. Link facts to source episode(s)
7. Resolve supersession / currentness
8. Compute derived ranking state
9. Optionally flag candidate procedures
10. Snapshot hot-tier state periodically
11. Log extraction metrics per episode

These pipelines share a database connection pool (sqlx::PgPool) but do not share runtime priority. Online queries use a separate connection pool or priority queue to ensure offline processing never starves the serving path.

### Rust Application Structure

```
loom-engine/
├── Cargo.toml
├── src/
│   ├── main.rs                 # tokio::main, axum router setup
│   ├── config.rs               # Configuration (env vars, model endpoints)
│   ├── db/
│   │   ├── mod.rs
│   │   ├── pool.rs             # sqlx::PgPool initialization
│   │   ├── episodes.rs         # Episode CRUD queries
│   │   ├── entities.rs         # Entity CRUD + resolution queries
│   │   ├── facts.rs            # Fact CRUD + supersession queries
│   │   ├── predicates.rs       # Predicate registry queries
│   │   ├── procedures.rs       # Procedure queries
│   │   ├── audit.rs            # Audit log writes
│   │   ├── snapshots.rs        # Hot-tier snapshot queries
│   │   ├── traverse.rs         # Graph traversal (calls loom_traverse SQL function)
│   │   └── dashboard.rs        # Read-only dashboard data queries
│   ├── llm/
│   │   ├── mod.rs
│   │   ├── client.rs           # Ollama HTTP client (reqwest)
│   │   ├── embeddings.rs       # Embedding generation
│   │   ├── extraction.rs       # Entity + fact extraction prompt execution
│   │   └── classification.rs   # Intent classification
│   ├── pipeline/
│   │   ├── mod.rs
│   │   ├── offline/
│   │   │   ├── mod.rs
│   │   │   ├── ingest.rs       # Episode ingestion + dedup
│   │   │   ├── extract.rs      # Entity + fact extraction orchestration
│   │   │   ├── resolve.rs      # Three-pass entity resolution
│   │   │   ├── supersede.rs    # Fact supersession detection
│   │   │   ├── state.rs        # Derived state computation + tier management
│   │   │   └── procedures.rs   # Candidate procedure flagging
│   │   └── online/
│   │       ├── mod.rs
│   │       ├── classify.rs     # Intent classification stage
│   │       ├── namespace.rs    # Namespace resolution
│   │       ├── retrieve.rs     # Retrieval profile execution (parallel)
│   │       ├── weight.rs       # Memory weight modifiers
│   │       ├── rank.rs         # 4-dimension ranking + trimming
│   │       └── compile.rs      # Package compilation (structured + compact)
│   ├── api/
│   │   ├── mod.rs
│   │   ├── mcp.rs              # MCP JSON-RPC endpoint (loom_think, loom_learn, loom_recall)
│   │   ├── rest.rs             # REST endpoint (/api/learn, /api/health)
│   │   ├── dashboard.rs        # Dashboard API endpoints (read-only)
│   │   └── auth.rs             # Bearer token middleware
│   ├── worker/
│   │   ├── mod.rs
│   │   ├── processor.rs        # Background episode processing loop
│   │   └── scheduler.rs        # Periodic tasks (snapshots, tier promotion)
│   └── types/
│       ├── mod.rs
│       ├── episode.rs          # Episode structs + serde
│       ├── entity.rs           # Entity structs + resolution types
│       ├── fact.rs             # Fact structs + evidence types
│       ├── predicate.rs        # PredicateEntry, PredicatePack, pack query types
│       ├── classification.rs   # ClassificationResult, TaskClass enum
│       ├── compilation.rs      # CompiledPackage, OutputFormat
│       ├── audit.rs            # AuditLogEntry struct
│       └── mcp.rs              # MCP protocol types (JSON-RPC request/response)
├── migrations/
│   ├── 001_episodes.sql
│   ├── 002_entities.sql
│   ├── 003_predicate_packs.sql         # loom_predicate_packs table + core pack seed
│   ├── 004_predicates.sql              # loom_predicates with pack column
│   ├── 005_facts.sql
│   ├── 006_procedures.sql
│   ├── 007_resolution_conflicts.sql
│   ├── 008_namespace_config.sql        # includes predicate_packs column
│   ├── 009_audit_log.sql
│   ├── 010_snapshots.sql
│   ├── 011_traverse_function.sql
│   ├── 012_seed_core_predicates.sql
│   └── 013_seed_grc_pack.sql           # GRC pack + predicates (seeded, not auto-assigned)
├── prompts/
│   ├── entity_extraction.txt   # Loaded at startup, not compiled
│   ├── fact_extraction.txt
│   └── classification.txt
└── Dockerfile                  # Multi-stage: builder + scratch/distroless
```

---

## Memory Model

### Three Memory Types

| Type | Role | Storage | Canonical Status | Retrieval Priority by Task |
|------|------|---------|-----------------|---------------------------|
| **Episodic** | Raw interaction records. Immutable evidence. Strongest audit anchor. | `loom_episodes` | **Immutable evidence** | Debug: primary. Compliance: primary. Architecture: secondary. |
| **Semantic** | Extracted facts and entity relationships. Derived from episodes. Revisable. | `loom_entities` + `loom_facts` | **Derived representation** | Architecture: primary. Debug: secondary. Compliance: secondary. |
| **Procedural** | Repeated behavioral patterns. Inferred from episodes. Most provisional. | `loom_procedures` | **Derived abstraction (candidate)** | Debug: secondary. Writing: secondary. All others: excluded unless high-confidence. |

**Authority hierarchy**: Episodes > Facts > Procedures. Facts are never more authoritative than their source episodes. Procedures are candidate patterns until promoted.

### Two Memory Tiers (MVP)

Cold tier is deferred. MVP ships hot and warm only.

| Tier | Behavior | Promotion Rule | Demotion Rule |
|------|----------|---------------|--------------|
| **Hot** | Always injected into every compiled context package. Budget configurable per namespace (default 500 tokens). | Explicit pin by user, OR retrieved and used in 5+ compilations within 14 days. | Unpinned by user, OR not retrieved in 30 days, OR superseded. |
| **Warm** | Retrieved per-query based on relevance. Default tier for all new memory. | N/A (default state). | Superseded facts → archived. Facts not accessed in 90 days → archived (logged, searchable, not auto-retrieved). |

Rules that are absolute:
- New facts start warm. Never hot by default.
- Superseded facts cannot be hot.
- Procedures cannot be hot until observed in ≥3 distinct episodes across ≥7 days AND confidence ≥0.8.
- Hot tier is capped at the namespace's configured budget (default 500 tokens). Overflow forces demotion of lowest-salience hot item.

### Namespace Isolation

**Hard isolation by default.** No exceptions in MVP.

- Every entity, fact, episode, and procedure belongs to exactly one namespace.
- Queries are scoped to one namespace. Cross-namespace retrieval is not supported.
- A `default` namespace exists for general knowledge not tied to a project.
- Entities cannot span namespaces. If the same real-world entity (e.g., "APIM") appears in two projects, it exists as two separate entity records.
- Hot-tier content is namespace-scoped. There is no global hot tier.

This is restrictive. It is intentionally restrictive. Cross-namespace features can be added later with explicit design, not as an accident. Track how often users hit the namespace wall during benchmarking. If > 10% of queries show cross-namespace intent, revisit isolation design in Phase 2.

---

## Schema

### Canonical vs. Derived Fields

Every table clearly separates canonical data (the truth) from derived serving state (computed, recomputable, disposable). All queries are compile-time checked via sqlx against this schema.

```sql
-- ========================================
-- EPISODES (Immutable Evidence Layer)
-- ========================================
CREATE TABLE loom_episodes (
  -- CANONICAL
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  source          TEXT NOT NULL,
  source_id       TEXT,
  source_event_id TEXT,
  content         TEXT NOT NULL,
  content_hash    TEXT NOT NULL,
  occurred_at     TIMESTAMPTZ NOT NULL,
  ingested_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
  namespace       TEXT NOT NULL,
  metadata        JSONB DEFAULT '{}',
  participants    TEXT[],
  -- EXTRACTION LINEAGE
  extraction_model TEXT,
  classification_model TEXT,
  extraction_metrics JSONB,              -- ExtractionMetrics struct serialized per ingestion
  -- DERIVED (recomputable)
  embedding       vector(768),
  tags            TEXT[],
  processed       BOOLEAN DEFAULT false,
  -- DELETION SEMANTICS
  deleted_at      TIMESTAMPTZ,
  deletion_reason TEXT,
  
  UNIQUE(source, source_event_id)
);

CREATE INDEX idx_episodes_embedding ON loom_episodes 
  USING ivfflat (embedding vector_cosine_ops) WHERE deleted_at IS NULL;
CREATE INDEX idx_episodes_ns_occurred ON loom_episodes (namespace, occurred_at DESC) 
  WHERE deleted_at IS NULL;
CREATE INDEX idx_episodes_hash ON loom_episodes (content_hash);
CREATE INDEX idx_episodes_unprocessed ON loom_episodes (ingested_at) 
  WHERE processed = false AND deleted_at IS NULL;

-- ========================================
-- ENTITIES (Graph Nodes)
-- ========================================
CREATE TABLE loom_entities (
  -- CANONICAL
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  name            TEXT NOT NULL,
  entity_type     TEXT NOT NULL CHECK (entity_type IN (
    'person', 'organization', 'project', 'service', 'technology',
    'pattern', 'environment', 'document', 'metric', 'decision'
  )),
  namespace       TEXT NOT NULL,
  properties      JSONB DEFAULT '{}',
  created_at      TIMESTAMPTZ DEFAULT now(),
  source_episodes UUID[],
  -- DELETION
  deleted_at      TIMESTAMPTZ,
  
  UNIQUE(name, entity_type, namespace)
);

CREATE INDEX idx_entities_ns_type ON loom_entities (namespace, entity_type) 
  WHERE deleted_at IS NULL;
CREATE INDEX idx_entities_aliases ON loom_entities 
  USING gin ((properties->'aliases')) WHERE deleted_at IS NULL;

-- ========================================
-- ENTITY SERVING STATE (Derived)
-- ========================================
CREATE TABLE loom_entity_state (
  entity_id       UUID PRIMARY KEY REFERENCES loom_entities(id),
  embedding       vector(768),
  summary         TEXT,
  tier            TEXT DEFAULT 'warm' CHECK (tier IN ('hot', 'warm')),
  salience_score  FLOAT DEFAULT 0.5,
  access_count    INT DEFAULT 0,
  last_accessed   TIMESTAMPTZ,
  pinned          BOOLEAN DEFAULT false,
  updated_at      TIMESTAMPTZ DEFAULT now()
);

-- ========================================
-- PREDICATE PACKS (Domain vocabulary sets)
-- ========================================
CREATE TABLE loom_predicate_packs (
  pack            TEXT PRIMARY KEY,         -- 'core', 'grc', 'healthcare', 'finserv'
  description     TEXT,
  created_at      TIMESTAMPTZ DEFAULT now()
);

INSERT INTO loom_predicate_packs (pack, description) VALUES
  ('core', 'General-purpose predicates for software engineering, architecture, and operations');

-- ========================================
-- CANONICAL PREDICATE REGISTRY
-- ========================================
CREATE TABLE loom_predicates (
  predicate       TEXT PRIMARY KEY,
  category        TEXT NOT NULL CHECK (category IN (
    'structural', 'temporal', 'decisional', 'operational',
    'regulatory'
  )),
  pack            TEXT NOT NULL DEFAULT 'core'
                  REFERENCES loom_predicate_packs(pack),
  inverse         TEXT,
  description     TEXT,
  usage_count     INT DEFAULT 0,
  created_at      TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX idx_predicates_pack ON loom_predicates (pack);

INSERT INTO loom_predicates (predicate, category, pack, inverse, description) VALUES
  ('uses',            'structural',  'core', 'used_by',        'Subject actively uses object'),
  ('used_by',         'structural',  'core', 'uses',           'Subject is used by object'),
  ('contains',        'structural',  'core', 'contained_in',   'Subject contains object as component'),
  ('contained_in',    'structural',  'core', 'contains',       'Subject is contained within object'),
  ('depends_on',      'structural',  'core', 'dependency_of',  'Subject requires object to function'),
  ('dependency_of',   'structural',  'core', 'depends_on',     'Subject is a dependency of object'),
  ('replaced_by',     'temporal',    'core', 'replaced',       'Subject was replaced by object'),
  ('replaced',        'temporal',    'core', 'replaced_by',    'Subject replaced object'),
  ('deployed_to',     'operational', 'core', 'hosts',          'Subject is deployed to object'),
  ('hosts',           'operational', 'core', 'deployed_to',    'Subject hosts object'),
  ('implements',      'structural',  'core', 'implemented_by', 'Subject implements object'),
  ('implemented_by',  'structural',  'core', 'implements',     'Subject is implemented by object'),
  ('decided',         'decisional',  'core', 'decided_by',     'Subject made decision about object'),
  ('decided_by',      'decisional',  'core', 'decided',        'Subject was decided by object'),
  ('integrates_with', 'structural',  'core', 'integrates_with','Bidirectional integration'),
  ('targets',         'operational', 'core', NULL,             'Subject targets object (customer/market)'),
  ('manages',         'operational', 'core', 'managed_by',     'Subject manages object'),
  ('managed_by',      'operational', 'core', 'manages',        'Subject is managed by object'),
  ('configured_with', 'operational', 'core', NULL,             'Subject is configured using object'),
  ('blocked_by',      'operational', 'core', 'blocks',         'Subject is blocked by object'),
  ('blocks',          'operational', 'core', 'blocked_by',     'Subject blocks object'),
  ('authored_by',     'structural',  'core', 'authored',       'Subject was created by object'),
  ('authored',        'structural',  'core', 'authored_by',    'Subject created object'),
  ('owns',            'structural',  'core', 'owned_by',       'Subject owns object'),
  ('owned_by',        'structural',  'core', 'owns',           'Subject is owned by object');

-- ========================================
-- GRC PREDICATE PACK (Domain: Governance, Risk, Compliance)
-- ========================================
INSERT INTO loom_predicate_packs (pack, description) VALUES
  ('grc', 'Governance, risk, and compliance predicates for regulated environments');

INSERT INTO loom_predicates (predicate, category, pack, inverse, description) VALUES
  ('scoped_as',                  'regulatory', 'grc', 'scoping_includes',            'Subject is scoped as object (in-scope, out-of-scope, connected-to)'),
  ('scoping_includes',           'regulatory', 'grc', 'scoped_as',                   'Scope boundary includes subject'),
  ('de-scoped_from',             'regulatory', 'grc', NULL,                          'Subject was removed from object scope boundary'),
  ('exception_granted_for',      'regulatory', 'grc', 'exception_applies_to',        'Subject has an approved exception for object requirement'),
  ('exception_applies_to',       'regulatory', 'grc', 'exception_granted_for',       'Exception on subject applies to object'),
  ('maps_to_control',            'regulatory', 'grc', 'control_mapped_from',         'Subject maps to object control requirement'),
  ('control_mapped_from',        'regulatory', 'grc', 'maps_to_control',             'Control on subject is mapped from object'),
  ('evidenced_by',               'regulatory', 'grc', 'evidence_for',                'Subject requirement is evidenced by object artifact'),
  ('evidence_for',               'regulatory', 'grc', 'evidenced_by',                'Subject artifact is evidence for object requirement'),
  ('satisfies',                  'regulatory', 'grc', 'satisfied_by',                'Subject control satisfies object requirement'),
  ('satisfied_by',               'regulatory', 'grc', 'satisfies',                   'Subject requirement is satisfied by object control'),
  ('precedent_set_by',           'regulatory', 'grc', 'sets_precedent_for',          'Subject decision was precedented by object prior decision'),
  ('sets_precedent_for',         'regulatory', 'grc', 'precedent_set_by',            'Subject decision sets precedent for object future decisions'),
  ('finding_on',                 'regulatory', 'grc', 'finding_raised_by',           'Subject finding applies to object system or control'),
  ('finding_raised_by',          'regulatory', 'grc', 'finding_on',                  'Subject system has finding raised by object assessment'),
  ('conflicts_with',             'regulatory', 'grc', 'conflicts_with',              'Bidirectional framework requirement conflict'),
  ('supplements',                'regulatory', 'grc', 'supplemented_by',             'Subject requirement supplements object requirement'),
  ('supplemented_by',            'regulatory', 'grc', 'supplements',                 'Subject requirement is supplemented by object'),
  ('fills_gap_in',               'regulatory', 'grc', 'gap_filled_by',              'Subject control fills a gap in object framework'),
  ('gap_filled_by',              'regulatory', 'grc', 'fills_gap_in',               'Subject framework has gap filled by object control'),
  ('supersedes_in_context',      'regulatory', 'grc', 'superseded_in_context_by',    'Subject interpretation supersedes object in current regulatory context'),
  ('superseded_in_context_by',   'regulatory', 'grc', 'supersedes_in_context',       'Subject interpretation is superseded by object in current context'),
  ('compensated_by',             'regulatory', 'grc', 'compensates_for',             'Subject requirement is compensated by object control'),
  ('compensates_for',            'regulatory', 'grc', 'compensated_by',              'Subject control compensates for object requirement');

-- ========================================
-- PREDICATE CANDIDATES (Custom predicates awaiting review)
-- ========================================
CREATE TABLE loom_predicate_candidates (
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  predicate       TEXT NOT NULL,
  occurrences     INT DEFAULT 1,
  example_facts   UUID[],
  mapped_to       TEXT REFERENCES loom_predicates(predicate),
  promoted_to_pack TEXT REFERENCES loom_predicate_packs(pack),  -- target pack when promoted
  created_at      TIMESTAMPTZ DEFAULT now(),
  resolved_at     TIMESTAMPTZ
);

-- ========================================
-- FACTS (Graph Edges)
-- ========================================
CREATE TABLE loom_facts (
  -- CANONICAL
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  subject_id      UUID NOT NULL REFERENCES loom_entities(id),
  predicate       TEXT NOT NULL,
  object_id       UUID NOT NULL REFERENCES loom_entities(id),
  namespace       TEXT NOT NULL,
  -- TEMPORAL
  valid_from      TIMESTAMPTZ NOT NULL DEFAULT now(),
  valid_until     TIMESTAMPTZ,
  -- PROVENANCE
  source_episodes UUID[] NOT NULL,
  superseded_by   UUID REFERENCES loom_facts(id),
  -- EVIDENCE STATUS
  evidence_status TEXT NOT NULL DEFAULT 'extracted' 
    CHECK (evidence_status IN (
      'user_asserted', 'observed', 'extracted', 'inferred',
      'promoted', 'deprecated', 'superseded'
    )),
  evidence_strength TEXT CHECK (evidence_strength IN ('explicit', 'implied')),
  properties      JSONB DEFAULT '{}',
  created_at      TIMESTAMPTZ DEFAULT now(),
  deleted_at      TIMESTAMPTZ
);

CREATE INDEX idx_facts_current ON loom_facts (namespace, subject_id) 
  WHERE valid_until IS NULL AND deleted_at IS NULL;
CREATE INDEX idx_facts_object ON loom_facts (object_id) 
  WHERE valid_until IS NULL AND deleted_at IS NULL;
CREATE INDEX idx_facts_status ON loom_facts (evidence_status);
CREATE INDEX idx_facts_predicate ON loom_facts (predicate) 
  WHERE valid_until IS NULL AND deleted_at IS NULL;

-- ========================================
-- FACT SERVING STATE (Derived)
-- ========================================
CREATE TABLE loom_fact_state (
  fact_id         UUID PRIMARY KEY REFERENCES loom_facts(id),
  embedding       vector(768),
  salience_score  FLOAT DEFAULT 0.5,
  access_count    INT DEFAULT 0,
  last_accessed   TIMESTAMPTZ,
  tier            TEXT DEFAULT 'warm' CHECK (tier IN ('hot', 'warm')),
  pinned          BOOLEAN DEFAULT false,
  updated_at      TIMESTAMPTZ DEFAULT now()
);

-- ========================================
-- PROCEDURES (Candidate Patterns)
-- ========================================
CREATE TABLE loom_procedures (
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  pattern         TEXT NOT NULL,
  category        TEXT,
  namespace       TEXT NOT NULL,
  source_episodes UUID[] NOT NULL,
  first_observed  TIMESTAMPTZ DEFAULT now(),
  last_observed   TIMESTAMPTZ DEFAULT now(),
  observation_count INT DEFAULT 1,
  evidence_status TEXT NOT NULL DEFAULT 'extracted'
    CHECK (evidence_status IN ('extracted', 'promoted', 'deprecated')),
  confidence      FLOAT DEFAULT 0.3,
  embedding       vector(768),
  tier            TEXT DEFAULT 'warm' CHECK (tier IN ('hot', 'warm')),
  deleted_at      TIMESTAMPTZ
);

-- ========================================
-- ENTITY RESOLUTION CONFLICT TRACKING
-- ========================================
CREATE TABLE loom_resolution_conflicts (
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  entity_name     TEXT NOT NULL,
  entity_type     TEXT NOT NULL,
  namespace       TEXT NOT NULL,
  candidates      JSONB NOT NULL,
  resolved        BOOLEAN DEFAULT false,
  resolution      TEXT,
  resolved_at     TIMESTAMPTZ,
  created_at      TIMESTAMPTZ DEFAULT now()
);

-- ========================================
-- NAMESPACE CONFIGURATION
-- ========================================
CREATE TABLE loom_namespace_config (
  namespace         TEXT PRIMARY KEY,
  hot_tier_budget   INT DEFAULT 500,
  warm_tier_budget  INT DEFAULT 3000,
  predicate_packs   TEXT[] DEFAULT '{core}',   -- which predicate packs this namespace uses
  description       TEXT,
  created_at        TIMESTAMPTZ DEFAULT now(),
  updated_at        TIMESTAMPTZ DEFAULT now()
);

-- ========================================
-- AUDIT LOG
-- ========================================
CREATE TABLE loom_audit_log (
  id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  created_at        TIMESTAMPTZ DEFAULT now(),
  task_class        TEXT NOT NULL,
  namespace         TEXT NOT NULL,
  query_text        TEXT,
  target_model      TEXT,
  primary_class     TEXT NOT NULL,
  secondary_class   TEXT,
  primary_confidence FLOAT,
  secondary_confidence FLOAT,
  profiles_executed TEXT[],
  retrieval_profile TEXT NOT NULL,
  candidates_found  INT,
  candidates_selected INT,
  candidates_rejected INT,
  selected_items    JSONB,
  rejected_items    JSONB,
  compiled_tokens   INT,
  output_format     TEXT,
  latency_total_ms  INT,
  latency_classify_ms INT,
  latency_retrieve_ms INT,
  latency_rank_ms   INT,
  latency_compile_ms INT,
  user_rating       FLOAT
);

-- ========================================
-- HOT TIER SNAPSHOTS
-- ========================================
CREATE TABLE loom_snapshots (
  id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  snapshot_at     TIMESTAMPTZ DEFAULT now(),
  namespace       TEXT NOT NULL,
  hot_entities    JSONB,
  hot_facts       JSONB,
  hot_procedures  JSONB,
  total_tokens    INT
);

-- ========================================
-- GRAPH TRAVERSAL (1-2 hop, cycle-safe)
-- ========================================
CREATE OR REPLACE FUNCTION loom_traverse(
  p_entity_id UUID,
  p_max_hops INT DEFAULT 2,
  p_namespace TEXT DEFAULT 'default'
) RETURNS TABLE (
  entity_id UUID, entity_name TEXT, entity_type TEXT,
  fact_id UUID, predicate TEXT, evidence_status TEXT,
  hop_depth INT, path UUID[]
) AS $$
WITH RECURSIVE walk AS (
  SELECT e.id, e.name, e.entity_type,
         NULL::UUID, NULL::TEXT, NULL::TEXT,
         0, ARRAY[e.id]
  FROM loom_entities e
  WHERE e.id = p_entity_id 
    AND e.namespace = p_namespace 
    AND e.deleted_at IS NULL

  UNION ALL

  SELECT e2.id, e2.name, e2.entity_type,
         f.id, f.predicate, f.evidence_status,
         w.hop_depth + 1, w.path || e2.id
  FROM walk w
  JOIN loom_facts f ON (f.subject_id = w.entity_id OR f.object_id = w.entity_id)
  JOIN loom_entities e2 ON (
    CASE WHEN f.subject_id = w.entity_id THEN f.object_id ELSE f.subject_id END = e2.id
  )
  WHERE w.hop_depth < p_max_hops
    AND f.valid_until IS NULL
    AND f.deleted_at IS NULL
    AND f.namespace = p_namespace
    AND e2.deleted_at IS NULL
    AND NOT e2.id = ANY(w.path)
)
SELECT * FROM walk WHERE hop_depth > 0
ORDER BY hop_depth, entity_name;
$$ LANGUAGE sql STABLE;
```

---

## Model Selection Strategy (Local-First)

### Primary: Fully Local via Ollama

| Role | Model | Hardware Requirement | Rationale |
|------|-------|---------------------|-----------|
| **Extraction** | Gemma 4 26B MoE (Q4) | 24GB VRAM (RTX 4090) or 36GB+ unified memory (Apple Silicon) | Native structured JSON output, 97% of 31B Dense quality at 3.8B active params. Apache 2.0. |
| **Classification** | Gemma 4 E4B | 12GB RAM | 5-class routing with keyword hints. 100% JSON parse success, 90% schema compliance. Overkill for this task, but already running in Ollama. |
| **Embeddings** | nomic-embed-text | 2GB RAM | 768-dimension embeddings. Fast, accurate, runs on CPU. |

### Fallback: Azure OpenAI API

If Gemma 4 26B MoE fails the week 4 extraction quality gate:

| Role | Model | When |
|------|-------|------|
| **Extraction** | gpt-4.1-mini | Default API fallback |
| **Extraction** | gpt-4.1 | If mini also fails quality gate |
| **Classification** | gpt-4.1-nano | Only if E4B classification is insufficient |
| **Embeddings** | text-embedding-3-small | Only if local embeddings show retrieval quality issues |

### Embedding Dimension (Schema Constraint)

All pgvector columns in the schema are declared as `vector(768)` to match `nomic-embed-text`, the primary local embedding model. This is a deliberate choice: the schema dimension is fixed at migration time and cannot vary per-row.

If the week 4 quality gate forces a fallback to Azure OpenAI, do **not** use `text-embedding-3-small` at its default 1536 dimensions. The API supports a `dimensions` parameter for output truncation via Matryoshka representation. Set `dimensions: 768` on every embedding call so the vectors remain schema-compatible.

```rust
// When calling Azure OpenAI as fallback:
// POST /embeddings { "model": "text-embedding-3-small", "dimensions": 768, "input": "..." }
```

If local-first is swapped for an embedding model that produces a different native dimension (e.g., `nomic-embed-text-v2-moe` with Matryoshka down to 256), update the schema dimension in migrations 001, 002, 005, 006 consistently and re-embed existing content. This is a breaking change, not a config toggle.

The model name is a configuration value loaded from environment variables. Both the extraction and classification model names are stored on every episode record (`extraction_model`, `classification_model`) for lineage tracking and quality comparison across model versions.

```rust
// loom_engine/src/config.rs
pub struct LlmConfig {
    pub extraction_model: String,      // "gemma4:26b-a4b" or "gpt-4.1-mini"
    pub classification_model: String,  // "gemma4:e4b" or "gpt-4.1-nano"
    pub embedding_model: String,       // "nomic-embed-text" or "text-embedding-3-small"
    pub ollama_base_url: String,       // "http://ollama:11434"
    pub azure_openai_base_url: Option<String>,  // None if fully local
    pub azure_openai_api_key: Option<String>,
}
```

### Ollama API Integration

All LLM calls use the Ollama-compatible OpenAI API format. This means the same Rust HTTP client code works for both local Ollama and Azure OpenAI with minimal branching.

```rust
// loom_engine/src/llm/client.rs
pub struct LlmClient {
    http: reqwest::Client,
    config: LlmConfig,
}

impl LlmClient {
    /// Generate a chat completion (extraction, classification)
    pub async fn chat_completion(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
    ) -> Result<String, LlmError> {
        // POST to {base_url}/v1/chat/completions
        // Ollama and Azure OpenAI both accept this format
    }

    /// Generate an embedding vector
    pub async fn embed(
        &self,
        model: &str,
        text: &str,
    ) -> Result<Vec<f32>, LlmError> {
        // POST to {base_url}/v1/embeddings
    }
}
```

---

## Extraction Pipeline

### Design Principle

Extraction quality is the foundation. If entities are wrong, the graph is wrong. If facts are wrong, compilation is wrong. Extraction is not a step in the pipeline. It is the pipeline.

### Prompt Management

Prompts are stored as text files in `prompts/`, loaded at engine startup, and cached in memory. This enables prompt iteration without recompilation.

```rust
// loom_engine/src/llm/extraction.rs
pub struct ExtractionPrompts {
    pub entity: String,     // loaded from prompts/entity_extraction.txt
    pub fact: String,       // loaded from prompts/fact_extraction.txt
    pub classify: String,   // loaded from prompts/classification.txt
}
```

### Entity Extraction Prompt

```
Extract entities from this episode. Return JSON only, no preamble.

Rules:
1. Entity types must be one of: person, organization, project, service, 
   technology, pattern, environment, document, metric, decision
2. Use the most specific common name (e.g., "APIM" not 
   "Azure API Management" unless the full name is the established 
   usage in context)
3. Do not extract generic concepts as entities (e.g., "authentication" 
   is not an entity; "APIM Tri-Auth Pattern" is)
4. Do not extract actions or events as entities

Output format:
{
  "entities": [
    {"name": "APIM", "type": "service", "aliases": ["Azure API Management"]},
    {"name": "Project Sentinel", "type": "project", "aliases": ["Sentinel"]}
  ]
}
```

### Fact Extraction Prompt

```
Extract factual relationships from this episode. Return JSON only, no preamble.

Rules:
1. Each fact is a (subject, predicate, object) triple where subject and object 
   are entities from the provided entity list
2. Predicates must use the canonical predicate list below. If no canonical 
   predicate fits, use a new one but flag it as "custom": true
3. Include temporal markers if the text indicates when something started, 
   ended, or changed
4. Include evidence_strength: "explicit" if the text directly states the 
   relationship, "implied" if it must be inferred

{predicate_block}

Output format:
{
  "facts": [
    {
      "subject": "Project Sentinel",
      "predicate": "uses",
      "object": "Semantic Kernel",
      "evidence_strength": "explicit",
      "temporal": {"valid_from": "2025-09-01"},
      "custom": false
    }
  ]
}
```

The `{predicate_block}` placeholder is replaced at runtime with predicates from the namespace's configured packs. Predicates are loaded from `loom_predicates` where `pack = ANY(namespace_packs)`, grouped by pack with section headers so the LLM distinguishes core from domain predicates. The `core` pack is always included regardless of namespace configuration (enforced in engine, not schema).

For a namespace using `{core}` only:
```
Core predicates:
- uses, used_by
- contains, contained_in
- depends_on, dependency_of
...
```

For a namespace using `{core, grc}`:
```
Core predicates:
- uses, used_by
- contains, contained_in
...

GRC predicates (domain pack):
- scoped_as, scoping_includes
- maps_to_control, control_mapped_from
- exception_granted_for, exception_applies_to
...
```

### Strict Response Deserialization (Rust Advantage)

LLM responses are deserialized into strict Rust types via serde. Malformed responses fail at the type boundary with clear error messages, not as runtime panics deep in the pipeline.

```rust
// loom_engine/src/types/entity.rs
#[derive(Debug, Deserialize)]
pub struct ExtractionResponse {
    pub entities: Vec<ExtractedEntity>,
}

#[derive(Debug, Deserialize)]
pub struct ExtractedEntity {
    pub name: String,
    #[serde(rename = "type")]
    pub entity_type: EntityType,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntityType {
    Person, Organization, Project, Service, Technology,
    Pattern, Environment, Document, Metric, Decision,
}
```

If the LLM returns an invalid entity type, serde rejects it at parse time. If a required field is missing, the error message names the field. This eliminates an entire category of bugs that Python catches only at runtime (or not at all).

### Custom Predicate Handling

When extraction produces a fact with `"custom": true`:
1. Check `loom_predicate_candidates` for an existing entry with the same predicate text
2. If exists, increment `occurrences` and append the fact ID to `example_facts`
3. If not exists, create a new candidate entry
4. If a candidate reaches 5+ occurrences, flag it for operator review via the dashboard
5. When promoting: the operator chooses which pack to promote into (defaults to the namespace's primary domain pack, or `core` if no domain pack is configured). The `promoted_to_pack` column on `loom_predicate_candidates` records this decision.

The fact is stored with the custom predicate regardless. Custom predicates participate in retrieval normally.

### Extraction Metrics

Every episode ingestion logs extraction metrics as a serde-serialized struct stored in JSONB:

```rust
#[derive(Debug, Serialize)]
pub struct ExtractionMetrics {
    pub entities_extracted: u32,
    pub entities_resolved_exact: u32,
    pub entities_resolved_alias: u32,
    pub entities_resolved_semantic: u32,
    pub entities_new: u32,
    pub entities_conflict_flagged: u32,
    pub facts_extracted: u32,
    pub facts_canonical_predicate: u32,
    pub facts_custom_predicate: u32,
    pub evidence_explicit: u32,
    pub evidence_implied: u32,
    pub extraction_model: String,
    pub processing_time_ms: u64,
}
```

---

## Entity Resolution Algorithm

### Design Principle

**Prefer fragmentation over collision.** Two separate entity nodes for the same real-world thing can be merged later with a single UPDATE. Two different things incorrectly merged corrupt every fact attached to both sides. Fragmentation is recoverable. Collision is not.

### Three-Pass Resolution

Resolution runs once per extracted entity, strictly ordered. A match at an earlier pass prevents later passes from executing.

```rust
// loom_engine/src/pipeline/offline/resolve.rs
pub enum ResolutionMethod {
    Exact,
    Alias,
    Semantic,
    New,
}

pub struct ResolutionResult {
    pub entity_id: Option<Uuid>,
    pub method: ResolutionMethod,
    pub confidence: f64,
    pub candidate_matches: Vec<CandidateMatch>,
}
```

**Pass 1: Exact match** on `(LOWER(name), entity_type, namespace)`. Confidence: 1.0.

**Pass 2: Alias match.** Bidirectional check: extracted name in existing aliases, OR extracted aliases match existing names. Single match → merge aliases (append-only). Multiple matches → fall through to Pass 3.

**Pass 3: Semantic similarity.** Embed entity name + context snippet (first 200 chars). Cosine similarity against existing entities of same type in same namespace. Threshold: **0.92**.

Safety rules:
- Top candidate > 0.92 AND gap to second >= 0.03 → merge
- Top two within 0.03 → create new entity, log conflict to `loom_resolution_conflicts`
- No candidate > 0.92 → create new entity. Confidence: 1.0 (confident it's new)

### Entity Health Check (Weekly)

```sql
SELECT a.name, b.name, a.entity_type, a.namespace,
       1 - (sa.embedding <=> sb.embedding) as similarity
FROM loom_entities a
JOIN loom_entity_state sa ON sa.entity_id = a.id
JOIN loom_entities b ON b.namespace = a.namespace 
    AND b.entity_type = a.entity_type AND b.id > a.id
JOIN loom_entity_state sb ON sb.entity_id = b.id
WHERE a.deleted_at IS NULL AND b.deleted_at IS NULL
  AND 1 - (sa.embedding <=> sb.embedding) > 0.85
ORDER BY similarity DESC
LIMIT 50;
```

Results surfaced in the dashboard conflict review queue.

---

## Online Pipeline: Context Compiler

### Stage 1: Intent Classification

```rust
#[derive(Debug)]
pub struct ClassificationResult {
    pub primary_class: TaskClass,
    pub secondary_class: Option<TaskClass>,
    pub primary_confidence: f64,
    pub secondary_confidence: Option<f64>,
}

#[derive(Debug, Clone, Copy)]
pub enum TaskClass {
    Debug,
    Architecture,
    Compliance,
    Writing,
    Chat,
}
```

If the confidence gap between primary and secondary is < 0.3, both classes inform retrieval. If the gap is >= 0.3, only the primary class is used. If ambiguous with no clear signal, default to `Chat`.

### Stage 2: Namespace Resolution

1. Explicit parameter in MCP call (highest priority)
2. Project context from Claude Code / Codex working directory
3. Default namespace

### Stage 3: Retrieval Profiles (Parallel via tokio::join!)

```rust
const PROFILE_MAP: &[(&str, &[&str])] = &[
    ("debug",        &["graph_neighborhood", "episode_recall"]),
    ("architecture", &["fact_lookup", "graph_neighborhood"]),
    ("compliance",   &["episode_recall", "fact_lookup"]),
    ("writing",      &["fact_lookup"]),
    ("chat",         &["fact_lookup"]),
];
```

Profiles execute via `tokio::join!` for true parallel database queries:

```rust
let (facts, episodes, graph) = tokio::join!(
    fact_lookup(&pool, &query, namespace),
    episode_recall(&pool, &query, namespace),
    graph_neighborhood(&pool, &query, namespace),
);
```

### Stage 4: Memory Weight Modifiers

```rust
const MEMORY_WEIGHTS: &[(&str, (f64, f64, f64))] = &[
    //                   episodic  semantic  procedural
    ("debug",           (1.0,      0.7,      0.8)),
    ("architecture",    (0.5,      1.0,      0.3)),
    ("compliance",      (1.0,      0.8,      0.0)),
    ("writing",         (0.3,      1.0,      0.6)),
    ("chat",            (0.4,      1.0,      0.3)),
];
```

### Stage 5: Rank and Trim

| Dimension | Weight | Meaning |
|-----------|--------|---------|
| Relevance | 0.40 | Cosine similarity, modified by memory weight |
| Recency | 0.25 | Days since last_accessed or occurred_at, decayed |
| Stability | 0.20 | Current + non-superseded + high evidence_status |
| Provenance | 0.15 | Source episode count + evidence_status authority |

### Stage 6: Compile Package

**Structured (for Claude, Claude Code, Copilot)**:
```xml
<loom model="claude" tokens="2340" ns="azure-msp" task="debug">
  <identity>Cloud Architect · Rackspace Azure Expert MSP</identity>
  <project>AMA Compliance Tracking — design phase</project>
  <knowledge>
    <fact status="observed" salience="0.94">APIM uses tri-auth pattern</fact>
  </knowledge>
  <episodes>
    <episode source="claude-code" date="2026-03-01">Fixed retry loop in APIM policy</episode>
  </episodes>
  <patterns>
    <procedure confidence="0.92">Debug auth: check DDI isolation first</procedure>
  </patterns>
</loom>
```

**Compact (for GPT, Codex, local models)**:
```json
{"ns":"azure-msp","task":"debug","identity":"Cloud Architect, Rackspace Azure MSP",
 "facts":["APIM tri-auth pattern"],
 "recent":["2026-03-01: Fixed retry loop in APIM"],
 "patterns":["Debug auth: check DDI isolation first"]}
```

### Stage 7: Audit Log

Every compilation writes a complete trace to `loom_audit_log`. This is not optional. The tracing crate instruments each stage with timing spans that are automatically captured in the audit record.

---

## Offline Pipeline: Episode Ingestion

### Step 1: Ingest Episode
Accept from MCP (`loom_learn`) or REST. Require: source, content, occurred_at, namespace.

### Step 2: Idempotency
Check `content_hash` (SHA-256 via `sha2` crate) and `(source, source_event_id)` unique constraint. If duplicate, skip and log.

### Step 3: Embed
Call Ollama embedding endpoint. Store vector on episode.

### Step 4: Extract Entities
Call Ollama chat completion with entity extraction prompt. Deserialize via serde into `ExtractionResponse`. Invalid responses caught at type boundary.

### Step 5: Resolve Entities
Three-pass resolution for each extracted entity. Log resolution method and confidence per entity.

### Step 6: Extract Candidate Facts
Call Ollama with fact extraction prompt + entity list. Deserialize into `FactExtractionResponse`. Validate predicates against registry.

### Step 7: Link and Store Facts
Each fact links to its source episode. Evidence status defaults to `extracted`.

### Step 8: Resolve Supersession
If a new fact contradicts an existing current fact (same subject + predicate + different object), mark old as `superseded`.

### Step 9: Compute Derived State
Update `loom_entity_state` and `loom_fact_state`. Recalculate salience. Apply tier rules.

### Step 10: Flag Candidate Procedures (Optional)
Detect repeated patterns. Create candidates with confidence = 0.3.

### Step 11: Log Extraction Metrics
Serialize `ExtractionMetrics` to JSONB and store on `loom_episodes.extraction_metrics` for the ingested episode. Dashboard extraction-quality queries join against this column, not the audit log.

### Step 12: Snapshot
Tokio scheduled task writes hot-tier snapshot every 24 hours.

---

## Dashboard

### Architecture

The dashboard is a Vite + React SPA that calls read-only JSON endpoints on the loom-engine. No separate backend. The engine serves both the operational API and the dashboard API from the same axum router, using separate route groups.

```rust
// loom_engine/src/api/dashboard.rs
// All endpoints are GET, read-only, no writes

Router::new()
    .route("/dashboard/api/health", get(pipeline_health))
    .route("/dashboard/api/episodes", get(list_episodes))
    .route("/dashboard/api/entities", get(list_entities))
    .route("/dashboard/api/entities/:id/graph", get(entity_graph))
    .route("/dashboard/api/facts", get(list_facts))
    .route("/dashboard/api/compilations", get(list_compilations))
    .route("/dashboard/api/compilations/:id", get(compilation_detail))
    .route("/dashboard/api/conflicts", get(list_conflicts))
    .route("/dashboard/api/conflicts/:id/resolve", post(resolve_conflict))
    .route("/dashboard/api/predicates/candidates", get(list_predicate_candidates))
    .route("/dashboard/api/predicates/candidates/:id/resolve", post(resolve_predicate))
    .route("/dashboard/api/predicates/packs", get(list_predicate_packs))
    .route("/dashboard/api/predicates/packs/:pack", get(get_pack_detail))
    .route("/dashboard/api/predicates/packs/:pack/predicates", get(list_pack_predicates))
    .route("/dashboard/api/predicates/active/:namespace", get(get_active_predicates))
    .route("/dashboard/api/metrics/precision", get(precision_over_time))
    .route("/dashboard/api/metrics/latency", get(latency_percentiles))
    .route("/dashboard/api/metrics/classification", get(classification_accuracy))
    .route("/dashboard/api/metrics/extraction", get(extraction_quality_by_model))
    .route("/dashboard/api/metrics/resolution", get(resolution_method_distribution))
    .route("/dashboard/api/metrics/hot-tier", get(hot_tier_utilization))
    .route("/dashboard/api/namespaces", get(list_namespaces))
```

Note: `resolve_conflict` and `resolve_predicate` are the only write operations on the dashboard API. They update `loom_resolution_conflicts` and `loom_predicate_candidates` respectively. `resolve_predicate` accepts an optional `target_pack` field when promoting a candidate to canonical. Pack creation and namespace pack assignment are configuration operations handled via SQL or REST admin endpoint, not the dashboard UI in MVP.

### Dashboard Views

**Pipeline Health (home page)**
- Episodes: total, by source, by namespace, pending processing
- Entities: count by type, by namespace
- Facts: current vs. superseded count
- Last ingestion timestamp, processing success/failure rate
- Queue depth (unprocessed episodes)
- Model configuration (extraction, classification, embedding)

**Compilation Trace Viewer**
- Paginated list of recent `loom_think` calls
- Each row: timestamp, query text, namespace, primary/secondary class, confidence, latency, token count
- Drill-down: full audit detail with candidates found/selected/rejected, per-candidate score breakdown (relevance, recency, stability, provenance), rejection reasons
- Visual score breakdown bar chart per candidate

**Knowledge Graph Explorer**
- Entity search by name, type, namespace
- Entity detail: properties, aliases, source episodes, facts (as subject, as object)
- Visual graph rendering: entity as node, facts as edges, 1-2 hop neighborhood
- Fact detail: temporal range, evidence status, source episodes, supersession chain

**Entity Conflict Review Queue**
- Unresolved conflicts from `loom_resolution_conflicts`
- Each conflict: entity name, type, namespace, candidate matches with scores
- Actions: merge (select target entity), keep separate, split
- Resolution logged with timestamp

**Predicate Candidate Review**
- Unmapped custom predicates from `loom_predicate_candidates`
- Each candidate: predicate text, occurrence count, example facts, suggested pack based on namespace of origin
- Actions: map to existing canonical predicate, promote to canonical (select target pack), ignore

**Predicate Pack Browser**
- List of all packs with predicate count per pack
- Drill into a pack to see all predicates, their categories, inverses, and usage counts
- Per-namespace: which packs are active and the combined predicate vocabulary
- Usage heatmap: which pack predicates are actually used vs. defined but unused

**Retrieval Quality Metrics**
- Precision over time (line chart)
- Latency percentiles: p50, p95, p99 (line chart)
- Classification confidence distribution (histogram)
- Secondary class "winning" rate (when secondary-class memory types dominate selections)
- Hot-tier utilization per namespace (bar chart)

**Extraction Quality**
- Extraction quality comparison across model versions (table)
- Entity resolution method distribution: exact / alias / semantic / new (pie chart)
- Custom predicate growth rate (line chart)
- Entity fragmentation detection results (table, from weekly health check)

**Benchmark Comparison (Week 7-8)**
- Side-by-side A/B/C condition results across 10 benchmark tasks
- Per-task: success, precision, stale fact rate, irrelevant inclusion, token cost, latency

### React Component Structure

```
loom-dashboard/
├── package.json
├── vite.config.ts
├── src/
│   ├── main.tsx
│   ├── App.tsx                   # Router + layout
│   ├── api/
│   │   └── client.ts            # Typed fetch wrapper for dashboard API
│   ├── pages/
│   │   ├── Health.tsx            # Pipeline health overview
│   │   ├── Compilations.tsx      # Compilation list + detail
│   │   ├── Graph.tsx             # Knowledge graph explorer
│   │   ├── Conflicts.tsx         # Entity conflict review queue
│   │   ├── Predicates.tsx        # Predicate candidate review + pack browser
│   │   ├── Metrics.tsx           # Retrieval quality charts
│   │   ├── Extraction.tsx        # Extraction quality dashboard
│   │   └── Benchmark.tsx         # A/B/C comparison (week 7-8)
│   ├── components/
│   │   ├── ScoreBreakdown.tsx    # 4-dimension score visualization
│   │   ├── EntityNode.tsx        # Graph node component
│   │   ├── FactEdge.tsx          # Graph edge component
│   │   ├── TimeSeriesChart.tsx   # Recharts line chart wrapper
│   │   ├── CandidateList.tsx     # Scored candidate list with expand
│   │   ├── PackBrowser.tsx       # Pack list, drill-down, usage heatmap
│   │   ├── PredicatePromote.tsx  # Promotion dialog with pack selector
│   │   └── StatusBadge.tsx       # Evidence status / tier badge
│   └── types/
│       └── index.ts              # TypeScript types mirroring Rust types
└── Dockerfile                    # Build stage + nginx/caddy static serve
```

---

## MCP Interface

Three tools. Not five.

### loom_think
```
loom_think(query, namespace?, model?, task_hint?)
→ compiled context package (structured or compact)
```

### loom_learn
```
loom_learn(content, source, namespace, occurred_at?, metadata?)
→ { episode_id, status: "accepted" | "duplicate" | "queued" }
```

### loom_recall
```
loom_recall(query, namespace, memory_type?, time_range?)
→ raw search results (not compiled)
```

`loom_inspect` and `loom_forget` are deferred. The dashboard provides inspection. Deletion is manual SQL with audit log entry.

---

## Integration Plan (Narrow)

### Primary: Claude Code (MCP Native)
```bash
claude mcp add --transport http \
  --scope user \
  --header "Authorization: Bearer ${LOOM_TOKEN}" \
  loom https://loom.yourdomain.com/mcp
```

CLAUDE.md instruction:
```
## Context
Call loom_think before complex tasks to retrieve professional context.
Call loom_learn after significant decisions or discoveries.
```

Claude Code hooks for auto-capture:
```json
{
  "hooks": {
    "PostSession": [{
      "matcher": "",
      "command": "loom-capture.sh"
    }]
  }
}
```

### Secondary: Manual Ingestion
```
POST /api/learn
Content-Type: application/json
{ "content": "...", "source": "manual", "namespace": "azure-msp" }
```

### Tertiary: Codex CLI (MCP Native)
Added in weeks 9-10 after Claude Code integration is stable.

---

## Extraction Quality Evaluation Protocol

### Pre-Benchmark Gate (End of Week 4)

**Procedure:**
1. Sample 50 episodes across sources
2. Human annotates expected entities and facts
3. Run extraction against all 50 with both Gemma 4 26B MoE (local) and gpt-4.1-mini (API)
4. Measure both:

| Metric | Target | Formula |
|--------|--------|---------|
| Entity precision | >= 0.80 | correct_entities / total_extracted_entities |
| Entity recall | >= 0.70 | correct_entities / total_expected_entities |
| Fact precision | >= 0.75 | correct_facts / total_extracted_facts |
| Fact recall | >= 0.60 | correct_facts / total_expected_facts |
| Predicate consistency | >= 0.85 | same_relationship_same_predicate / total_relationship_pairs |

**Decision gate:**
- If Gemma 4 26B MoE meets thresholds → use local model, eliminate API dependency
- If Gemma 4 fails but gpt-4.1-mini passes → use API model, plan local model upgrade path
- If both fail → iterate on extraction prompts before proceeding
- Do not proceed to Week 5 until thresholds are met with at least one model

The `extraction_model` column on every episode record enables retroactive quality comparison between models.

### Ongoing Extraction Monitoring (Post-Launch)

Surfaced in the dashboard Extraction Quality view. Alert thresholds:
- Custom predicate appears 3+ times in one week (predicate drift)
- Entity extraction produces > 20% new entities not matching existing (entity sprawl)
- Fact count per episode drops below 1.0 average (extraction regression)
- Resolution conflict rate exceeds 5% (ambiguity escalation)

---

## Evaluation Plan

### Benchmark Tasks (10 real scenarios)

| # | Task | Type | Expected Memory |
|---|------|------|----------------|
| 1 | Debug APIM service token retry issue | debug | Past retry bugs, tri-auth pattern, DDI isolation pattern |
| 2 | Design AMA v3 DCR drift detection | architecture | rax- prefix convention, OS categorization, v2 schema risks |
| 3 | What was decided about SOX compliance for AI dev? | compliance | SOX policy episodes, pgAudit decisions, approval gates |
| 4 | Write Cyber Recovery value proposition | writing | 36 customer targeting, BFSI/Healthcare focus, impact metrics |
| 5 | Which customers were discussed for Project Sentinel? | compliance | Sentinel episodes, customer entity references |
| 6 | How did we solve the MFA Phase 2 ARM issue? | debug | MFA episodes, az rest vs curl decision, ARM scope changes |
| 7 | What's the current AeroPro Texas architecture? | architecture | AeroPro entities, infrastructure facts, recent decisions |
| 8 | Draft the Wisdm RFP section on governance automation | writing | Governance facts, automation metrics, style preferences |
| 9 | What patterns do I follow when debugging auth issues? | procedure | DDI isolation first, token inspection, APIM policy review |
| 10 | What changed in Azure-PROD schema in the last month? | compliance | Recent schema episodes, entity changes, fact supersessions |

### Three Conditions (A/B/C)

| Condition | Description |
|-----------|------------|
| **A: No memory** | LLM with no Loom context (baseline) |
| **B: Simple retrieval** | Top-10 vector-similar episodes injected as raw text |
| **C: Loom compiled** | Full online pipeline: classify → retrieve → rank → compile |

### Decision Gate (End of Week 8)

- If C beats B by >= 15% precision AND >= 30% token reduction with no task success regression → proceed to Phase 2
- If C ~ B → simplify further, investigate whether compilation overhead is justified
- If C < B → stop, use simple retrieval, Loom thesis is disproven

Results displayed in the dashboard Benchmark Comparison view.

---

## Observability

All observability data is captured via the `tracing` crate and stored in `loom_audit_log`. The dashboard surfaces every metric listed below.

### Per-Request (Online)
- primary_class, secondary_class, confidences
- profiles_executed
- namespace, query_text, target_model
- candidates found/selected/rejected with score breakdowns and memory_type
- compiled_tokens, output_format
- latency per stage (classify, retrieve, rank, compile, total)
- user_rating (if provided)

### Per-Ingestion (Offline)
- episode_id, source, namespace
- duplicate_skipped
- ExtractionMetrics (full struct)
- facts_superseded count
- procedures_flagged count
- processing_time_ms

---

## Implementation Timeline

| Week | Milestone | Deliverable |
|------|-----------|------------|
| 1-2 | **Rust scaffolding + Schema + Basic Ingestion** | Cargo workspace setup, axum router, sqlx migrations (all tables including predicate packs), Ollama client wrapper, `loom_learn` endpoint, episode dedup (SHA-256 via sha2), manual ingestion REST endpoint, predicate pack table + core and GRC pack seeding, Docker Compose (engine + postgres + ollama), basic health endpoint |
| 3-4 | **Extraction + Resolution** | Entity extraction (Ollama + serde deserialization), pack-aware fact extraction prompt assembly (load predicates by namespace packs, format into grouped prompt block), fact extraction with predicate validation, three-pass entity resolution, alias accumulation, conflict tracking, custom predicate candidate tracking, extraction metrics logging. **Parallel: run quality gate against both Gemma 4 26B MoE and gpt-4.1-mini on 50 episodes.** |
| **Gate** | **End of Week 4** | **Entity precision >= 0.80, fact precision >= 0.75, predicate consistency >= 0.85. Select extraction model (local or API). Do not proceed until thresholds met.** |
| 5-6 | **Context Compiler + MCP** | Intent classification, dual-profile retrieval (tokio::join!), memory weight modifiers, 4-dimension ranking, `loom_think` + `loom_recall`, audit logging, structured + compact output formats, MCP JSON-RPC protocol implementation |
| 7-8 | **Dashboard + Claude Code Integration + Benchmark** | React dashboard (Tier 1 + Tier 2 views), dashboard API endpoints, MCP registration with Claude Code, hooks, CLAUDE.md, run 10 benchmark tasks across A/B/C, benchmark comparison view |
| **Gate** | **End of Week 8** | **C beats B by >= 15% precision AND >= 30% token reduction. If fail: simplify or kill.** |
| 9-10 | **Codex CLI + Tier Management** | Codex MCP config, hot/warm promotion rules, periodic snapshots (tokio scheduled task), namespace budget tuning, entity health check automation |
| 11-12 | **Second Connector + Hardening** | GitHub webhooks OR manual bulk import, error handling, Rust error types with thiserror, performance profiling, predicate candidate review workflow with pack-aware promotion in dashboard, pack browser component, entity graph visualization |

---

## Bootstrap (Python, Run-Once)

Bootstrap scripts remain Python. They are run-once data transformation tools, not long-running services. See the separate Bootstrap Guide for source-specific transform scripts.

```
loom-bootstrap/
├── requirements.txt            # httpx, json, hashlib (stdlib)
├── claude_ai.py
├── claude_code.py
├── codex_cli.py
├── chatgpt.py
├── manual.py
├── git_history.py
├── azure_devops.py
├── documents.py
└── loader.py                   # Bulk POST to /api/learn
```

---

## Backup Strategy

PostgreSQL is the single system of record. Local deployment means backups are your responsibility.

- **Automated pg_dump** on a daily cron schedule to a separate drive or off-site location
- Retain 7 daily + 4 weekly snapshots
- Test restore monthly
- Docker volume for PostgreSQL data directory, backed up separately from container
- Add `pg_dump` as a scheduled task in the Docker Compose setup (or host cron)

---

## What Is Explicitly Deferred

| Capability | Why Deferred | Trigger to Revisit |
|-----------|-------------|-------------------|
| Advanced procedural mining | Easy to overfit, hard to evaluate | After 50+ episodes show clear repeated patterns |
| Memify / semantic compaction | Can mask ingestion quality issues | After retrieval precision is measured and stable |
| 7+ source connectors | Breadth creates noise before quality is proven | After Claude Code retrieval quality is validated |
| Browser extensions | Complex to maintain, privacy concerns | After MCP-native tools are proven sufficient |
| Git-style memory versioning | Over-engineered for initial needs | After SOX audit actually requires historical diff |
| 3+ hop graph traversal | Performance risk, rarely needed | After 1-2 hop is measured as insufficient |
| Restorable compression | Optimization before baseline exists | After token budgets are proven too tight |
| Advanced salience self-improvement | Ranking drift risk before data exists | After 100+ compilations provide feedback signal |
| Cold tier | Additional complexity before warm is useful | After warm tier has enough volume to need archival |
| loom_inspect / loom_forget MCP tools | Dashboard provides inspection. Admin functions not core to thesis. | After MVP is stable |
| Cross-namespace retrieval | Intentionally restrictive for data integrity | After > 10% of benchmark queries show cross-namespace intent |
| Fact embeddings | Episode embeddings sufficient for initial retrieval | After fact-level similarity is measured as needed |
| Azure OpenAI dependency | Local-first with Gemma 4. API is fallback only. | If local model fails quality gate |
| Astro/SSR dashboard framework | Vite + React SPA is sufficient for operational views | If dashboard needs SEO or public-facing pages (it won't) |