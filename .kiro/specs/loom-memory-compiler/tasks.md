# Implementation Plan: Project Loom Memory Compiler

## Overview

This implementation plan breaks down Project Loom into discrete coding tasks for a PostgreSQL-native memory compiler. The system is a single Rust binary (loom-engine) using tokio async runtime and axum HTTP framework, with local LLM inference via Ollama (Gemma 4 models) and nomic-embed-text embeddings (768 dimensions). It features online (query serving) and offline (episode processing) pipelines with separate connection pools, MCP interface, entity/fact extraction with 3-pass resolution, pack-aware predicate system, comprehensive audit logging, and an operational React dashboard served by Caddy reverse proxy.

Implementation language: Rust
HTTP framework: axum + tokio async runtime
Database: PostgreSQL 16 + pgvector + pgAudit, queries via sqlx (compile-time checked)
Testing: Rust `#[test]` modules + proptest crate for property-based testing
Deployment: Docker Compose (loom-engine, loom-dashboard, PostgreSQL, Ollama, Caddy)

## Tasks

- [x] 1. Project scaffolding and infrastructure setup
  - [x] 1.1 Initialize Rust project with Cargo and dependencies
    - Create `loom-engine/` directory with `Cargo.toml`
    - Add dependencies: axum, tokio (full features), sqlx (postgres, runtime-tokio, tls-rustls), pgvector, serde, serde_json, reqwest, sha2, uuid, chrono, tracing, tracing-subscriber, tower, tower-http, proptest (dev)
    - Create directory structure: `src/`, `src/db/`, `src/llm/`, `src/pipeline/offline/`, `src/pipeline/online/`, `src/api/`, `src/worker/`, `src/types/`, `migrations/`, `prompts/`
    - Create `src/main.rs` with `#[tokio::main]` entry point and axum router skeleton
    - Create `src/config.rs` for environment variable configuration (database URLs, Ollama endpoint, model names, Azure OpenAI fallback, bearer token)
    - Create `.env.example` with all configuration variables
    - _Requirements: 45.1, 45.5, 52.6_

  - [x] 1.2 Create Docker Compose configuration with five containers
    - Write `docker-compose.yml` with five services: loom-engine, loom-dashboard, postgres, ollama, caddy
    - Configure PostgreSQL 16 container with pgvector and pgAudit extensions, persistent volume, port 5432 (internal only)
    - Configure Ollama container with GPU passthrough, model volume for gemma4:26b-a4b-q4, gemma4:e4b, nomic-embed-text
    - Configure Caddy container with Caddyfile for reverse proxy routing
    - Configure loom-engine container connecting to postgres and ollama via internal Docker network
    - Configure loom-dashboard container (Vite build output served by Caddy)
    - Set up Docker internal network and volume mounts
    - _Requirements: 45.1, 45.6, 45.7, 45.8, 45.9, 45.10, 45.11, 46.1, 46.2_

  - [x] 1.3 Create Dockerfile for loom-engine (multi-stage build)
    - Write multi-stage Dockerfile: Rust builder stage + scratch/distroless runtime stage
    - Build static binary (~20MB image)
    - Configure health check endpoint at /api/health
    - Expose port 8080 for MCP, REST, and Dashboard API endpoints
    - _Requirements: 45.1, 45.2, 45.3, 45.4_

  - [x] 1.4 Create Caddyfile for reverse proxy configuration
    - Route `/api/*` to loom-engine:8080
    - Route `/mcp/*` to loom-engine:8080
    - Route `/dashboard/api/*` to loom-engine:8080
    - Serve dashboard static files for all other paths
    - Configure automatic TLS certificate management
    - _Requirements: 51.1, 51.2, 51.3, 51.4, 51.5_

  - [x] 1.5 Create database connection pool initialization with online/offline separation
    - Implement `src/db/pool.rs` with two separate `sqlx::PgPool` instances
    - Configure online pool (dedicated for query serving, latency-sensitive)
    - Configure offline pool (separate for episode processing, throughput-oriented)
    - Pool sizes configurable via environment variables
    - Add health check query for database availability
    - Validate pgvector and pgAudit extension availability during startup
    - _Requirements: 44.7, 46.1, 46.2, 46.5_

- [x] 2. Database schema and SQL migrations
  - [x] 2.1 Create sqlx migrations for core tables (001-002)
    - Create `migrations/001_episodes.sql`: loom_episodes table with all canonical, extraction lineage, derived, and deletion fields; vector(768) embedding column; indexes (embedding IVFFlat cosine, namespace+occurred_at, content_hash, unprocessed); UNIQUE(source, source_event_id)
    - Create `migrations/002_entities.sql`: loom_entities table with entity_type CHECK constraint (10 types), UNIQUE(name, entity_type, namespace), properties JSONB with aliases; loom_entity_state table with tier CHECK, embedding vector(768), salience_score, pinned; indexes (namespace+type, aliases GIN)
    - _Requirements: 20.1, 20.5, 20.6, 20.7, 20.8, 21.3, 22.3_

  - [x] 2.2 Create sqlx migrations for predicate pack system (003-004)
    - Create `migrations/003_predicate_packs.sql`: loom_predicate_packs table with pack TEXT PRIMARY KEY, description; INSERT core pack seed row
    - Create `migrations/004_predicates.sql`: loom_predicates table with predicate TEXT PRIMARY KEY, category CHECK (structural, temporal, decisional, operational, regulatory), pack FK to loom_predicate_packs, inverse, description, usage_count; loom_predicate_candidates table with occurrences, example_facts UUID[], mapped_to FK, promoted_to_pack FK; index on pack
    - _Requirements: 25.1, 25.2, 25.3, 25.4, 25.5, 25.6, 25.7_

  - [x] 2.3 Create sqlx migrations for facts and procedures (005-006)
    - Create `migrations/005_facts.sql`: loom_facts table with subject_id/object_id FK to loom_entities, predicate, namespace, valid_from/valid_until, source_episodes UUID[], superseded_by self-FK, evidence_status CHECK (7 values), evidence_strength CHECK (explicit/implied); loom_fact_state table with embedding vector(768), tier, salience_score, pinned; indexes (current facts, object, status, predicate)
    - Create `migrations/006_procedures.sql`: loom_procedures table with pattern, category, namespace, source_episodes, observation_count, confidence, embedding vector(768), tier, evidence_status CHECK
    - _Requirements: 20.1, 20.8, 32.1, 33.2_

  - [x] 2.4 Create sqlx migrations for resolution conflicts, namespace config, audit, and snapshots (007-010)
    - Create `migrations/007_resolution_conflicts.sql`: loom_resolution_conflicts table with entity_name, entity_type, namespace, candidates JSONB, resolved boolean, resolution text, timestamps
    - Create `migrations/008_namespace_config.sql`: loom_namespace_config table with namespace PK, hot_tier_budget (default 500), warm_tier_budget (default 3000), predicate_packs TEXT[] DEFAULT '{core}', description, timestamps
    - Create `migrations/009_audit_log.sql`: loom_audit_log table with all compilation tracking fields (task_class, namespace, query_text, target_model, classification, profiles, candidates, selected/rejected items JSONB, tokens, format, latency breakdown, user_rating)
    - Create `migrations/010_snapshots.sql`: loom_snapshots table with namespace, hot_entities/facts/procedures JSONB, total_tokens
    - _Requirements: 18.1, 23.1, 28.1, 28.2, 28.3, 28.4, 29.1_

  - [x] 2.5 Create sqlx migration for graph traversal function (011)
    - Create `migrations/011_traverse_function.sql`: loom_traverse SQL function with recursive CTE
    - Accept p_entity_id UUID, p_max_hops INT DEFAULT 2, p_namespace TEXT DEFAULT 'default'
    - Return entity_id, entity_name, entity_type, fact_id, predicate, evidence_status, hop_depth, path UUID[]
    - Implement cycle prevention via path array (NOT entity_id = ANY(path))
    - Filter to namespace, valid_until IS NULL, deleted_at IS NULL
    - Traverse facts in both subject and object directions
    - _Requirements: 26.1, 26.2, 26.3, 26.4, 26.5, 26.6, 26.7, 26.8_

  - [x] 2.6 Create sqlx migrations for predicate seed data (012-013)
    - Create `migrations/012_seed_core_predicates.sql`: INSERT 25 core predicates across structural, temporal, decisional, operational categories (uses, used_by, contains, contained_in, depends_on, dependency_of, replaced_by, replaced, deployed_to, hosts, implements, implemented_by, decided, decided_by, integrates_with, targets, manages, managed_by, configured_with, blocked_by, blocks, authored_by, authored, owns, owned_by) with inverse relationships and descriptions
    - Create `migrations/013_seed_grc_pack.sql`: INSERT grc pack row into loom_predicate_packs; INSERT 23 regulatory predicates (scoped_as, scoping_includes, de-scoped_from, exception_granted_for, exception_applies_to, maps_to_control, control_mapped_from, evidenced_by, evidence_for, satisfies, satisfied_by, precedent_set_by, sets_precedent_for, finding_on, finding_raised_by, conflicts_with, supplements, supplemented_by, fills_gap_in, gap_filled_by, supersedes_in_context, superseded_in_context_by, compensated_by, compensates_for) with inverse relationships
    - _Requirements: 25.8, 25.9, 25.10_

  - [x] 2.7 Write property test for database schema constraints
    - **Property 25: Uniqueness Constraints**
    - **Validates: Requirements 20.6, 20.7**
    - Test that duplicate (source, source_event_id) insertions fail or return existing ID
    - Test that duplicate (name, entity_type, namespace) insertions fail or return existing ID
    - Use proptest with 100 iterations minimum


- [x] 3. Rust type definitions and database query layer
  - [x] 3.1 Define core Rust types with serde
    - Create `src/types/episode.rs`: Episode struct with all fields, serde Serialize/Deserialize, sqlx::FromRow; ExtractionMetrics struct for JSONB serialization (entity counts by resolution method, fact counts by predicate type, evidence counts, processing_time_ms, extraction_model)
    - Create `src/types/entity.rs`: Entity struct, EntityState struct, EntityType enum (10 variants with serde rename_all), ExtractedEntity struct, ResolutionResult struct with method and confidence
    - Create `src/types/fact.rs`: Fact struct, FactState struct, ExtractedFact struct, EvidenceStatus enum (7 variants), EvidenceStrength enum (explicit/implied), TemporalMarkers struct
    - Create `src/types/predicate.rs`: PredicateEntry struct, PredicatePack struct, PredicateCandidate struct, pack query types
    - Create `src/types/classification.rs`: ClassificationResult struct, TaskClass enum (5 variants with serde rename_all lowercase)
    - Create `src/types/compilation.rs`: CompiledPackage struct, OutputFormat enum (Structured/Compact), RankingScore struct with relevance/recency/stability/provenance fields
    - Create `src/types/audit.rs`: AuditLogEntry struct with all compilation tracking fields
    - Create `src/types/mcp.rs`: MCP JSON-RPC request/response types (LearnRequest, ThinkRequest, RecallRequest, LearnResponse, ThinkResponse, RecallResponse)
    - Create `src/types/mod.rs` re-exporting all types
    - _Requirements: 2.5, 8.1, 16.6, 16.8, 17.1, 17.2, 17.3, 32.1, 43.9, 48.1_

  - [x] 3.2 Implement database query module for episodes
    - Create `src/db/episodes.rs` with compile-time checked sqlx queries
    - Implement insert_episode with idempotency check (content_hash and source_event_id)
    - Implement get_episode_by_id, query_episodes_by_namespace with vector similarity
    - Implement mark_episode_processed, update_extraction_metrics (JSONB)
    - Implement soft_delete_episode (set deleted_at + deletion_reason)
    - _Requirements: 1.1, 1.2, 1.3, 1.4, 27.1, 27.2, 27.3, 48.2_

  - [x] 3.3 Implement database query module for entities
    - Create `src/db/entities.rs` with compile-time checked sqlx queries
    - Implement insert_entity with unique constraint handling
    - Implement get_entity_by_name_type_namespace (exact match on LOWER(name))
    - Implement query_entities_by_alias (GIN index on properties->'aliases')
    - Implement query_entities_by_embedding_similarity (pgvector cosine on loom_entity_state)
    - Implement update_entity_aliases (append to JSONB aliases array)
    - Implement update_entity_state (embedding, salience, tier, access_count)
    - _Requirements: 3.1, 3.4, 3.6, 22.1, 22.2, 42.1, 42.2_

  - [x] 3.4 Implement database query module for facts
    - Create `src/db/facts.rs` with compile-time checked sqlx queries
    - Implement insert_fact with provenance tracking (source_episodes array)
    - Implement query_current_facts_by_namespace (valid_until IS NULL, deleted_at IS NULL)
    - Implement query_facts_by_entity (subject or object)
    - Implement supersede_fact (set valid_until, superseded_by, evidence_status='superseded')
    - Implement soft_delete_fact
    - Implement update_fact_state (salience, tier, access_count)
    - _Requirements: 4.11, 6.1, 6.2, 6.3, 6.4, 10.1, 10.2, 10.3, 41.1_

  - [x] 3.5 Implement database query modules for predicates, audit, dashboard, and supporting tables
    - Create `src/db/predicates.rs`: query predicates by pack, increment usage_count, insert/update predicate candidates, query candidates by occurrence threshold
    - Create `src/db/audit.rs`: insert audit log entry, query audit logs with filters
    - Create `src/db/snapshots.rs`: insert snapshot, query snapshots by namespace
    - Create `src/db/procedures.rs`: insert/update procedures, query by namespace and evidence_status
    - Create `src/db/traverse.rs`: call loom_traverse SQL function via sqlx
    - Create `src/db/dashboard.rs`: read-only queries for all dashboard views (pipeline health, compilations, conflicts, predicate candidates, packs, metrics)
    - Create `src/db/mod.rs` re-exporting all modules
    - _Requirements: 5.1, 5.2, 18.1, 25.7, 26.1, 29.1, 33.2, 50.2_

  - [x] 3.6 Write unit tests for database query layer
    - Test episode idempotency with duplicate source_event_id
    - Test entity exact match, alias match, and embedding similarity queries
    - Test fact supersession logic (set valid_until and superseded_by)
    - Test soft deletion filtering across all tables
    - Test predicate candidate occurrence counting
    - Use sqlx test transactions with rollback for isolation
    - _Requirements: 1.2, 3.1, 6.3, 27.3, 5.2_

- [x] 4. Ollama LLM client and embedding service
  - [x] 4.1 Implement Ollama HTTP client with Azure OpenAI fallback
    - Create `src/llm/client.rs` with reqwest HTTP client for Ollama OpenAI-compatible API (`/v1/chat/completions`, `/v1/embeddings`)
    - Implement `call_llm` function accepting model name, system prompt, user prompt; return structured JSON response
    - Implement retry logic with exponential backoff (3 retries)
    - Implement Azure OpenAI fallback: if Ollama is unavailable or quality thresholds not met, switch to API models (gpt-4.1-mini for extraction)
    - Configure model endpoints via environment variables (OLLAMA_URL, AZURE_OPENAI_URL, AZURE_OPENAI_KEY)
    - Add timeout configuration (30 seconds per request)
    - _Requirements: 52.1, 52.5, 52.6, 31.3, 31.5_

  - [x] 4.2 Implement embedding generation service
    - Create `src/llm/embeddings.rs` with nomic-embed-text embedding generation via Ollama
    - Implement `generate_embedding` function returning Vec<f32> of 768 dimensions
    - Validate embedding dimension is exactly 768
    - Add retry logic with exponential backoff (3 retries)
    - _Requirements: 21.1, 21.2, 22.1, 22.2, 52.4_

  - [x] 4.3 Implement extraction prompt execution
    - Create `src/llm/extraction.rs` with entity and fact extraction via Gemma 4 26B MoE
    - Implement `extract_entities` function: send structured prompt, deserialize response via serde into Vec<ExtractedEntity>, reject malformed responses at type boundary
    - Implement `extract_facts` function: send pack-aware prompt, deserialize response via serde into Vec<ExtractedFact>, reject malformed responses
    - Record extraction model identifier for each call
    - _Requirements: 2.1, 2.5, 4.1, 31.1, 43.1, 43.9, 52.2_

  - [x] 4.4 Implement intent classification service
    - Create `src/llm/classification.rs` with intent classification via Gemma 4 E4B
    - Implement keyword matching for high-confidence signals (debug keywords, architecture keywords, etc.)
    - Implement LLM call for ambiguous cases returning ClassificationResult
    - Compute primary and secondary class with confidence scores
    - Default to TaskClass::Chat if ambiguous with no clear signal
    - Record classification model identifier
    - _Requirements: 8.1, 8.2, 8.3, 8.4, 8.5, 31.2, 31.4, 52.3_

  - [x] 4.5 Write property test for embedding dimension consistency
    - **Property 26: Embedding Dimension Consistency**
    - **Validates: Requirements 21.1, 22.1, 46.1**
    - Test that all generated embeddings have exactly 768 dimensions
    - Use proptest with 100 iterations, mock Ollama responses

  - [x] 4.6 Write unit tests for LLM client and services
    - Test retry logic with mocked Ollama failures (reqwest mock)
    - Test Azure OpenAI fallback activation
    - Test embedding generation with various text inputs
    - Test entity extraction JSON parsing via serde (valid and malformed responses)
    - Test fact extraction JSON parsing via serde
    - Test classification with keyword matching and LLM fallback
    - _Requirements: 21.1, 52.1, 52.5, 43.9_


- [x] 5. Checkpoint - Infrastructure validation
  - Verify Cargo project builds successfully
  - Verify Docker Compose brings up all 5 containers (loom-engine, loom-dashboard placeholder, postgres, ollama, caddy)
  - Verify sqlx migrations run against PostgreSQL (all 13 migration files)
  - Verify pgvector and pgAudit extensions are enabled
  - Verify Ollama connectivity and model availability
  - Verify Caddy routing rules work (proxy to loom-engine, static file serving)
  - Verify online and offline connection pools initialize separately
  - Ensure all tests pass, ask the user if questions arise
  - _Requirements: 45.1, 46.1, 46.5_


- [x] 6. Entity extraction and three-pass resolution
  - [x] 6.1 Implement entity extraction prompt and orchestration
    - Create `prompts/entity_extraction.txt` with structured prompt constraining to 10 entity types
    - Instruct to use most specific common name, extract aliases, avoid generic concepts
    - Instruct JSON-only output with no preamble
    - Create `src/pipeline/offline/extract.rs` orchestrating entity extraction: call LLM, deserialize via serde, validate entity types against EntityType enum
    - _Requirements: 2.1, 2.2, 2.3, 2.4, 43.1, 43.2, 43.3, 43.4_

  - [x] 6.2 Implement Pass 1: Exact match resolution
    - In `src/pipeline/offline/resolve.rs`, implement exact match query via sqlx: LOWER(name), entity_type, namespace
    - Return existing entity_id with confidence 1.0 on match
    - _Requirements: 3.1, 3.2_

  - [x] 6.3 Implement Pass 2: Alias match resolution
    - Query entities where extracted name appears in existing entity's aliases (GIN index on properties->'aliases')
    - Check both directions: extracted name in existing aliases, extracted aliases match existing name
    - If exactly one match: merge with confidence 0.95, append new name to aliases array (case-insensitive dedup)
    - If multiple matches: fall through to Pass 3
    - _Requirements: 3.3, 3.4, 3.5, 42.1, 42.2, 42.3_

  - [x] 6.4 Implement Pass 3: Semantic similarity resolution
    - Embed entity name + context snippet via nomic-embed-text (768 dimensions)
    - Query loom_entity_state by cosine similarity (pgvector) WHERE entity_type and namespace match
    - If top candidate > 0.92 AND gap to second >= 0.03: merge with confidence = similarity score
    - If top two within 0.03: create new entity, log conflict to loom_resolution_conflicts
    - If no candidate > 0.92: create new entity with confidence 1.0
    - _Requirements: 3.6, 3.7, 3.8, 3.9, 22.3, 23.1, 23.2_

  - [x] 6.5 Implement entity resolution orchestrator
    - Execute 3-pass algorithm in sequence in `src/pipeline/offline/resolve.rs`
    - Track resolution method (exact, alias, semantic, new) for logging
    - Update entity serving state (embedding, salience) in loom_entity_state
    - Link entity to source episode (append to source_episodes array)
    - Log resolution method per entity
    - _Requirements: 3.10, 2.6, 2.7_

  - [x] 6.6 Write property test for exact match resolution confidence
    - **Property 5: Exact Match Resolution Confidence**
    - **Validates: Requirements 3.1, 3.2**
    - Test that exact matches always return confidence 1.0
    - Use proptest with 100 iterations

  - [x] 6.7 Write property test for semantic resolution threshold
    - **Property 6: Semantic Resolution Threshold**
    - **Validates: Requirements 3.7**
    - Test that semantic matches above 0.92 with gap >= 0.03 merge correctly

  - [x] 6.8 Write property test for resolution conflict logging
    - **Property 7: Resolution Conflict Logging**
    - **Validates: Requirements 3.8, 23.1, 23.2**
    - Test that ambiguous resolutions (top two within 0.03) create conflict records in loom_resolution_conflicts

  - [x] 6.9 Write unit tests for entity extraction and resolution
    - Test entity type constraint validation via serde deserialization
    - Test alias accumulation and case-insensitive deduplication
    - Test resolution method selection logic across all 3 passes
    - Test conflict detection and logging
    - _Requirements: 2.2, 3.1, 3.4, 3.6, 3.8, 42.3_

- [x] 7. Fact extraction with pack-aware prompts and supersession
  - [x] 7.1 Implement pack-aware fact extraction prompt assembly
    - Create `prompts/fact_extraction.txt` as a dynamic template with placeholder for predicate block
    - In `src/pipeline/offline/extract.rs`, implement `assemble_fact_prompt`: load namespace's configured predicate packs from loom_namespace_config, always include core pack, query loom_predicates for all predicates in active packs, format into grouped prompt block organized by pack name, inject into template
    - Instruct to validate subject/object references, classify evidence strength (explicit/implied), extract temporal markers (valid_from, valid_until), use canonical predicates when available, flag custom predicates
    - _Requirements: 4.1, 4.2, 4.3, 4.4, 4.5, 4.9, 4.10, 43.5, 43.6, 43.7, 43.8_

  - [x] 7.2 Implement predicate validation and custom predicate tracking
    - Check each extracted predicate against canonical registry via sqlx query
    - If match: mark custom=false, increment usage_count on loom_predicates
    - If no match: mark custom=true, insert or update loom_predicate_candidates (increment occurrences, append fact ID to example_facts)
    - When candidate reaches 5 occurrences, flag for operator review (surface in dashboard)
    - Log canonical and custom predicate counts per episode
    - _Requirements: 4.6, 4.7, 4.8, 4.12, 5.1, 5.2, 5.3, 5.4_

  - [x] 7.3 Implement fact supersession resolver
    - Create `src/pipeline/offline/supersede.rs`
    - Query existing facts with same (subject_id, predicate, namespace) but different object_id
    - Set valid_until = new_fact.valid_from on old fact
    - Set superseded_by = new_fact.id on old fact
    - Update evidence_status to 'superseded' on old fact
    - Log supersession count per ingestion
    - _Requirements: 6.3, 6.4, 6.5_

  - [x] 7.4 Implement fact extraction orchestrator
    - In `src/pipeline/offline/extract.rs`, orchestrate full fact extraction flow
    - Assemble pack-aware prompt, call LLM, deserialize via serde
    - Validate entity references (subject and object must be in extracted entities)
    - Validate and track predicates (canonical vs custom)
    - Insert facts with provenance (source_episodes array)
    - Resolve supersession for contradicting facts
    - Link each fact to source episode identifiers
    - _Requirements: 4.1, 4.2, 4.11, 41.1, 41.2_

  - [x] 7.5 Write property test for pack-aware prompt assembly
    - **Property 8: Pack-Aware Prompt Assembly**
    - **Validates: Requirements 4.3, 4.4, 4.5, 28.6**
    - Test that prompt always contains core pack predicates regardless of namespace config
    - Test that configured packs' predicates appear grouped by pack name
    - Use proptest with 100 iterations

  - [x] 7.6 Write property test for canonical predicate classification
    - **Property 9: Canonical Predicate Classification**
    - **Validates: Requirements 4.7, 4.8**
    - Test that facts with canonical predicates are marked custom=false
    - Test that facts with non-canonical predicates are marked custom=true and tracked in candidates

  - [x] 7.7 Write property test for custom predicate occurrence tracking
    - **Property 10: Custom Predicate Occurrence Tracking**
    - **Validates: Requirements 5.2, 5.4**
    - Test that occurrence count equals number of facts using that predicate
    - Test that candidates reaching 5 occurrences are flagged for review

  - [x] 7.8 Write property test for fact supersession
    - **Property 12: Fact Supersession**
    - **Validates: Requirements 6.3, 6.4**
    - Test that contradicting facts trigger supersession with correct timestamps and superseded_by

  - [x] 7.9 Write property test for predicate pack isolation
    - **Property 33: Predicate Pack Isolation**
    - **Validates: Requirements 25.3, 28.4, 28.6**
    - Test that each predicate belongs to exactly one pack
    - Test that namespace predicate_packs always contains 'core'

  - [x] 7.10 Write unit tests for fact extraction and supersession
    - Test predicate validation against canonical registry
    - Test custom predicate candidate creation and flagging at 5 occurrences
    - Test supersession with multiple contradicting facts
    - Test temporal marker extraction
    - Test pack-aware prompt assembly with various namespace configurations
    - _Requirements: 4.3, 4.7, 5.4, 6.3_


- [x] 8. Background worker and offline pipeline orchestration
  - [x] 8.1 Implement episode processing via tokio spawned tasks
    - Create `src/worker/processor.rs` with background episode processing loop
    - Poll for unprocessed episodes (processed=false, deleted_at IS NULL) using offline connection pool
    - Spawn tokio tasks for each episode (concurrency controlled)
    - Mark episodes as processed after extraction completes
    - Return immediately from loom_learn without blocking on extraction
    - _Requirements: 44.1, 44.2, 44.3, 44.4_

  - [x] 8.2 Implement episode embedding generation in offline pipeline
    - Generate 768-dimension embedding for episode content via nomic-embed-text through Ollama
    - Store embedding in loom_episodes table via sqlx
    - Handle embedding failures with retry queue
    - _Requirements: 21.1, 21.2, 21.4_

  - [x] 8.3 Implement full extraction pipeline orchestrator
    - In `src/pipeline/offline/extract.rs`, orchestrate complete offline flow:
      1. Generate episode embedding
      2. Extract entities via Gemma 4 26B MoE
      3. Resolve each entity through 3-pass algorithm
      4. Assemble pack-aware fact extraction prompt
      5. Extract facts via Gemma 4 26B MoE
      6. Validate fact entity references
      7. Resolve fact supersession
      8. Update entity and fact serving state (salience, tier)
      9. Optionally flag candidate procedures
    - Use offline connection pool for all database operations
    - _Requirements: 2.1, 3.1, 4.1, 6.3, 33.1_

  - [x] 8.4 Implement extraction metrics logging
    - Create `src/pipeline/offline/state.rs` for derived state computation
    - Compute ExtractionMetrics struct: entity counts by resolution method (exact, alias, semantic, new, conflict_flagged), fact counts by predicate type (canonical, custom), evidence counts (explicit, implied), processing_time_ms, extraction_model
    - Serialize as JSONB and store in episode.extraction_metrics column via sqlx
    - Log entity extraction counts and predicate counts per episode via tracing
    - _Requirements: 2.6, 2.7, 3.10, 4.12, 48.1, 48.2, 48.3, 48.4, 48.5, 48.6, 48.7_

  - [x] 8.5 Implement scheduled tasks via tokio
    - Create `src/worker/scheduler.rs` with tokio-based periodic task runner
    - Daily hot tier snapshot job: capture hot entities, facts, procedures per namespace, compute total tokens, store in loom_snapshots
    - Daily tier promotion/demotion check: evaluate promotion and demotion criteria, update tier assignments
    - Weekly entity health check: identify entity pairs in same namespace and type with embedding similarity > 0.85, rank by similarity, return top 50 pairs, exclude soft-deleted
    - _Requirements: 24.1, 24.2, 24.3, 24.4, 24.5, 29.1, 29.2, 29.3, 29.4, 29.5_

  - [x] 8.6 Write property test for asynchronous episode ingestion
    - **Property 24: Asynchronous Ingestion and Pipeline Separation**
    - **Validates: Requirements 17.7, 44.1, 44.6**
    - Test that loom_learn returns immediately with status "queued" or "accepted"
    - Test that loom_think completes without blocking on offline processing

  - [x] 8.7 Write property test for extraction metrics completeness
    - **Property 31: Extraction Metrics Completeness**
    - **Validates: Requirements 48.1, 48.3, 48.4, 48.5, 48.6, 48.7**
    - Test that processed episodes have extraction_metrics JSONB containing all required fields

  - [x] 8.8 Write unit tests for background worker
    - Test episode processing loop with mocked extraction
    - Test extraction pipeline orchestration end-to-end
    - Test extraction metrics computation and JSONB serialization
    - Test scheduled task execution (snapshots, health check)
    - _Requirements: 44.1, 44.4, 44.5, 48.1_

- [x] 9. Checkpoint - Extraction quality gate (50 episodes)
  - Ingest 50 representative episodes from Claude Code sessions
  - Run extraction against both Gemma 4 26B MoE (local via Ollama) and gpt-4.1-mini (API) in parallel
  - Evaluate extraction quality:
    - Entity precision >= 0.80 (correctly resolved / total extracted)
    - Entity recall >= 0.70 (correctly resolved / total expected)
    - Fact precision >= 0.75 (valid facts / total facts)
    - Fact recall >= 0.60 (correct facts / total expected)
    - Predicate consistency >= 0.85 (canonical / total predicates)
  - If Gemma 4 26B MoE meets all thresholds, use local model (zero cloud dependency)
  - If Gemma 4 26B MoE fails but gpt-4.1-mini passes, use API model with planned local upgrade path
  - Review resolution conflicts and predicate candidates
  - Ensure all tests pass, ask the user if questions arise
  - _Requirements: 19.1, 19.2, 19.3, 19.4, 19.5, 19.6, 19.7, 19.8, 19.9_


- [x] 10. Intent classification and retrieval profiles
  - [x] 10.1 Implement intent classifier
    - Create `prompts/classification.txt` with classification prompt for 5 task classes
    - In `src/pipeline/online/classify.rs`, implement classification pipeline:
      1. Keyword matching for high-confidence signals
      2. LLM call (Gemma 4 E4B via Ollama) for ambiguous cases
      3. Compute primary and secondary class with confidence scores
      4. If confidence gap < 0.3, record both primary and secondary classes
      5. If gap >= 0.3, use only primary class
      6. Default to TaskClass::Chat if ambiguous
    - Log primary class, secondary class, and confidence scores to audit log
    - _Requirements: 8.1, 8.2, 8.3, 8.4, 8.5, 8.6_

  - [x] 10.2 Implement retrieval profile mapper
    - In `src/pipeline/online/retrieve.rs`, implement profile mapping:
      - Debug → [GraphNeighborhood, EpisodeRecall]
      - Architecture → [FactLookup, GraphNeighborhood]
      - Compliance → [EpisodeRecall, FactLookup]
      - Writing → [FactLookup]
      - Chat → [FactLookup]
    - Merge profiles from primary and secondary classes, deduplicate, cap at 3
    - _Requirements: 9.1, 9.2, 9.3, 9.4, 9.5, 9.6, 9.7_

  - [x] 10.3 Implement fact_lookup retrieval profile
    - Filter facts: valid_until IS NULL, deleted_at IS NULL, namespace match
    - Rank by vector similarity on source episodes (pgvector cosine)
    - Boost facts where entity names match query terms
    - Return up to 20 fact candidates
    - Use online connection pool
    - _Requirements: 10.1, 10.2, 10.3, 10.4, 10.5, 10.6_

  - [x] 10.4 Implement episode_recall retrieval profile
    - Filter episodes: deleted_at IS NULL, namespace match
    - Rank by vector similarity to query (pgvector cosine on episode embeddings)
    - Apply recency weighting favoring recent occurred_at timestamps
    - Return up to 10 episode candidates
    - Use online connection pool
    - _Requirements: 11.1, 11.2, 11.3, 11.4, 11.5_

  - [x] 10.5 Implement graph_neighborhood retrieval profile
    - Identify entities mentioned in query (keyword matching against entity names)
    - Call loom_traverse SQL function with max_hops=1
    - If < 3 results, retry with max_hops=2
    - Filter to current facts (valid_until IS NULL, deleted_at IS NULL), namespace match
    - Return entities and connecting facts
    - Use online connection pool
    - _Requirements: 12.1, 12.2, 12.3, 12.4, 12.5, 12.6, 12.7_

  - [x] 10.6 Implement procedure_assist retrieval profile
    - Filter procedures: evidence_status='promoted', confidence >= 0.8, observation_count >= 3
    - Filter to namespace, deleted_at IS NULL
    - Exclude for compliance task class (weight 0.0)
    - Return up to 3 procedure candidates
    - Use online connection pool
    - _Requirements: 34.1, 34.2, 34.3, 34.4, 34.5, 34.6, 34.7_

  - [x] 10.7 Implement retrieval profile executor with parallel execution
    - Execute all active profiles in parallel via `tokio::join!`
    - Merge and deduplicate candidates by ID
    - Track profile execution for audit logging
    - Log executed profile names to audit log
    - _Requirements: 9.8, 9.9_

  - [x] 10.8 Write property test for task class validity
    - **Property 15: Task Class Validity**
    - **Validates: Requirements 8.1, 8.3**
    - Test that all classifications return one of 5 valid task classes
    - Test that gap < 0.3 records both primary and secondary classes

  - [x] 10.9 Write property test for retrieval profile mapping and cap
    - **Property 16: Retrieval Profile Mapping and Cap**
    - **Validates: Requirements 9.1, 9.2, 9.3, 9.4, 9.5, 9.6, 9.7**
    - Test that each task class maps to correct profiles
    - Test that merged profiles never exceed 3

  - [x] 10.10 Write property test for cycle prevention in graph traversal
    - **Property 17: Cycle Prevention in Graph Traversal**
    - **Validates: Requirements 12.4, 26.4**
    - Test that no entity appears more than once in traversal path

  - [x] 10.11 Write unit tests for classification and retrieval
    - Test keyword matching for high-confidence signals
    - Test LLM fallback for ambiguous cases (mocked Ollama)
    - Test profile merging and deduplication
    - Test each retrieval profile with various queries
    - Test parallel execution via tokio::join!
    - _Requirements: 8.1, 8.5, 9.7, 10.1, 11.1, 12.1, 34.1_


- [x] 11. Memory weight modifiers and four-dimension ranking
  - [x] 11.1 Implement memory weight modifiers
    - Create `src/pipeline/online/weight.rs`
    - Define weight matrix for (TaskClass, memory_type) combinations:
      - Debug: (episodic=1.0, semantic=0.7, procedural=0.8)
      - Architecture: (episodic=0.5, semantic=1.0, procedural=0.3)
      - Compliance: (episodic=1.0, semantic=0.8, procedural=0.0)
      - Writing: (episodic=0.3, semantic=1.0, procedural=0.6)
      - Chat: (episodic=0.4, semantic=1.0, procedural=0.3)
    - Apply weights to candidate relevance scores
    - Hard-exclude candidates with weight 0.0 (remove from candidate list)
    - _Requirements: 13.1, 13.2, 13.3, 13.4, 13.5, 13.6_

  - [x] 11.2 Implement four-dimension ranker
    - Create `src/pipeline/online/rank.rs`
    - Score relevance (weight 0.40): vector similarity to query + entity name matching, modified by memory weight
    - Score recency (weight 0.25): time-based decay from occurred_at or valid_from timestamps
    - Score stability (weight 0.20): current status (non-superseded), evidence_status authority, salience score
    - Score provenance (weight 0.15): source episode count, evidence_status authority (user_asserted > observed > extracted > inferred)
    - Each dimension scored 0.0-1.0
    - Final score = (relevance × 0.40) + (recency × 0.25) + (stability × 0.20) + (provenance × 0.15)
    - Sort candidates by final score descending
    - _Requirements: 40.1, 40.2, 40.3, 40.4, 40.5, 40.6_

  - [x] 11.3 Implement salience score computation
    - In `src/pipeline/offline/state.rs`, implement salience computation
    - Initialize new items to 0.5
    - Increment on access with decay factor
    - Track access_count and last_accessed in serving state tables
    - Use salience as input to stability ranking dimension
    - _Requirements: 39.1, 39.2, 39.3, 39.4, 39.5, 39.6, 39.7_

  - [x] 11.4 Write property test for hard exclusion by weight
    - **Property 18: Hard Exclusion by Weight**
    - **Validates: Requirements 13.6**
    - Test that candidates with weight 0.0 never appear in final ranked results

  - [x] 11.5 Write property test for four-dimension weighted ranking
    - **Property 20: Four-Dimension Weighted Ranking**
    - **Validates: Requirements 40.1, 40.2, 40.3, 40.4, 40.5, 40.6**
    - Test that final scores equal (relevance × 0.40) + (recency × 0.25) + (stability × 0.20) + (provenance × 0.15)
    - Test that candidates are sorted in descending order by final score

  - [x] 11.6 Write unit tests for ranking and weighting
    - Test weight application for each task class
    - Test four-dimension score computation with known inputs
    - Test salience score updates on access
    - Test final score combination and sorting
    - _Requirements: 13.1, 40.1, 40.5, 39.2_

- [x] 12. Hot and warm tier management
  - [x] 12.1 Implement hot tier promotion logic
    - In `src/pipeline/offline/state.rs`, implement promotion rules:
      - Promote on explicit user pin (set pinned=true, tier='hot')
      - Promote when retrieved and used in 5+ compilations within 14 days
      - Prevent procedures from hot tier until 3+ episodes, 7+ days, confidence >= 0.8
    - Update tier field in loom_entity_state and loom_fact_state via sqlx
    - _Requirements: 14.3, 14.4, 14.9_

  - [x] 12.2 Implement hot tier demotion logic
    - Demote when user unpins (set pinned=false, tier='warm')
    - Demote when not retrieved in 30 days (last_accessed check)
    - Demote when fact is superseded (superseded_by IS NOT NULL → tier='warm')
    - Demote lowest-salience item when budget exceeded
    - _Requirements: 14.5, 14.6, 14.7, 14.8_

  - [x] 12.3 Implement hot tier budget enforcement
    - Query namespace config for hot_tier_budget (default 500 tokens)
    - Compute total tokens for hot tier items in namespace
    - When budget exceeded, demote lowest-salience hot item to warm tier
    - _Requirements: 14.2, 14.8, 28.2_

  - [x] 12.4 Implement warm tier archival logic
    - Archive superseded facts immediately (exclude from auto-retrieval)
    - Archive facts not accessed in 90 days
    - Maintain archived facts as searchable but exclude from automatic retrieval
    - _Requirements: 15.1, 15.2, 15.3, 15.4, 15.5_

  - [x] 12.5 Write property test for hot tier constraints
    - **Property 19: Hot Tier Constraints**
    - **Validates: Requirements 14.3, 14.7, 14.8**
    - Test that hot tier total tokens never exceed namespace budget
    - Test that facts with superseded_by != NULL are never in hot tier
    - Test that explicitly pinned items are always in hot tier

  - [x] 12.6 Write unit tests for tier management
    - Test promotion criteria (5+ uses in 14 days)
    - Test demotion criteria (30 days no access, supersession)
    - Test budget overflow handling (lowest-salience demotion)
    - Test archival logic (90 days, superseded)
    - Test procedure hot tier prevention criteria
    - _Requirements: 14.3, 14.4, 14.6, 14.7, 15.3, 15.4_


- [x] 13. Context package compilation with XML and JSON output
  - [x] 13.1 Implement context compiler
    - Create `src/pipeline/online/compile.rs`
    - Inject all hot tier memory for namespace (always included)
    - Add warm tier candidates up to token budget (default 3000 tokens)
    - Merge candidates from all executed profiles
    - Deduplicate candidates by identifier
    - Trim to fit warm_tier_budget
    - Include provenance information for each memory item
    - Compute total token count for compiled package
    - _Requirements: 16.1, 16.2, 16.4, 16.5, 16.9, 16.10, 28.3_

  - [x] 13.2 Implement structured output format (XML-like tags)
    - Format with XML-like tags: `<loom>`, `<identity>`, `<project>`, `<knowledge>`, `<episodes>`, `<patterns>`
    - Include model, token count, namespace, and task class as attributes on root `<loom>` tag
    - Format facts with subject, predicate, object, evidence, observed date, source attributes
    - Format episodes with date, source, id attributes
    - Format patterns with confidence and observations attributes
    - _Requirements: 16.6, 16.7_

  - [x] 13.3 Implement compact output format (JSON)
    - Format as JSON object with fields: ns, task, identity, facts, recent, patterns
    - Facts as array of {s, p, o, e, t} objects
    - Recent episodes as array of {date, src, text} objects
    - Patterns as array of {p, c, n} objects
    - Optimize for token efficiency
    - _Requirements: 16.8_

  - [x] 13.4 Implement compilation audit logging
    - Log task class, namespace, query text, target model
    - Log primary and secondary classification with confidence scores
    - Log executed retrieval profiles
    - Log candidate counts: found, selected, rejected
    - Log selected items with memory type, identifier, and score breakdown (relevance, recency, stability, provenance)
    - Log rejected items with rejection reason
    - Log compiled token count and output format
    - Log latency breakdown via tracing spans: total, classification, retrieval, ranking, compilation
    - Log user rating when provided
    - _Requirements: 18.1, 18.2, 18.3, 18.4, 18.5, 18.6, 18.7, 18.8, 18.9, 40.7_

  - [x] 13.5 Write property test for candidate deduplication
    - **Property 21: Candidate Deduplication**
    - **Validates: Requirements 16.2**
    - Test that no candidate appears more than once in final context package

  - [x] 13.6 Write property test for hot tier injection
    - **Property 22: Hot Tier Injection**
    - **Validates: Requirements 16.5**
    - Test that all hot tier items for namespace are included in context package

  - [x] 13.7 Write property test for output format correctness
    - **Property 23: Output Format Correctness**
    - **Validates: Requirements 16.6, 16.7, 16.8**
    - Test that structured output contains XML-like tags with correct attributes
    - Test that compact output is valid JSON with required fields

  - [x] 13.8 Write unit tests for context compilation
    - Test structured XML format generation with sample data
    - Test compact JSON format generation with sample data
    - Test token budget enforcement (warm tier trimming)
    - Test provenance tracking in output
    - Test audit logging completeness
    - Test latency measurement via tracing spans
    - _Requirements: 16.4, 16.6, 16.8, 18.1, 35.1_


- [x] 14. Checkpoint - Online pipeline integration testing
  - Run end-to-end integration tests for complete workflows
  - Test loom_learn → extraction → resolution → fact creation (offline pipeline)
  - Test loom_think → classification → retrieval → ranking → compilation (online pipeline)
  - Test loom_recall → direct fact lookup
  - Verify namespace isolation (no cross-namespace leaks)
  - Verify hot/warm tier management
  - Verify audit logging completeness
  - Verify online and offline pipelines use separate connection pools
  - Ensure all tests pass, ask the user if questions arise
  - _Requirements: 7.2, 14.1, 15.1, 18.1, 44.6, 44.7_

- [ ] 15. MCP interface implementation
  - [x] 15.1 Implement loom_learn MCP endpoint
    - Create `src/api/mcp.rs` with axum JSON-RPC handler
    - Accept content, source, namespace, occurred_at, metadata, participants via serde deserialization
    - Validate required fields at serde type boundary
    - Compute content_hash via sha2 crate (SHA-256)
    - Check idempotency: query (source, source_event_id) and content_hash via sqlx
    - If duplicate: return existing episode_id with status "duplicate"
    - If new: insert episode via sqlx, spawn tokio task for offline processing, return episode_id with status "queued"
    - Return immediately without blocking on extraction
    - _Requirements: 17.1, 17.4, 17.7, 1.1, 1.2, 1.3, 1.4, 1.5, 1.6_

  - [~] 15.2 Implement loom_think MCP endpoint
    - Accept query, namespace, task_class_override, target_model via serde
    - Start latency tracking via tracing spans
    - Classify intent (or use override) via online pipeline
    - Map to retrieval profiles, execute in parallel via tokio::join! (online pool)
    - Apply memory weight modifiers
    - Rank candidates on 4 dimensions (relevance 0.40, recency 0.25, stability 0.20, provenance 0.15)
    - Compile context package (structured XML or compact JSON based on target_model)
    - Update serving state (access counts, last_accessed)
    - Write audit log entry with full trace
    - Return context package with token count and compilation_id
    - _Requirements: 17.2, 17.5, 8.1, 9.1, 10.1, 16.1, 18.1_

  - [~] 15.3 Implement loom_recall MCP endpoint
    - Accept entity_names, namespace, include_historical via serde
    - Query facts by entity names (subject or object) via sqlx
    - Filter to current facts (valid_until IS NULL) unless include_historical=true
    - Return fact list with subject, predicate, object, evidence_status, provenance
    - Bypass intent classification and retrieval profiles
    - _Requirements: 17.3, 17.6_

  - [~] 15.4 Implement bearer token authentication middleware
    - Create `src/api/auth.rs` with tower middleware for bearer token validation
    - Apply to all MCP, REST, and Dashboard API endpoints
    - Token configured via environment variable
    - Return 401 Unauthorized on missing or invalid token
    - _Requirements: 45.2, 45.3, 45.4_

  - [~] 15.5 Write property test for episode idempotency
    - **Property 1: Episode Idempotency**
    - **Validates: Requirements 1.2**
    - Test that duplicate submissions return same episode_id with status "duplicate"
    - Use proptest with 100 iterations

  - [~] 15.6 Write property test for episode field completeness
    - **Property 2: Episode Field Completeness**
    - **Validates: Requirements 1.1, 1.3, 1.4, 1.5, 31.1, 31.2, 31.7**
    - Test that all required fields are stored correctly

  - [~] 15.7 Write property test for content hash correctness
    - **Property 3: Content Hash Correctness**
    - **Validates: Requirements 1.3**
    - Test that stored content_hash equals SHA-256 digest of content via sha2 crate

  - [~] 15.8 Write property test for connection pool separation
    - **Property 34: Connection Pool Separation**
    - **Validates: Requirements 44.7**
    - Test that online pipeline queries use dedicated online pool
    - Test that offline pipeline tasks use separate offline pool

  - [~] 15.9 Write unit tests for MCP endpoints
    - Test loom_learn with various source types (manual, claude-code, github)
    - Test loom_think with different task classes and namespaces
    - Test loom_recall with and without historical flag
    - Test error handling and validation (missing fields, invalid namespace)
    - Test bearer token authentication (valid, invalid, missing)
    - _Requirements: 17.1, 17.2, 17.3, 17.4, 17.5, 17.6_


- [ ] 16. REST API and dashboard API endpoints
  - [~] 16.1 Implement REST API endpoints
    - Create `src/api/rest.rs` with axum handlers
    - POST /api/learn: manual episode submission (same logic as loom_learn with source='manual')
    - GET /api/health: health check returning database and Ollama connectivity status
    - _Requirements: 37.1, 37.2, 37.3, 37.4, 37.5, 37.6_

  - [~] 16.2 Implement dashboard API read-only endpoints
    - Create `src/api/dashboard.rs` with axum handlers
    - GET /dashboard/api/health: pipeline health overview (episode counts by source/namespace, entity counts by type, fact counts current vs superseded, queue depth, model config)
    - GET /dashboard/api/namespaces: namespace listing for navigation
    - GET /dashboard/api/compilations: paginated compilation trace list
    - GET /dashboard/api/compilations/:id: compilation detail with candidates and score breakdowns
    - GET /dashboard/api/entities: entity search with filters
    - GET /dashboard/api/entities/:id: entity detail with properties, aliases, graph
    - GET /dashboard/api/entities/:id/graph: 1-2 hop neighborhood via loom_traverse
    - GET /dashboard/api/facts: fact listing with filters
    - GET /dashboard/api/conflicts: unresolved entity conflicts
    - GET /dashboard/api/predicates/candidates: custom predicate candidates with occurrence counts
    - GET /dashboard/api/predicates/packs: all packs with predicate counts
    - GET /dashboard/api/predicates/packs/:pack: pack detail with predicates, categories, usage counts
    - GET /dashboard/api/predicates/active/:namespace: active predicates for namespace
    - GET /dashboard/api/metrics/retrieval: precision over time, latency percentiles
    - GET /dashboard/api/metrics/extraction: model comparison, resolution distribution, custom predicate growth
    - GET /dashboard/api/metrics/classification: confidence distribution
    - GET /dashboard/api/metrics/hot-tier: hot-tier utilization per namespace
    - _Requirements: 49.1, 49.2, 49.3, 49.4, 49.5, 49.6, 49.7, 49.8, 49.9, 50.1, 50.2, 50.5_

  - [~] 16.3 Implement dashboard API write endpoints (only 2)
    - POST /dashboard/api/conflicts/:id/resolve: resolve entity conflict (merge, keep_separate, split)
    - POST /dashboard/api/predicates/candidates/:id/resolve: resolve predicate candidate (map to existing canonical, or promote to canonical with target_pack selection)
    - _Requirements: 23.4, 23.5, 5.5, 5.6, 5.7, 50.3, 50.4_

  - [~] 16.4 Write property test for dashboard API read-only enforcement
    - **Property 32: Dashboard API Read-Only Enforcement**
    - **Validates: Requirements 50.2, 50.3**
    - Test that GET endpoints do not modify database state
    - Test that only conflict resolution and predicate candidate resolution POSTs perform writes

  - [~] 16.5 Write unit tests for REST and dashboard API
    - Test manual ingestion via POST /api/learn
    - Test health check endpoint
    - Test dashboard read-only endpoints return correct data shapes
    - Test conflict resolution write endpoint
    - Test predicate candidate resolution with pack-aware promotion
    - Test namespace listing
    - _Requirements: 37.1, 50.2, 50.3, 50.4, 50.5_

- [ ] 17. Source connectors and namespace isolation
  - [~] 17.1 Implement GitHub webhook connector
    - In `src/api/rest.rs`, add POST /api/webhooks/github endpoint
    - Support pull request comment events and issue comment events
    - Extract occurred_at from GitHub event timestamps
    - Use GitHub event identifiers for idempotency (source_event_id)
    - Resolve namespace from GitHub repository name
    - Extract participants from event actors
    - Process through same extraction pipeline as other sources
    - _Requirements: 38.1, 38.2, 38.3, 38.4, 38.5, 38.6, 38.7_

  - [~] 17.2 Implement Claude Code MCP integration
    - Register MCP endpoints with Claude Code via HTTP transport
    - Ingest episodes with source='claude-code'
    - Resolve namespace from Claude Code working directory context
    - Support manual namespace override in MCP calls
    - Create CLAUDE.md configuration file documenting MCP integration
    - _Requirements: 30.1, 30.2, 30.3, 30.4, 30.5, 30.6_

  - [~] 17.3 Implement namespace isolation enforcement
    - Add namespace filter to all retrieval queries (episodes, entities, facts, procedures)
    - Prevent cross-namespace entity references in facts (validate subject, object, fact all same namespace)
    - Maintain separate hot tier budgets per namespace
    - Create separate entity records for same real-world entity in different namespaces
    - Maintain default namespace for general knowledge
    - _Requirements: 7.1, 7.2, 7.3, 7.4, 7.5, 7.6_

  - [~] 17.4 Implement soft deletion with audit trail
    - Implement soft_delete functions for episodes, entities, facts, procedures
    - Set deleted_at timestamp and optional deletion_reason
    - Exclude soft-deleted records from all retrieval queries (WHERE deleted_at IS NULL)
    - Maintain soft-deleted records in database for audit
    - Allow operators to query soft-deleted records for audit purposes
    - _Requirements: 27.1, 27.2, 27.3, 27.4, 27.5, 20.5_

  - [~] 17.5 Write property test for namespace isolation
    - **Property 14: Namespace Isolation**
    - **Validates: Requirements 7.1, 7.2, 7.4**
    - Test that queries scoped to namespace A never return results from namespace B
    - Test that facts always have subject, object, and fact in same namespace

  - [~] 17.6 Write property test for soft deletion filtering
    - **Property 27: Soft Deletion Filtering**
    - **Validates: Requirements 27.1, 27.3**
    - Test that soft-deleted items have deleted_at set to non-NULL timestamp
    - Test that all retrieval queries exclude items with deleted_at != NULL

  - [~] 17.7 Write unit tests for connectors and isolation
    - Test GitHub webhook parsing and idempotency
    - Test Claude Code namespace resolution
    - Test namespace filter application in all queries
    - Test cross-namespace reference validation
    - Test soft deletion with deletion reasons
    - _Requirements: 38.1, 30.1, 7.2, 7.4, 27.1_


- [ ] 18. Procedure extraction and retrieval
  - [~] 18.1 Implement procedure extraction
    - In `src/pipeline/offline/procedures.rs`, add optional procedure flagging to extraction pipeline
    - Store procedures with pattern, category, namespace in loom_procedures
    - Link to source episode identifiers (source_episodes array)
    - Track first_observed, last_observed, observation_count
    - Initialize confidence at 0.3, increment with observations
    - Assign 'extracted' evidence_status to new procedures
    - _Requirements: 33.1, 33.2, 33.3, 33.4, 33.5, 33.6, 33.7_

  - [~] 18.2 Implement procedure promotion logic
    - Promote to evidence_status='promoted' when confidence >= 0.8 and observation_count >= 3
    - Prevent hot tier until 3+ episodes, 7+ days, confidence >= 0.8
    - Generate 768-dimension embedding for procedure pattern via nomic-embed-text
    - _Requirements: 33.8, 14.9_

  - [~] 18.3 Write unit tests for procedure extraction and retrieval
    - Test procedure flagging and storage
    - Test confidence scoring and promotion
    - Test hot tier prevention criteria
    - Test procedure_assist profile filtering
    - _Requirements: 33.1, 33.6, 33.8, 34.1, 34.7_


- [ ] 19. Additional property tests and validation
  - [~] 19.1 Write property test for entity type constraint
    - **Property 4: Entity Type Constraint**
    - **Validates: Requirements 2.2, 2.5, 43.9**
    - Test that all extracted entity types are one of 10 valid types
    - Test that malformed LLM responses with invalid types are rejected by serde

  - [~] 19.2 Write property test for pack-aware predicate promotion
    - **Property 11: Pack-Aware Predicate Promotion**
    - **Validates: Requirements 5.6, 5.7**
    - Test that promoted custom predicates have promoted_to_pack referencing a valid pack

  - [~] 19.3 Write property test for current fact filtering
    - **Property 13: Current Fact Filtering**
    - **Validates: Requirements 6.6, 10.1, 10.3**
    - Test that fact retrieval without historical flag only returns facts with valid_until IS NULL and deleted_at IS NULL

  - [~] 19.4 Write property test for evidence status validity
    - **Property 28: Evidence Status Validity**
    - **Validates: Requirements 32.1**
    - Test that all facts have one of 7 valid evidence_status values

  - [~] 19.5 Write property test for fact provenance non-empty
    - **Property 29: Fact Provenance Non-Empty**
    - **Validates: Requirements 4.11, 41.1**
    - Test that all facts have non-empty source_episodes array

  - [~] 19.6 Write property test for alias deduplication
    - **Property 30: Alias Deduplication**
    - **Validates: Requirements 42.3**
    - Test that entity aliases arrays have no case-insensitive duplicates

- [ ] 20. Error handling and resilience
  - [~] 20.1 Implement episode ingestion error handling
    - Handle duplicate detection (return existing episode_id with status "duplicate")
    - Handle invalid data via serde deserialization (return 400 Bad Request with specific field errors)
    - Handle embedding generation failures (retry 3x with exponential backoff via Ollama)
    - Queue failed episodes for retry (mark processed=false)
    - Log all errors via tracing with span context

  - [~] 20.2 Implement entity resolution error handling
    - Handle semantic similarity API failures (fall back to creating new entity — prefer fragmentation)
    - Handle ambiguous resolutions (create new entity, log conflict to loom_resolution_conflicts)
    - Handle entity type constraint violations (serde rejection, skip entity, log error)
    - Mark entities for manual review in dashboard on failures

  - [~] 20.3 Implement fact extraction error handling
    - Handle invalid entity references (skip fact, log reference error)
    - Handle predicate validation failures (skip fact, log error)
    - Handle supersession conflicts (use most recent valid_from)
    - Handle pack loading failure (fall back to core pack only, log warning)
    - Continue processing other facts on individual failures

  - [~] 20.4 Implement retrieval and compilation error handling
    - Handle namespace not found (fall back to 'default' namespace)
    - Handle profile execution timeouts (cancel via tokio timeout at 5 seconds, continue with other profiles)
    - Handle empty result sets (return empty context package with explanation)
    - Handle token budget exceeded (trim warm tier candidates)
    - Handle classification failures (default to TaskClass::Chat)
    - Handle ranking dimension errors (use default score 0.5)
    - Log all errors via tracing

  - [~] 20.5 Implement database and external service error handling
    - Handle PostgreSQL connection failures (sqlx pool auto-reconnect, retry 3x, return 503)
    - Handle constraint violations (treat as duplicate, return existing record)
    - Handle transaction rollbacks (mark episode unprocessed, retry after 5 min via tokio timer)
    - Handle Ollama unavailable (retry 3x with exponential backoff, fall back to Azure OpenAI if configured)
    - Handle Azure OpenAI rate limits (exponential backoff: 1s, 2s, 4s, 8s, 16s, queue after 5 retries)
    - Handle embedding dimension mismatch (reject, log error, retry with correct model)
    - Log all errors via tracing with span context

  - [~] 20.6 Write unit tests for error handling
    - Test all error scenarios with mocked failures (reqwest mock for Ollama)
    - Test retry logic and exponential backoff
    - Test fallback behaviors (Ollama → Azure OpenAI, namespace → default)
    - Test error logging completeness


- [ ] 21. Dashboard React SPA
  - [~] 21.1 Initialize loom-dashboard project
    - Create `loom-dashboard/` directory with Vite + React + TypeScript setup
    - Configure package.json with dependencies: react, react-dom, react-router-dom, typescript, vite, tailwindcss (or similar)
    - Create typed API client module communicating with loom-engine `/dashboard/api/*` endpoints
    - Configure build output for static file serving by Caddy
    - Create Dockerfile for dashboard build (multi-stage: node builder + output copy)
    - _Requirements: 45.8, 49.1_

  - [~] 21.2 Implement pipeline health view
    - Episode counts by source and namespace
    - Entity counts by type
    - Current vs superseded fact counts
    - Queue depth (unprocessed episodes)
    - Model configuration display (Ollama models, fallback status)
    - _Requirements: 49.1_

  - [~] 21.3 Implement compilation trace viewer
    - Paginated list of loom_think calls with timestamp, query, namespace, classification, confidence, latency, token count
    - Drill-down showing candidates found/selected/rejected with per-candidate score breakdown across relevance, recency, stability, provenance
    - _Requirements: 49.2, 49.3_

  - [~] 21.4 Implement knowledge graph explorer
    - Entity search with type and namespace filters
    - Entity detail with properties, aliases, source episodes
    - Visual graph rendering of 1-2 hop neighborhoods (using loom_traverse data)
    - Fact detail with temporal range (valid_from/valid_until) and supersession chain
    - _Requirements: 49.4_

  - [~] 21.5 Implement entity conflict review queue
    - List unresolved conflicts with candidate matches and similarity scores
    - Actions: merge, keep separate, split
    - POST to /dashboard/api/conflicts/:id/resolve
    - _Requirements: 49.5, 23.4, 23.6_

  - [~] 21.6 Implement predicate management views
    - Predicate candidate review: unmapped custom predicates with occurrence counts, example facts; actions to map to existing canonical or promote with target pack selection
    - Predicate pack browser: all packs with predicate counts, drill-down to pack predicates with categories and usage counts, per-namespace active packs, usage heatmap
    - _Requirements: 49.6, 49.7, 5.4, 5.6_

  - [~] 21.7 Implement metrics and quality views
    - Retrieval quality metrics: precision over time, latency percentiles (p50, p95, p99), classification confidence distribution, hot-tier utilization per namespace
    - Extraction quality view: model comparison (Gemma 4 vs gpt-4.1-mini), entity resolution method distribution, custom predicate growth rate, entity fragmentation detection
    - Benchmark comparison view: side-by-side A/B/C condition results
    - _Requirements: 49.8, 49.9, 49.10, 35.7, 48.8_

  - [~] 21.8 Write unit tests for dashboard components
    - Test typed API client with mocked responses
    - Test view rendering with sample data
    - Test conflict resolution and predicate management actions
    - _Requirements: 49.1, 49.5, 49.6_


- [ ] 22. Checkpoint - Full system integration testing
  - Run end-to-end integration tests for all workflows
  - Test complete offline pipeline: loom_learn → embedding → extraction → resolution → facts → supersession → metrics
  - Test complete online pipeline: loom_think → classification → retrieval (parallel) → weighting → ranking → compilation → audit
  - Test loom_recall → direct fact lookup with and without historical flag
  - Test dashboard API endpoints return correct data
  - Test dashboard SPA builds and serves via Caddy
  - Verify namespace isolation across all operations
  - Verify hot/warm tier management with promotion/demotion
  - Verify predicate pack system (core always included, pack-aware prompts)
  - Verify extraction metrics stored as JSONB on episodes
  - Verify connection pool separation (online vs offline)
  - Verify all 5 Docker containers work together
  - Ensure all tests pass, ask the user if questions arise
  - _Requirements: 7.2, 14.1, 15.1, 18.1, 44.6, 44.7, 45.1_

- [ ] 23. Performance optimization and monitoring
  - [~] 23.1 Optimize database queries and connection pools
    - Tune IVFFlat index parameters for vector similarity queries
    - Configure online and offline connection pool sizes independently
    - Add query timeout limits
    - Add query result caching for hot tier items
    - _Requirements: 44.7_

  - [~] 23.2 Implement performance monitoring via tracing
    - Track latency breakdown by stage (classify, retrieve, rank, compile) using tracing spans
    - Track extraction metrics per episode (stored as JSONB)
    - Track resolution method distribution (exact, alias, semantic, new)
    - Track hot tier size per namespace
    - Track retrieval precision over time
    - Surface all metrics in dashboard views
    - _Requirements: 35.1, 35.2, 35.3, 35.4, 35.5, 35.6, 35.7, 36.1, 36.2, 36.3_

  - [~] 23.3 Validate performance targets
    - Validate loom_think p95 < 500ms, p99 < 1000ms
    - Validate loom_learn < 100ms (async return)
    - Validate episode processing < 3 seconds per episode
    - Test 10 concurrent loom_think calls (online pool)
    - Test 100 episodes per minute ingestion (offline pool)
    - _Requirements: 35.7_

  - [~] 23.4 Write unit tests for performance monitoring
    - Test latency measurement accuracy via tracing spans
    - Test metrics aggregation
    - Test percentile calculations (p50, p95, p99)
    - _Requirements: 35.1, 35.6, 35.7_


- [ ] 24. Documentation and deployment finalization
  - [~] 24.1 Create project documentation
    - README.md: project overview, architecture, prerequisites (Docker, PostgreSQL, Ollama), installation, configuration (.env), quick start, MCP endpoint documentation
    - CLAUDE.md: MCP endpoint registration for Claude Code, namespace resolution, manual override examples, usage examples
    - _Requirements: 30.4_

  - [~] 24.2 Finalize Docker deployment
    - Test `docker-compose up` with all 5 services (loom-engine, loom-dashboard, postgres, ollama, caddy)
    - Verify PostgreSQL extensions enabled (pgvector, pgAudit)
    - Verify sqlx migrations run on startup
    - Verify loom-engine serves MCP, REST, and Dashboard API on port 8080
    - Verify Caddy routes correctly (/api/*, /mcp/*, /dashboard/api/* → loom-engine; /* → dashboard static files)
    - Verify Ollama models available (gemma4:26b-a4b-q4, gemma4:e4b, nomic-embed-text)
    - Verify dashboard SPA loads and communicates with API
    - Verify health checks pass for all containers
    - _Requirements: 45.1, 45.2, 45.3, 45.4, 45.5, 45.6, 45.7, 45.8, 45.9, 45.10, 45.11, 46.1, 46.2, 46.3, 46.4, 46.5, 51.1, 51.2, 51.3, 51.4, 51.5_


- [ ] 25. Final validation and benchmark evaluation
  - [~] 25.1 Run comprehensive test suite
    - Execute all Rust unit tests (target 80% coverage)
    - Execute all property tests (34 correctness properties, 100 iterations each via proptest)
    - Execute all integration tests
    - Execute dashboard component tests
    - Verify all tests pass

  - [~] 25.2 Execute benchmark evaluation protocol
    - Prepare 10+ benchmark tasks
    - Run Condition A: No memory (baseline)
    - Run Condition B: Episode-only retrieval
    - Run Condition C: Full Loom (entities + facts + episodes)
    - Measure precision, token reduction, task success rate
    - Display results in dashboard benchmark comparison view
    - _Requirements: 47.1, 47.2, 47.3, 47.4, 47.5, 47.9_

  - [~] 25.3 Validate benchmark success criteria
    - Verify Condition C beats Condition B by >= 15% precision
    - Verify Condition C achieves >= 30% token reduction vs Condition B
    - Verify Condition C maintains task success rate (no regression)
    - _Requirements: 47.6, 47.7, 47.8_

  - [~] 25.4 Final system validation
    - Verify all 52 requirements are covered by implementation
    - Verify all 34 correctness properties pass
    - Verify extraction quality gate passed (50 episodes)
    - Verify benchmark evaluation passed
    - Verify performance targets met (p95 < 500ms, p99 < 1000ms)
    - Verify all 5 Docker containers operational
    - Verify dashboard functional with all 9 views

  - [~] 25.5 Checkpoint - Final review
    - Review all implementation artifacts
    - Review test coverage and results
    - Review benchmark evaluation results
    - Review documentation completeness
    - Ensure all tests pass, ask the user if questions arise

## Notes

- All testing tasks are required and must not be skipped
- Each task references specific requirements for traceability
- Checkpoints at tasks 5, 9, 14, 22, and 25 ensure incremental validation
- Property tests validate 34 universal correctness properties (100 iterations each via proptest)
- Unit tests validate specific examples, edge cases, and error conditions
- All Rust code uses compile-time checked SQL queries via sqlx
- All LLM response parsing uses strict serde deserialization
- Online and offline pipelines use separate sqlx::PgPool instances
- Docker deployment includes 5 containers: loom-engine, loom-dashboard, postgres, ollama, caddy
- Embeddings are 768 dimensions via nomic-embed-text (not 1536)
- Ranking weights: relevance 0.40, recency 0.25, stability 0.20, provenance 0.15
- Primary LLM inference via Ollama (Gemma 4 models), Azure OpenAI as fallback only
