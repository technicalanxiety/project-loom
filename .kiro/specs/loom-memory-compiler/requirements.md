# Requirements Document

## Introduction

Project Loom is a PostgreSQL-native memory compiler for AI workflows that provides evidence-grounded memory, strict scoping, shallow graph traversal, and inspectable context assembly. The system is implemented as a single Rust binary (loom-engine) using the tokio async runtime and axum HTTP framework, with local LLM inference via Ollama (Gemma 4 models) and nomic-embed-text embeddings. It maintains three memory types (episodic, semantic, procedural) with strict namespace isolation, enabling AI assistants to retrieve relevant context from past interactions while maintaining audit trails and data integrity. An operational React dashboard provides pipeline health monitoring, compilation trace viewing, conflict review, predicate pack management, and retrieval quality metrics. The system deploys via Docker Compose with five containers: loom-engine, loom-dashboard, PostgreSQL, Ollama, and Caddy reverse proxy.

## Glossary

- **Loom_Engine**: The single Rust binary serving MCP, REST, dashboard API endpoints, background workers, and scheduled tasks via tokio async runtime and axum HTTP framework
- **Episode**: An immutable interaction record representing raw evidence from a source system
- **Entity**: A graph node representing a person, organization, project, service, technology, pattern, environment, document, metric, or decision
- **Fact**: A temporal graph edge representing a relationship between two entities with provenance tracking
- **Predicate**: The relationship type in a fact triple (subject-predicate-object)
- **Predicate_Pack**: A domain vocabulary set grouping related predicates (core, grc, healthcare, finserv)
- **Namespace**: An isolated scope for memory storage with no cross-namespace retrieval
- **Hot_Tier**: Memory always injected into compiled context packages
- **Warm_Tier**: Memory retrieved per-query based on relevance
- **Context_Compiler**: The online pipeline component that assembles memory packages for AI queries
- **Extraction_Pipeline**: The offline pipeline component that processes episodes into entities and facts via tokio spawned tasks
- **Resolution_Algorithm**: The three-pass entity matching process (exact, alias, semantic)
- **Retrieval_Profile**: A named strategy for finding relevant memory (fact_lookup, episode_recall, graph_neighborhood, procedure_assist)
- **Task_Class**: The classified intent of a query (debug, architecture, compliance, writing, chat)
- **Evidence_Status**: The reliability classification of a fact (user_asserted, observed, extracted, inferred, promoted, deprecated, superseded)
- **Canonical_Predicate**: A predefined relationship type in the predicate registry belonging to a specific pack
- **Custom_Predicate**: An extracted relationship type not in the canonical registry, tracked as a candidate
- **MCP_Interface**: Model Context Protocol endpoints for AI tool integration (loom_think, loom_learn, loom_recall)
- **Dashboard**: A Vite + React SPA providing operational views into pipeline health, compilation traces, conflict review, predicate management, and retrieval quality metrics
- **Caddy_Proxy**: Reverse proxy providing TLS termination, static file serving for the dashboard, and routing to the loom-engine
- **Ollama_Service**: Local LLM inference server running Gemma 4 26B MoE (extraction), Gemma 4 E4B (classification), and nomic-embed-text (embeddings)
- **Extraction_Metrics**: A structured record of per-episode extraction statistics stored as JSONB on the episode record
- **Ingestion_Mode**: The provenance class of an episode — one of `user_authored_seed` (Mode 1), `vendor_import` (Mode 2), or `live_mcp_capture` (Mode 3). Stored on every episode row and enforced by CHECK constraint.
- **Sole_Source_Flag**: A per-item boolean on compiled output indicating that the item's only provenance is a `user_authored_seed` episode — no live or vendor corroboration exists.
- **Parser_Version**: Semantic version identifier of a bootstrap parser (e.g. `claude_ai_parser@0.3.1`) populated on every `vendor_import` episode.
- **Parser_Source_Schema**: Vendor export schema version a bootstrap parser asserts against (e.g. `claude_ai_export_v2`) populated on every `vendor_import` episode.
- **LLM_Reconstruction_Trap**: The architectural pattern in which LLM summaries or paraphrases enter the authority hierarchy as first-class evidence, poisoning the provenance chain. Rejected by design — no ingestion mode accepts it, the `content` verbatim invariant forbids it, shipped client templates prevent it.


## Requirements

### Requirement 1: Episode Ingestion with Idempotency

**User Story:** As a developer, I want to ingest interaction records from multiple sources, so that the system builds a complete memory history without duplicates.

#### Acceptance Criteria

1. WHEN an episode is submitted via loom_learn, THE Loom_Engine SHALL store the episode with source, content, occurred_at timestamp, namespace, and ingestion_mode
2. WHEN an episode with an existing source and source_event_id combination is submitted, THE Loom_Engine SHALL skip ingestion and return a duplicate indicator
3. THE Loom_Engine SHALL compute a SHA-256 content hash for each episode using the sha2 crate
4. THE Loom_Engine SHALL record ingestion timestamp and source metadata for each episode
5. WHERE an episode includes participant information, THE Loom_Engine SHALL store the participant list
6. THE Loom_Engine SHALL accept a free-form `source` string identifier on ingestion — not an enumerated type — so new clients can self-identify without schema changes. Canonical source identifiers include (non-exhaustive): `claude-code`, `claude-desktop`, `chatgpt`, `github-copilot`, `m365-copilot`, `manual`, `github`
7. THE Loom_Engine SHALL enforce that episode `content` is verbatim — a transcript excerpt, a vendor export excerpt, or user-authored prose — and never LLM summarization output; enforcement lives in the MCP tool contract, shipped client templates, and user discipline, not at runtime

### Requirement 2: Entity Extraction with Type Constraints

**User Story:** As a system operator, I want entities extracted from episodes with strict type constraints, so that the knowledge graph maintains semantic consistency.

#### Acceptance Criteria

1. WHEN an episode is processed, THE Extraction_Pipeline SHALL extract entities using a structured prompt executed against Ollama_Service
2. THE Extraction_Pipeline SHALL constrain entity types to exactly ten types: person, organization, project, service, technology, pattern, environment, document, metric, decision
3. THE Extraction_Pipeline SHALL extract the most specific common name for each entity
4. THE Extraction_Pipeline SHALL extract alias names for entities when present in the episode
5. THE Extraction_Pipeline SHALL deserialize LLM responses into strict Rust types via serde, rejecting malformed responses at the type boundary
6. THE Extraction_Pipeline SHALL record the extraction model identifier used for each episode
7. THE Extraction_Pipeline SHALL log entity extraction counts per episode

### Requirement 3: Three-Pass Entity Resolution

**User Story:** As a system operator, I want entities resolved through exact, alias, and semantic matching, so that the same real-world entity is not fragmented across multiple nodes.

#### Acceptance Criteria

1. WHEN an extracted entity is processed, THE Resolution_Algorithm SHALL attempt exact match on lowercase name, entity type, and namespace
2. IF exact match succeeds, THEN THE Resolution_Algorithm SHALL merge with the existing entity with confidence 1.0
3. IF exact match fails, THEN THE Resolution_Algorithm SHALL attempt alias match in both directions
4. WHEN alias match finds exactly one candidate, THE Resolution_Algorithm SHALL merge and append the new name to the aliases array with confidence 0.95
5. IF alias match finds multiple candidates, THEN THE Resolution_Algorithm SHALL proceed to semantic matching
6. WHEN semantic matching is attempted, THE Resolution_Algorithm SHALL embed the entity name with context and query existing entities by cosine similarity
7. IF the top semantic candidate exceeds 0.92 similarity AND the gap to the second candidate is at least 0.03, THEN THE Resolution_Algorithm SHALL merge with confidence equal to the similarity score
8. IF semantic candidates are within 0.03 of each other, THEN THE Resolution_Algorithm SHALL create a new entity and log a resolution conflict
9. IF no semantic candidate exceeds 0.92, THEN THE Resolution_Algorithm SHALL create a new entity with confidence 1.0
10. THE Resolution_Algorithm SHALL log resolution method (exact, alias, semantic, new) for each entity


### Requirement 4: Fact Extraction with Canonical Predicates and Pack-Aware Prompts

**User Story:** As a system operator, I want facts extracted using a canonical predicate registry organized into packs, so that relationship types remain consistent and domain-specific vocabularies are available per namespace.

#### Acceptance Criteria

1. WHEN an episode is processed, THE Extraction_Pipeline SHALL extract facts as subject-predicate-object triples via Ollama_Service
2. THE Extraction_Pipeline SHALL validate that subject and object reference extracted entities
3. THE Extraction_Pipeline SHALL dynamically assemble the fact extraction prompt by loading predicates from the namespace's configured predicate packs
4. THE Extraction_Pipeline SHALL always include the core Predicate_Pack in the extraction prompt regardless of namespace configuration
5. THE Extraction_Pipeline SHALL format loaded predicates into a grouped prompt block organized by pack
6. THE Extraction_Pipeline SHALL check each predicate against the canonical predicate registry
7. WHEN a predicate matches a canonical predicate, THE Extraction_Pipeline SHALL use the canonical form and mark custom as false
8. WHEN a predicate does not match any canonical predicate, THE Extraction_Pipeline SHALL mark custom as true and record it in the predicate candidates table
9. THE Extraction_Pipeline SHALL classify evidence strength as explicit or implied for each fact
10. WHERE temporal markers are present in the episode, THE Extraction_Pipeline SHALL extract valid_from and valid_until timestamps
11. THE Extraction_Pipeline SHALL link each fact to its source episode identifiers
12. THE Extraction_Pipeline SHALL log canonical and custom predicate counts per episode

### Requirement 5: Custom Predicate Tracking with Pack-Aware Promotion

**User Story:** As a system operator, I want custom predicates tracked for review and promotable to specific packs, so that frequently used relationships can be organized into the appropriate domain vocabulary.

#### Acceptance Criteria

1. WHEN a custom predicate is encountered, THE Loom_Engine SHALL check for an existing candidate entry
2. IF a candidate entry exists, THEN THE Loom_Engine SHALL increment the occurrence count and append the fact identifier
3. IF no candidate entry exists, THEN THE Loom_Engine SHALL create a new predicate candidate record
4. WHEN a predicate candidate reaches 5 occurrences, THE Loom_Engine SHALL flag it for operator review via the Dashboard
5. THE Loom_Engine SHALL allow operators to map custom predicates to canonical predicates or promote them to canonical status
6. WHEN promoting a custom predicate, THE Loom_Engine SHALL require the operator to select a target Predicate_Pack
7. THE Loom_Engine SHALL record the target pack in the promoted_to_pack field on the predicate candidate record

### Requirement 6: Temporal Fact Management with Supersession

**User Story:** As a developer, I want facts to track validity periods and supersession, so that the system represents how relationships change over time.

#### Acceptance Criteria

1. THE Loom_Engine SHALL store valid_from timestamp for each fact
2. THE Loom_Engine SHALL store valid_until timestamp for facts that are no longer current
3. WHEN a new fact contradicts an existing fact with the same subject, predicate, and object, THE Loom_Engine SHALL set valid_until on the old fact to the new fact's valid_from
4. THE Loom_Engine SHALL record the superseding fact identifier in the superseded_by field
5. THE Loom_Engine SHALL log supersession count per ingestion
6. WHEN retrieving facts, THE Loom_Engine SHALL filter to facts where valid_until is NULL unless historical queries are explicitly requested

### Requirement 7: Namespace Isolation

**User Story:** As a developer, I want memory strictly isolated by namespace, so that project-specific knowledge does not leak across boundaries.

#### Acceptance Criteria

1. THE Loom_Engine SHALL assign exactly one namespace to each episode, entity, fact, and procedure
2. THE Loom_Engine SHALL scope all retrieval queries to a single namespace
3. THE Loom_Engine SHALL maintain a default namespace for general knowledge
4. THE Loom_Engine SHALL prevent cross-namespace entity references
5. THE Loom_Engine SHALL maintain separate hot tier budgets per namespace
6. WHEN the same real-world entity appears in multiple namespaces, THE Loom_Engine SHALL create separate entity records

### Requirement 8: Intent Classification with Confidence Scoring

**User Story:** As a developer, I want queries classified by intent with confidence scores, so that retrieval strategies match the task type.

#### Acceptance Criteria

1. WHEN a query is submitted via loom_think, THE Context_Compiler SHALL classify the intent into one of five task classes: debug, architecture, compliance, writing, chat using Gemma 4 E4B via Ollama_Service
2. THE Context_Compiler SHALL compute a primary confidence score for the classification
3. WHEN the confidence gap between the top two classes is less than 0.3, THE Context_Compiler SHALL record both primary and secondary task classes
4. WHEN the confidence gap is at least 0.3, THE Context_Compiler SHALL use only the primary class
5. IF classification is ambiguous with no clear signal, THEN THE Context_Compiler SHALL default to chat task class
6. THE Context_Compiler SHALL log primary class, secondary class, and confidence scores to the audit log

### Requirement 9: Retrieval Profile Execution

**User Story:** As a developer, I want retrieval strategies selected based on task class, so that the most relevant memory types are prioritized.

#### Acceptance Criteria

1. THE Context_Compiler SHALL map debug task class to graph_neighborhood and episode_recall profiles
2. THE Context_Compiler SHALL map architecture task class to fact_lookup and graph_neighborhood profiles
3. THE Context_Compiler SHALL map compliance task class to episode_recall and fact_lookup profiles
4. THE Context_Compiler SHALL map writing task class to fact_lookup profile
5. THE Context_Compiler SHALL map chat task class to fact_lookup profile
6. WHEN a secondary task class is present, THE Context_Compiler SHALL merge profiles from both classes and deduplicate
7. THE Context_Compiler SHALL cap the number of active profiles at 3
8. THE Context_Compiler SHALL execute all active profiles in parallel via tokio::join!
9. THE Context_Compiler SHALL log executed profile names to the audit log

### Requirement 10: Fact Lookup Retrieval Strategy

**User Story:** As a developer, I want semantic facts retrieved by relevance, so that current knowledge answers my query.

#### Acceptance Criteria

1. WHEN fact_lookup profile executes, THE Context_Compiler SHALL retrieve facts where valid_until is NULL
2. THE Context_Compiler SHALL filter facts to the active namespace
3. THE Context_Compiler SHALL filter facts where deleted_at is NULL
4. THE Context_Compiler SHALL rank facts by vector similarity between the query and source episodes
5. THE Context_Compiler SHALL boost facts where entity names match query terms
6. THE Context_Compiler SHALL return up to 20 fact candidates

### Requirement 11: Episode Recall Retrieval Strategy

**User Story:** As a developer, I want recent episodes retrieved by relevance, so that I can trace back to original evidence.

#### Acceptance Criteria

1. WHEN episode_recall profile executes, THE Context_Compiler SHALL retrieve episodes from the active namespace
2. THE Context_Compiler SHALL filter episodes where deleted_at is NULL
3. THE Context_Compiler SHALL rank episodes by vector similarity to the query
4. THE Context_Compiler SHALL apply recency weighting favoring recent occurred_at timestamps
5. THE Context_Compiler SHALL return up to 10 episode candidates

### Requirement 12: Graph Neighborhood Retrieval Strategy

**User Story:** As a developer, I want related entities discovered through graph traversal, so that I understand connections between concepts.

#### Acceptance Criteria

1. WHEN graph_neighborhood profile executes, THE Context_Compiler SHALL identify entities mentioned in the query
2. THE Context_Compiler SHALL perform 1-hop traversal from identified entities
3. IF 1-hop traversal returns fewer than 3 candidates, THEN THE Context_Compiler SHALL perform 2-hop traversal
4. THE Context_Compiler SHALL prevent cycles by tracking visited entities in the traversal path
5. THE Context_Compiler SHALL filter facts to the active namespace
6. THE Context_Compiler SHALL filter facts where valid_until is NULL and deleted_at is NULL
7. THE Context_Compiler SHALL return entities and connecting facts from the traversal

### Requirement 13: Memory Weight Modifiers

**User Story:** As a developer, I want memory types weighted by task class, so that retrieval prioritizes the most useful evidence for each task.

#### Acceptance Criteria

1. THE Context_Compiler SHALL apply weight 1.0 to episodic memory for debug task class
2. THE Context_Compiler SHALL apply weight 1.0 to semantic memory for architecture task class
3. THE Context_Compiler SHALL apply weight 1.0 to episodic memory for compliance task class
4. THE Context_Compiler SHALL apply weight 0.0 to procedural memory for compliance task class
5. THE Context_Compiler SHALL multiply each candidate's relevance score by the memory type weight for the active task class
6. WHEN weight is 0.0, THE Context_Compiler SHALL exclude the candidate from ranking


### Requirement 14: Hot Tier Management

**User Story:** As a developer, I want critical memory always available in context, so that frequently used knowledge does not require retrieval.

#### Acceptance Criteria

1. THE Loom_Engine SHALL maintain a hot tier for entities and facts that are always injected into compiled context
2. THE Loom_Engine SHALL configure hot tier token budget per namespace with a default of 500 tokens
3. WHEN a user explicitly pins memory, THE Loom_Engine SHALL promote it to hot tier
4. WHEN memory is retrieved and used in 5 or more compilations within 14 days, THE Loom_Engine SHALL promote it to hot tier
5. WHEN pinned memory is unpinned by the user, THE Loom_Engine SHALL demote it to warm tier
6. WHEN hot tier memory is not retrieved in 30 days, THE Loom_Engine SHALL demote it to warm tier
7. WHEN a fact is superseded, THE Loom_Engine SHALL prevent it from being hot tier
8. WHEN hot tier exceeds the namespace token budget, THE Loom_Engine SHALL demote the lowest-salience hot item to warm tier
9. THE Loom_Engine SHALL prevent procedures from being hot tier until observed in 3 or more distinct episodes across 7 or more days AND confidence is at least 0.8

### Requirement 15: Warm Tier Management

**User Story:** As a developer, I want memory retrieved on demand by relevance, so that the system scales to large knowledge bases.

#### Acceptance Criteria

1. THE Loom_Engine SHALL assign all new facts to warm tier by default
2. THE Loom_Engine SHALL configure warm tier token budget per namespace with a default of 3000 tokens
3. WHEN a fact is superseded, THE Loom_Engine SHALL archive it
4. WHEN a fact is not accessed in 90 days, THE Loom_Engine SHALL archive it
5. THE Loom_Engine SHALL maintain archived facts as searchable but exclude them from automatic retrieval

### Requirement 16: Context Package Compilation with XML and JSON Output Formats

**User Story:** As a developer, I want retrieved memory compiled into structured packages using XML-like tags for Claude models and JSON for local models, so that AI models receive well-formatted context optimized for their capabilities.

#### Acceptance Criteria

1. WHEN compilation is requested, THE Context_Compiler SHALL merge candidates from all executed profiles
2. THE Context_Compiler SHALL deduplicate candidates by identifier
3. THE Context_Compiler SHALL rank candidates on four weighted dimensions: relevance (0.40), recency (0.25), stability (0.20), provenance (0.15)
4. THE Context_Compiler SHALL trim candidates to fit the namespace warm tier token budget
5. THE Context_Compiler SHALL inject all hot tier memory for the namespace
6. THE Context_Compiler SHALL format the structured output using XML-like tags: loom, identity, project, knowledge, episodes, patterns
7. THE Context_Compiler SHALL include model, token count, namespace, and task class as attributes on the root loom tag in structured format
8. THE Context_Compiler SHALL format the compact output as a JSON object with ns, task, identity, facts, recent, and patterns fields
9. THE Context_Compiler SHALL include provenance information for each memory item
10. THE Context_Compiler SHALL compute total token count for the compiled package

### Requirement 17: MCP Interface for AI Integration

**User Story:** As an AI assistant, I want Model Context Protocol endpoints, so that I can learn from interactions and retrieve memory.

#### Acceptance Criteria

1. THE Loom_Engine SHALL expose a loom_learn endpoint that accepts episode content, source, namespace, and metadata
2. THE Loom_Engine SHALL expose a loom_think endpoint that accepts a query, namespace, and optional task class override
3. THE Loom_Engine SHALL expose a loom_recall endpoint that accepts a query, namespace, optional memory type filter, and optional time range
4. THE Loom_Engine SHALL return episode identifiers from loom_learn calls with status accepted, duplicate, or queued
5. THE Loom_Engine SHALL return compiled context packages from loom_think calls in structured or compact format
6. THE Loom_Engine SHALL return raw search results from loom_recall calls without compilation
7. THE Loom_Engine SHALL process loom_learn calls asynchronously without blocking the caller

### Requirement 18: Comprehensive Audit Logging

**User Story:** As a system operator, I want every compilation decision logged, so that I can analyze retrieval quality and debug issues.

#### Acceptance Criteria

1. WHEN a compilation is executed, THE Loom_Engine SHALL log the task class, namespace, query text, and target model
2. THE Loom_Engine SHALL log primary and secondary classification with confidence scores
3. THE Loom_Engine SHALL log the list of executed retrieval profiles
4. THE Loom_Engine SHALL log candidate counts: found, selected, rejected
5. THE Loom_Engine SHALL log selected items with memory type, identifier, and score breakdown across relevance, recency, stability, and provenance dimensions
6. THE Loom_Engine SHALL log rejected items with rejection reason
7. THE Loom_Engine SHALL log compiled token count and output format
8. THE Loom_Engine SHALL log latency breakdown: total, classification, retrieval, ranking, compilation using the tracing crate with span-based instrumentation
9. THE Loom_Engine SHALL log user rating when provided

### Requirement 19: Extraction Quality Evaluation

**User Story:** As a system operator, I want extraction quality measured against thresholds comparing local and API models, so that I can select the best extraction model and validate the system before production use.

#### Acceptance Criteria

1. THE Loom_Engine SHALL compute entity precision as the ratio of correctly resolved entities to total entities extracted with a target of at least 0.80
2. THE Loom_Engine SHALL compute entity recall as the ratio of correctly resolved entities to total expected entities with a target of at least 0.70
3. THE Loom_Engine SHALL compute fact precision as the ratio of valid facts to total facts extracted with a target of at least 0.75
4. THE Loom_Engine SHALL compute fact recall as the ratio of correct facts to total expected facts with a target of at least 0.60
5. THE Loom_Engine SHALL compute predicate consistency as the ratio of canonical predicates to total predicates used with a target of at least 0.85
6. THE Loom_Engine SHALL evaluate extraction quality after 50 episodes
7. THE Loom_Engine SHALL run extraction quality evaluation against both Gemma 4 26B MoE (local via Ollama) and gpt-4.1-mini (API) in parallel
8. IF Gemma 4 26B MoE meets all thresholds, THEN THE Loom_Engine SHALL use the local model to eliminate API dependency
9. IF Gemma 4 26B MoE fails but gpt-4.1-mini passes, THEN THE Loom_Engine SHALL use the API model with a planned local model upgrade path

### Requirement 20: PostgreSQL Schema with Extensions

**User Story:** As a system operator, I want all memory stored in PostgreSQL with vector and audit extensions, so that the system has a single source of truth.

#### Acceptance Criteria

1. THE Loom_Engine SHALL store all episodes, entities, facts, and procedures in PostgreSQL tables with compile-time checked queries via sqlx
2. THE Loom_Engine SHALL use the pgvector extension for embedding storage and similarity search
3. THE Loom_Engine SHALL use the pgAudit extension for compliance logging
4. THE Loom_Engine SHALL separate canonical fields from derived serving state in the schema
5. THE Loom_Engine SHALL implement soft deletion with deleted_at timestamps
6. THE Loom_Engine SHALL enforce unique constraints on episode source and source_event_id
7. THE Loom_Engine SHALL enforce unique constraints on entity name, type, and namespace
8. THE Loom_Engine SHALL create indexes on namespace, timestamps, embeddings, and foreign keys

### Requirement 21: Episode Embedding Generation

**User Story:** As a developer, I want episodes embedded for similarity search, so that retrieval finds semantically related interactions.

#### Acceptance Criteria

1. WHEN an episode is ingested, THE Loom_Engine SHALL generate a 768-dimension embedding using nomic-embed-text via Ollama_Service
2. THE Loom_Engine SHALL store the embedding in the episodes table
3. THE Loom_Engine SHALL create an IVFFlat index on episode embeddings for cosine similarity search
4. THE Loom_Engine SHALL filter embedding queries to exclude soft-deleted episodes

### Requirement 22: Entity Embedding Generation

**User Story:** As a developer, I want entities embedded for semantic resolution, so that similar entities can be matched even with different names.

#### Acceptance Criteria

1. WHEN an entity is created or updated, THE Loom_Engine SHALL generate a 768-dimension embedding combining entity name and context using nomic-embed-text via Ollama_Service
2. THE Loom_Engine SHALL store the embedding in the entity serving state table
3. THE Loom_Engine SHALL use entity embeddings for semantic similarity matching during resolution
4. THE Loom_Engine SHALL filter embedding queries to exclude soft-deleted entities

### Requirement 23: Resolution Conflict Tracking

**User Story:** As a system operator, I want ambiguous entity resolutions logged for review via the dashboard, so that I can prevent incorrect merges and fix fragmentation.

#### Acceptance Criteria

1. WHEN semantic resolution finds multiple candidates within 0.03 similarity, THE Loom_Engine SHALL create a resolution conflict record
2. THE Loom_Engine SHALL store entity name, type, namespace, and candidate details in the conflict record
3. THE Loom_Engine SHALL mark conflicts as unresolved by default
4. THE Loom_Engine SHALL allow operators to record resolution decisions via the Dashboard: merged, kept_separate, or split
5. THE Loom_Engine SHALL timestamp resolution decisions
6. THE Loom_Engine SHALL surface unresolved conflicts in the Dashboard entity conflict review queue

### Requirement 24: Entity Health Check

**User Story:** As a system operator, I want potential duplicate entities detected, so that I can merge fragmented knowledge.

#### Acceptance Criteria

1. THE Loom_Engine SHALL execute a weekly entity health check query via a tokio scheduled task
2. THE Loom_Engine SHALL identify entity pairs in the same namespace and type with embedding similarity above 0.85
3. THE Loom_Engine SHALL rank potential duplicates by similarity score
4. THE Loom_Engine SHALL return the top 50 potential duplicate pairs
5. THE Loom_Engine SHALL exclude soft-deleted entities from health checks
6. THE Loom_Engine SHALL surface entity health check results in the Dashboard


### Requirement 25: Predicate Pack System

**User Story:** As a system operator, I want predicates organized into domain-specific packs, so that namespaces can use vocabulary sets appropriate to their domain.

#### Acceptance Criteria

1. THE Loom_Engine SHALL maintain a predicate pack registry with at least two packs: core and grc
2. THE Loom_Engine SHALL store each pack with a name and description in the loom_predicate_packs table
3. THE Loom_Engine SHALL assign each canonical predicate to exactly one Predicate_Pack
4. THE Loom_Engine SHALL categorize predicates as structural, temporal, decisional, operational, or regulatory
5. THE Loom_Engine SHALL define inverse relationships for bidirectional predicates
6. THE Loom_Engine SHALL include descriptions for each canonical predicate
7. THE Loom_Engine SHALL track usage count for each predicate
8. THE Loom_Engine SHALL seed the core pack with at least 25 predicates including: uses, contains, depends_on, replaced_by, deployed_to, implements, decided, integrates_with, manages, owns
9. THE Loom_Engine SHALL seed the grc pack with at least 23 regulatory predicates including: scoped_as, maps_to_control, exception_granted_for, evidenced_by, satisfies, finding_on, conflicts_with
10. THE Loom_Engine SHALL index predicates by pack for efficient lookup

### Requirement 26: Graph Traversal with Cycle Prevention

**User Story:** As a developer, I want to traverse entity relationships without infinite loops, so that graph queries complete reliably.

#### Acceptance Criteria

1. THE Loom_Engine SHALL provide a graph traversal SQL function accepting entity identifier, max hops, and namespace
2. THE Loom_Engine SHALL default max hops to 2
3. THE Loom_Engine SHALL track visited entities in the traversal path
4. THE Loom_Engine SHALL prevent revisiting entities already in the path
5. THE Loom_Engine SHALL traverse facts in both subject and object directions
6. THE Loom_Engine SHALL filter traversal to current facts where valid_until is NULL
7. THE Loom_Engine SHALL filter traversal to the specified namespace
8. THE Loom_Engine SHALL return entities, connecting facts, predicates, evidence status, hop depth, and path for each result

### Requirement 27: Soft Deletion with Audit Trail

**User Story:** As a system operator, I want memory soft-deleted rather than physically removed, so that deletions are reversible and auditable.

#### Acceptance Criteria

1. WHEN memory is deleted, THE Loom_Engine SHALL set deleted_at timestamp to the current time
2. THE Loom_Engine SHALL record deletion reason when provided
3. THE Loom_Engine SHALL exclude soft-deleted records from all retrieval queries by default
4. THE Loom_Engine SHALL maintain soft-deleted records in the database
5. THE Loom_Engine SHALL allow operators to query soft-deleted records for audit purposes

### Requirement 28: Namespace Configuration with Predicate Pack Assignment

**User Story:** As a system operator, I want namespace-specific memory budgets and predicate pack assignments, so that different projects can have different context sizes and domain vocabularies.

#### Acceptance Criteria

1. THE Loom_Engine SHALL maintain a configuration table for namespace settings
2. THE Loom_Engine SHALL configure hot tier token budget per namespace with a default of 500 tokens
3. THE Loom_Engine SHALL configure warm tier token budget per namespace with a default of 3000 tokens
4. THE Loom_Engine SHALL configure predicate packs per namespace with a default of core only
5. THE Loom_Engine SHALL allow operators to assign additional predicate packs to a namespace
6. THE Loom_Engine SHALL enforce that the core pack is always included regardless of namespace configuration
7. THE Loom_Engine SHALL store namespace descriptions
8. THE Loom_Engine SHALL timestamp namespace configuration creation and updates

### Requirement 29: Hot Tier Snapshot Audit

**User Story:** As a system operator, I want periodic snapshots of hot tier content, so that I can audit what memory was always-available at any point in time.

#### Acceptance Criteria

1. THE Loom_Engine SHALL execute a daily hot tier snapshot job via a tokio scheduled task
2. THE Loom_Engine SHALL capture hot entities, facts, and procedures per namespace
3. THE Loom_Engine SHALL compute total token count for each snapshot
4. THE Loom_Engine SHALL timestamp each snapshot
5. THE Loom_Engine SHALL store snapshots in a dedicated audit table

### Requirement 30: Client Integrations

**User Story:** As the primary user of Loom, I want first-class integrations with every AI client I actually use, so that every surface — terminal, desktop chat, IDE, tenant-wide assistant — speaks to the same memory layer with equal footing.

#### Acceptance Criteria

1. THE Loom_Engine SHALL expose MCP endpoints over HTTP transport that any MCP-compliant client can register with
2. THE Loom_Engine SHALL ship a per-client integration guide under `docs/clients/` for each first-class client: Claude Code, Claude Desktop, ChatGPT Desktop, GitHub Copilot (VS Code), and Microsoft 365 Copilot
3. THE Loom_Engine SHALL ship a discipline template under `templates/` for each first-class client, paired with its guide, enforcing the verbatim-content invariant (ADR-005)
4. WHERE a client publishes a conversation export, THE Loom_Engine SHALL ship a bootstrap parser under `bootstrap/` that asserts a pinned schema and POSTs verbatim excerpts as `vendor_import` episodes
5. WHERE a client does not publish a conversation export, THE Loom_Engine SHALL ship a stub parser that exits non-zero with a pointer to the live-capture path in the client guide
6. THE Loom_Engine SHALL support manual namespace override in MCP calls regardless of client
7. WHERE a client supports session-lifecycle hooks (currently Claude Code's PostSession), THE Loom_Engine SHALL ship a shell hook that achieves exhaustive verbatim live capture without model involvement. WHERE it does not, selective MCP `loom_learn` under the shipped discipline template SHALL be the documented path
8. THE Loom_Engine SHALL hardcode `ingestion_mode = live_mcp_capture` at the MCP handler boundary regardless of which client is connected, so client-side forgery of the mode is impossible

### Requirement 31: Extraction Model Tracking and Selection

**User Story:** As a system operator, I want extraction model identifiers recorded and local-first model selection, so that I can compare quality across model versions and minimize cloud dependency.

#### Acceptance Criteria

1. WHEN an episode is processed, THE Loom_Engine SHALL record the extraction model identifier used for entity and fact extraction
2. THE Loom_Engine SHALL record the classification model identifier used for intent classification
3. THE Loom_Engine SHALL support Gemma 4 26B MoE via Ollama as the default extraction model
4. THE Loom_Engine SHALL support Gemma 4 E4B via Ollama as the default classification model
5. THE Loom_Engine SHALL support gpt-4.1-mini via Azure OpenAI as a fallback extraction model
6. THE Loom_Engine SHALL allow operators to query extraction metrics grouped by model identifier
7. THE Loom_Engine SHALL store the extraction_model and classification_model on each episode record for retroactive quality comparison

### Requirement 32: Evidence Status Classification

**User Story:** As a developer, I want facts classified by evidence reliability, so that I can trust high-confidence knowledge and question provisional patterns.

#### Acceptance Criteria

1. THE Loom_Engine SHALL classify each fact with one of seven evidence statuses: user_asserted, observed, extracted, inferred, promoted, deprecated, superseded
2. THE Loom_Engine SHALL assign extracted status to facts derived from LLM extraction by default
3. THE Loom_Engine SHALL assign user_asserted status to facts explicitly stated by users
4. THE Loom_Engine SHALL assign observed status to facts directly witnessed in episodes
5. THE Loom_Engine SHALL assign inferred status to facts derived from multiple other facts
6. THE Loom_Engine SHALL assign promoted status to facts elevated from candidate status
7. THE Loom_Engine SHALL assign deprecated status to facts marked as unreliable
8. THE Loom_Engine SHALL assign superseded status to facts replaced by newer facts
9. THE Loom_Engine SHALL index facts by evidence status for filtering

### Requirement 33: Procedure Extraction and Confidence Scoring

**User Story:** As a developer, I want behavioral patterns extracted from repeated episodes, so that the system learns common workflows.

#### Acceptance Criteria

1. WHEN an episode is processed, THE Extraction_Pipeline SHALL optionally flag candidate procedures
2. THE Loom_Engine SHALL store procedures with pattern description, category, and namespace
3. THE Loom_Engine SHALL link procedures to source episode identifiers
4. THE Loom_Engine SHALL track first observed and last observed timestamps for each procedure
5. THE Loom_Engine SHALL increment observation count when a procedure is seen again
6. THE Loom_Engine SHALL compute confidence score starting at 0.3 and increasing with observations
7. THE Loom_Engine SHALL assign extracted evidence status to new procedures
8. THE Loom_Engine SHALL assign promoted evidence status when confidence reaches 0.8 and observation count reaches 3

### Requirement 34: Procedure Assist Retrieval Strategy

**User Story:** As a developer, I want high-confidence behavioral patterns retrieved, so that the system suggests proven workflows.

#### Acceptance Criteria

1. WHEN procedure_assist profile executes, THE Context_Compiler SHALL retrieve procedures with evidence_status promoted
2. THE Context_Compiler SHALL filter procedures to confidence at least 0.8
3. THE Context_Compiler SHALL filter procedures to observation_count at least 3
4. THE Context_Compiler SHALL filter procedures to the active namespace
5. THE Context_Compiler SHALL filter procedures where deleted_at is NULL
6. THE Context_Compiler SHALL return up to 3 procedure candidates
7. THE Context_Compiler SHALL exclude procedure_assist profile for compliance task class

### Requirement 35: Latency Tracking and Performance Monitoring

**User Story:** As a system operator, I want compilation latency broken down by stage, so that I can identify performance bottlenecks.

#### Acceptance Criteria

1. WHEN a compilation executes, THE Loom_Engine SHALL measure total latency in milliseconds using the tracing crate
2. THE Loom_Engine SHALL measure classification latency separately
3. THE Loom_Engine SHALL measure retrieval latency separately
4. THE Loom_Engine SHALL measure ranking latency separately
5. THE Loom_Engine SHALL measure compilation latency separately
6. THE Loom_Engine SHALL log all latency measurements to the audit log
7. THE Loom_Engine SHALL support percentile queries on latency (p50, p95, p99) via the Dashboard

### Requirement 36: Retrieval Precision Metrics

**User Story:** As a system operator, I want retrieval precision measured over time, so that I can validate that the system improves with more data.

#### Acceptance Criteria

1. THE Loom_Engine SHALL compute retrieval precision as the ratio of selected candidates to found candidates
2. THE Loom_Engine SHALL log candidates_found, candidates_selected, and candidates_rejected for each compilation
3. THE Loom_Engine SHALL support daily aggregation of precision metrics via the Dashboard
4. THE Loom_Engine SHALL support queries identifying most frequently rejected items
5. THE Loom_Engine SHALL log rejection reasons for rejected candidates


### Requirement 37: Manual Ingestion Path

**User Story:** As a developer, I want to manually submit episodes via REST, so that I can seed the system with important knowledge before automated connectors are available.

#### Acceptance Criteria

1. THE Loom_Engine SHALL accept manual episode submissions via loom_learn with source type manual
2. THE Loom_Engine SHALL accept manual episode submissions via REST endpoint POST /api/learn
3. THE Loom_Engine SHALL require content, namespace, and occurred_at timestamp for manual submissions
4. THE Loom_Engine SHALL accept optional metadata and participant lists for manual submissions
5. THE Loom_Engine SHALL process manual episodes through the same extraction pipeline as automated sources
6. THE Loom_Engine SHALL return episode identifier and processing status for manual submissions

### Requirement 38: GitHub Connector

**User Story:** As a developer, I want GitHub events ingested as episodes, so that code review discussions and issue conversations become searchable memory.

#### Acceptance Criteria

1. THE Loom_Engine SHALL accept GitHub webhook events as episode sources
2. THE Loom_Engine SHALL support pull request comment events
3. THE Loom_Engine SHALL support issue comment events
4. THE Loom_Engine SHALL extract occurred_at from GitHub event timestamps
5. THE Loom_Engine SHALL use GitHub event identifiers for idempotency
6. THE Loom_Engine SHALL resolve namespace from GitHub repository name
7. THE Loom_Engine SHALL extract participant lists from GitHub event actors

### Requirement 39: Salience Score Computation

**User Story:** As a developer, I want memory items scored by importance, so that retrieval prioritizes the most valuable knowledge.

#### Acceptance Criteria

1. THE Loom_Engine SHALL compute salience score for each entity and fact
2. THE Loom_Engine SHALL initialize salience score to 0.5 for new items
3. THE Loom_Engine SHALL increase salience score when an item is accessed
4. THE Loom_Engine SHALL track access count for each item
5. THE Loom_Engine SHALL track last accessed timestamp for each item
6. THE Loom_Engine SHALL use salience score as input to the stability ranking dimension during compilation
7. THE Loom_Engine SHALL store salience score in the serving state tables

### Requirement 40: Four-Dimension Weighted Ranking

**User Story:** As a developer, I want candidates ranked on four weighted dimensions, so that the best memory items are selected for context.

#### Acceptance Criteria

1. WHEN ranking candidates, THE Context_Compiler SHALL score each candidate on relevance with weight 0.40 based on cosine similarity modified by memory weight
2. THE Context_Compiler SHALL score each candidate on recency with weight 0.25 based on days since last_accessed or occurred_at with decay
3. THE Context_Compiler SHALL score each candidate on stability with weight 0.20 based on current status, non-superseded status, and evidence_status authority
4. THE Context_Compiler SHALL score each candidate on provenance with weight 0.15 based on source episode count and evidence_status authority
5. THE Context_Compiler SHALL combine the four dimension scores using the specified weights into a final ranking score
6. THE Context_Compiler SHALL sort candidates by final ranking score in descending order
7. THE Context_Compiler SHALL log score breakdown for selected items in the audit log

### Requirement 41: Provenance Tracking

**User Story:** As a developer, I want every fact linked to source episodes, so that I can trace knowledge back to original evidence.

#### Acceptance Criteria

1. THE Loom_Engine SHALL store source episode identifiers for each fact as an array
2. THE Loom_Engine SHALL store source episode identifiers for each entity as an array
3. THE Loom_Engine SHALL store source episode identifiers for each procedure as an array
4. THE Loom_Engine SHALL include provenance information in compiled context packages
5. THE Loom_Engine SHALL allow queries to retrieve all facts derived from a specific episode

### Requirement 42: Alias Accumulation and Deduplication

**User Story:** As a developer, I want entity aliases accumulated over time, so that resolution improves as the system learns more names for the same entity.

#### Acceptance Criteria

1. WHEN an entity is resolved via alias match, THE Loom_Engine SHALL append the new name to the aliases array
2. WHEN an entity is resolved via semantic match, THE Loom_Engine SHALL append the new name to the aliases array
3. THE Loom_Engine SHALL deduplicate aliases using case-insensitive comparison
4. THE Loom_Engine SHALL never remove aliases through automated processes
5. THE Loom_Engine SHALL store aliases in the entity properties JSONB field

### Requirement 43: Extraction Prompt Constraints with Serde Deserialization

**User Story:** As a system operator, I want extraction prompts to enforce strict output formats with Rust type-safe deserialization, so that parsing is reliable and errors are caught at the type boundary.

#### Acceptance Criteria

1. THE Extraction_Pipeline SHALL use structured prompts that specify JSON-only output with no preamble
2. THE Extraction_Pipeline SHALL constrain entity extraction to exactly ten entity types
3. THE Extraction_Pipeline SHALL instruct entity extraction to use the most specific common name
4. THE Extraction_Pipeline SHALL instruct entity extraction to avoid generic concepts
5. THE Extraction_Pipeline SHALL instruct fact extraction to use canonical predicates when available
6. THE Extraction_Pipeline SHALL instruct fact extraction to flag custom predicates
7. THE Extraction_Pipeline SHALL instruct fact extraction to include temporal markers when present
8. THE Extraction_Pipeline SHALL instruct fact extraction to classify evidence strength as explicit or implied
9. THE Extraction_Pipeline SHALL deserialize all LLM responses via serde into strict Rust types, rejecting malformed responses with clear error messages naming the invalid field

### Requirement 44: Asynchronous Offline Processing via Tokio Spawned Tasks

**User Story:** As a developer, I want episode processing to run asynchronously within the same Rust binary, so that ingestion calls return immediately without blocking and no separate worker container is needed.

#### Acceptance Criteria

1. WHEN loom_learn is called, THE Loom_Engine SHALL store the episode and return immediately
2. THE Loom_Engine SHALL process entity extraction asynchronously via tokio spawned tasks
3. THE Loom_Engine SHALL process fact extraction asynchronously via tokio spawned tasks
4. THE Loom_Engine SHALL mark episodes as processed after extraction completes
5. THE Loom_Engine SHALL log extraction metrics after processing completes
6. THE Loom_Engine SHALL never block online compilation queries waiting for offline processing
7. THE Loom_Engine SHALL use separate database connection pools for online and offline pipelines to prevent offline processing from starving the serving path

### Requirement 45: Deployment via Docker Compose with Five Containers

**User Story:** As a system operator, I want Loom deployed as a Docker Compose stack with loom-engine, loom-dashboard, PostgreSQL, Ollama, and Caddy, so that the system runs locally with zero cloud dependency for inference.

#### Acceptance Criteria

1. THE Loom_Engine SHALL deploy as a single Rust binary in a minimal Docker image (approximately 20MB)
2. THE Loom_Engine SHALL expose MCP endpoints on port 8080 at /mcp path
3. THE Loom_Engine SHALL expose REST API endpoints on port 8080 at /api path
4. THE Loom_Engine SHALL expose Dashboard API endpoints on port 8080 at /dashboard path
5. THE Loom_Engine SHALL run background workers and scheduled tasks within the same binary via tokio spawned tasks
6. THE Loom_Engine SHALL connect to PostgreSQL 16 with pgvector and pgAudit extensions
7. THE Loom_Engine SHALL connect to Ollama_Service for local LLM inference
8. THE Dashboard SHALL deploy as a Vite + React static build served by Caddy_Proxy
9. THE Caddy_Proxy SHALL route /api and /mcp and /dashboard/api requests to the Loom_Engine
10. THE Caddy_Proxy SHALL serve Dashboard static files for all other paths
11. THE Caddy_Proxy SHALL provide TLS termination

### Requirement 46: PostgreSQL Extension Requirements

**User Story:** As a system operator, I want pgvector and pgAudit extensions enabled, so that the system supports vector similarity and compliance logging.

#### Acceptance Criteria

1. THE Loom_Engine SHALL require pgvector extension for 768-dimension vector storage and similarity operations
2. THE Loom_Engine SHALL require pgAudit extension for compliance audit logging
3. THE Loom_Engine SHALL create IVFFlat indexes for vector columns using cosine similarity
4. THE Loom_Engine SHALL configure pgAudit to log all data modifications
5. THE Loom_Engine SHALL validate extension availability during startup

### Requirement 47: Benchmark Evaluation Protocol

**User Story:** As a system operator, I want retrieval quality measured against baseline conditions, so that I can validate that Loom improves AI assistant performance.

#### Acceptance Criteria

1. THE Loom_Engine SHALL support benchmark evaluation with three conditions: A (no memory), B (episode-only), C (full Loom)
2. THE Loom_Engine SHALL execute at least 10 benchmark tasks per condition
3. THE Loom_Engine SHALL measure precision as the ratio of relevant retrieved items to total retrieved items
4. THE Loom_Engine SHALL measure token reduction as the difference in context size between conditions
5. THE Loom_Engine SHALL measure task success rate as the ratio of correctly completed tasks to total tasks
6. THE Loom_Engine SHALL require condition C to beat condition B by at least 15% precision
7. THE Loom_Engine SHALL require condition C to achieve at least 30% token reduction compared to condition B
8. THE Loom_Engine SHALL require condition C to maintain task success rate with no regression
9. THE Loom_Engine SHALL display benchmark results in the Dashboard benchmark comparison view

### Requirement 48: Extraction Metrics per Episode

**User Story:** As a system operator, I want per-episode extraction metrics stored as structured JSONB, so that I can analyze extraction quality trends and compare model performance over time.

#### Acceptance Criteria

1. WHEN an episode is processed, THE Loom_Engine SHALL compute extraction metrics including entity counts by resolution method, fact counts by predicate type, evidence strength counts, and processing time
2. THE Loom_Engine SHALL serialize extraction metrics as a structured JSONB object on the episode record in the extraction_metrics column
3. THE Loom_Engine SHALL include the extraction model identifier in the metrics
4. THE Loom_Engine SHALL include entity counts: extracted, resolved_exact, resolved_alias, resolved_semantic, new, conflict_flagged
5. THE Loom_Engine SHALL include fact counts: extracted, canonical_predicate, custom_predicate
6. THE Loom_Engine SHALL include evidence counts: explicit, implied
7. THE Loom_Engine SHALL include processing_time_ms
8. THE Dashboard SHALL query extraction metrics by joining against the episode extraction_metrics column

### Requirement 49: Operational Dashboard

**User Story:** As a system operator, I want a React-based operational dashboard, so that I can monitor pipeline health, review conflicts, manage predicates, and analyze retrieval quality.

#### Acceptance Criteria

1. THE Dashboard SHALL display a pipeline health overview showing episode counts by source and namespace, entity counts by type, current vs superseded fact counts, queue depth, and model configuration
2. THE Dashboard SHALL provide a compilation trace viewer with paginated list of loom_think calls showing timestamp, query, namespace, classification, confidence, latency, and token count
3. THE Dashboard SHALL provide drill-down on each compilation showing candidates found, selected, and rejected with per-candidate score breakdown across relevance, recency, stability, and provenance dimensions
4. THE Dashboard SHALL provide a knowledge graph explorer with entity search, entity detail with properties and aliases, visual graph rendering of 1-2 hop neighborhoods, and fact detail with temporal range and supersession chain
5. THE Dashboard SHALL provide an entity conflict review queue showing unresolved conflicts with candidate matches and scores, with actions to merge, keep separate, or split
6. THE Dashboard SHALL provide a predicate candidate review view showing unmapped custom predicates with occurrence counts, example facts, and actions to map to existing canonical predicate or promote to canonical with target pack selection
7. THE Dashboard SHALL provide a predicate pack browser listing all packs with predicate counts, drill-down to pack predicates with categories and usage counts, per-namespace active packs, and a usage heatmap of used vs unused predicates
8. THE Dashboard SHALL provide retrieval quality metrics including precision over time, latency percentiles (p50, p95, p99), classification confidence distribution, and hot-tier utilization per namespace
9. THE Dashboard SHALL provide an extraction quality view with model comparison, entity resolution method distribution, custom predicate growth rate, and entity fragmentation detection results
10. THE Dashboard SHALL provide a benchmark comparison view with side-by-side A/B/C condition results

### Requirement 50: Dashboard API Endpoints

**User Story:** As a dashboard developer, I want read-only JSON API endpoints on the loom-engine, so that the React dashboard can fetch all operational data without a separate backend.

#### Acceptance Criteria

1. THE Loom_Engine SHALL serve dashboard API endpoints from the same axum router as MCP and REST endpoints
2. THE Loom_Engine SHALL expose read-only GET endpoints for pipeline health, episodes, entities, entity graph, facts, compilations, compilation detail, conflicts, predicate candidates, predicate packs, pack detail, pack predicates, active predicates per namespace, and all metrics
3. THE Loom_Engine SHALL expose POST endpoints for conflict resolution and predicate candidate resolution as the only write operations on the dashboard API
4. WHEN resolving a predicate candidate via the dashboard API, THE Loom_Engine SHALL accept an optional target_pack field for pack-aware promotion
5. THE Loom_Engine SHALL expose namespace listing endpoint for the Dashboard

### Requirement 51: Caddy Reverse Proxy Configuration

**User Story:** As a system operator, I want Caddy configured as a reverse proxy, so that TLS, static file serving, and request routing are handled outside the application.

#### Acceptance Criteria

1. THE Caddy_Proxy SHALL route requests matching /api/* to the Loom_Engine on port 8080
2. THE Caddy_Proxy SHALL route requests matching /mcp/* to the Loom_Engine on port 8080
3. THE Caddy_Proxy SHALL route requests matching /dashboard/api/* to the Loom_Engine on port 8080
4. THE Caddy_Proxy SHALL serve Dashboard static files for all other request paths
5. THE Caddy_Proxy SHALL provide automatic TLS certificate management

### Requirement 52: Ollama Local LLM Inference

**User Story:** As a system operator, I want all LLM inference running locally via Ollama, so that the system has zero cloud dependency for extraction, classification, and embedding.

#### Acceptance Criteria

1. THE Loom_Engine SHALL connect to Ollama_Service via HTTP using the reqwest crate
2. THE Loom_Engine SHALL use Gemma 4 26B MoE (gemma4:26b) for entity and fact extraction
3. THE Loom_Engine SHALL use Gemma 4 E4B (gemma4:e4b) for intent classification
4. THE Loom_Engine SHALL use nomic-embed-text for generating 768-dimension embeddings
5. THE Loom_Engine SHALL support Azure OpenAI as a fallback inference provider when Ollama is unavailable or when quality thresholds are not met with local models
6. THE Loom_Engine SHALL configure model endpoints via environment variables

### Requirement 53: Three-Mode Ingestion Taxonomy

**User Story:** As the primary user of Loom, I want every episode tagged with its provenance class at ingestion time, so that ranking and compilation can distinguish seed content from live capture from vendor imports without inference.

#### Acceptance Criteria

1. THE Loom_Engine SHALL enforce that every episode carries exactly one of three `ingestion_mode` values: `user_authored_seed`, `vendor_import`, or `live_mcp_capture`
2. THE Loom_Engine SHALL enforce the CHECK constraint on `ingestion_mode` at the database layer so invalid values cannot be inserted
3. THE Loom_Engine SHALL NOT accept any `llm_reconstruction` value or equivalent mode representing LLM-generated content; that mode is rejected architecturally
4. WHEN a request arrives via MCP, THE Loom_Engine SHALL hardcode `ingestion_mode = live_mcp_capture` regardless of what the client sent, preventing client-side forgery of the mode
5. WHEN a request arrives via REST `/api/learn`, THE Loom_Engine SHALL require `ingestion_mode` in the request body and return HTTP 400 when absent or invalid
6. WHEN `ingestion_mode = vendor_import`, THE Loom_Engine SHALL require non-empty `parser_version` and `parser_source_schema` fields; otherwise THE Loom_Engine SHALL reject both fields
7. THE Loom_Engine SHALL expose the three-mode taxonomy via a shared Rust enum so every module that reads or writes mode values uses identical names

### Requirement 54: Provenance Coefficient in Stage 5 Ranking

**User Story:** As the primary user of Loom, I want live-captured evidence to outrank seed content, and seed to outrank vendor imports, so that compiled context reflects the authority hierarchy.

#### Acceptance Criteria

1. THE Loom_Engine SHALL apply a provenance coefficient of 1.0 to candidates whose effective ingestion mode is `live_mcp_capture`
2. THE Loom_Engine SHALL apply a provenance coefficient of 0.8 to candidates whose effective ingestion mode is `user_authored_seed`
3. THE Loom_Engine SHALL apply a provenance coefficient of 0.6 to candidates whose effective ingestion mode is `vendor_import`
4. WHEN a candidate is a fact supported by multiple source episodes, THE Loom_Engine SHALL use the MAX provenance coefficient across all source episodes
5. THE Loom_Engine SHALL multiply the provenance dimension score by the coefficient before combining it with relevance, recency, and stability in the composite score
6. WHERE a candidate lacks mode metadata (synthetic or hot-tier items), THE Loom_Engine SHALL use a neutral coefficient of 1.0 so the intrinsic provenance score is returned unchanged

### Requirement 55: Sole-Source Flag in Stage 6 Compilation

**User Story:** As the primary user of Loom, I want compiled output to mark facts whose only provenance is seed content, so that I can decide whether to trust them or verify against source.

#### Acceptance Criteria

1. THE Loom_Engine SHALL emit a `sole_source` boolean on every compiled fact whose effective ingestion mode is known
2. THE Loom_Engine SHALL set `sole_source = true` when the effective ingestion mode is `user_authored_seed` (no live or vendor corroboration exists)
3. THE Loom_Engine SHALL set `sole_source = false` when the effective ingestion mode is `live_mcp_capture` or `vendor_import`
4. WHERE a fact lacks mode metadata (hot-tier items), THE Loom_Engine SHALL omit the `sole_source` attribute rather than defaulting to either boolean
5. THE Loom_Engine SHALL render `sole_source` as an XML attribute on `<fact>` elements in structured output
6. THE Loom_Engine SHALL render `sole_source` as a JSON field on fact objects in compact output
7. THE Loom_Engine SHALL render `mode="<ingestion_mode>"` as an XML attribute on `<episode>` elements in structured output when mode is known

### Requirement 56: Bootstrap Parser Schema Assertions and Degraded-Mode Contract

**User Story:** As the primary user of Loom, I want vendor-export parsers to fail loud on schema drift rather than silently ingest partial data, so that my authority model is not polluted by parsers pretending everything is fine.

#### Acceptance Criteria

1. WHEN a bootstrap parser processes a vendor export, THE parser SHALL assert a pinned schema naming each required field and its expected type
2. IF a required field is missing or wrongly typed, THE parser SHALL exit non-zero with an error naming the specific failing field and the schema version
3. THE parser SHALL NOT best-effort parse, silently skip, or otherwise degrade when the asserted schema does not match
4. THE parser SHALL set `ingestion_mode = vendor_import`, `parser_version = "<name>@<semver>"`, and `parser_source_schema = "<schema_identifier>"` on every episode it posts
5. WHEN a parser re-runs against an export it has partially ingested before, THE content_hash idempotency layer SHALL deduplicate without manual cleanup
6. THE parser SHALL NOT transform or summarize the source content; episode `content` SHALL be the verbatim excerpt from the export

### Requirement 57: Mode 1 User-Authored Seed CLI

**User Story:** As the primary user of Loom, I want a CLI tool that ingests markdown documents I have authored as Mode 1 seed episodes, so that I can establish foundational context for a namespace without depending on export quality.

#### Acceptance Criteria

1. THE CLI seed tool SHALL accept one or more file or directory paths and discover `*.md` files recursively
2. THE CLI seed tool SHALL POST each markdown document as a single episode to `/api/learn` with `ingestion_mode = user_authored_seed`
3. THE CLI seed tool SHALL NOT populate `parser_version` or `parser_source_schema` (those fields are reserved for Mode 2)
4. THE CLI seed tool SHALL treat the file content as verbatim and make no transformations to it
5. THE CLI seed tool SHALL read `LOOM_URL` and `LOOM_TOKEN` from the environment and exit non-zero if either is unset
6. THE CLI seed tool SHALL generate a stable `source_event_id` per file so re-runs are idempotent at the dedup layer

### Requirement 58: Parser Health and Ingestion Distribution Dashboard Views

**User Story:** As the primary user of Loom, I want the dashboard to surface which bootstrap parsers have run recently and which namespaces are seed-only, so that I can see at a glance where my authority model has gaps.

#### Acceptance Criteria

1. THE Dashboard API SHALL expose `GET /dashboard/api/metrics/parser-health` returning one row per `(parser_version, parser_source_schema)` pair with episode count and last-ingested timestamp
2. THE Dashboard API SHALL expose `GET /dashboard/api/metrics/ingestion-distribution` returning episode counts per `(namespace, ingestion_mode)` pair
3. THE ingestion-distribution endpoint SHALL separately return a list of namespaces whose episodes are 100% `user_authored_seed` so the dashboard can surface them as a warning
4. THE Dashboard SPA SHALL render a Parser Health page showing the per-parser rows and a note that failed parser runs do not appear here (they fail loud at the CLI and never write)
5. THE Dashboard SPA SHALL render an Ingestion Mode Distribution page showing the per-namespace breakdown and prominently surfacing the seed-only warning list

### Requirement 59: Exhaustive Live Capture and Per-Client Discipline Templates

**User Story:** As the primary user of Loom, I want Claude Code sessions captured in full, verbatim, without the model making judgment calls about what matters; and every other first-class client covered by a shipped discipline template that enforces the verbatim-content invariant on the selective-capture path.

#### Acceptance Criteria

1. THE Loom_Engine SHALL ship a `templates/loom-capture.sh` PostSession hook that reads raw session JSONL from Claude Code
2. THE hook SHALL POST the full transcript verbatim to `/api/learn` with `ingestion_mode = live_mcp_capture`
3. THE hook SHALL NOT summarize, filter, or transform the transcript content before posting
4. THE hook SHALL derive `source_event_id` from the session file path and modification time so re-runs are idempotent
5. THE Loom_Engine SHALL ship a discipline template for every first-class client, each instructing the model to call `loom_learn` only with verbatim content:
   - `templates/CLAUDE.md` — Claude Code project-scoped instructions
   - `templates/claude_desktop_projects_instructions.md` — Claude Desktop Projects
   - `templates/chatgpt_custom_instructions.md` — ChatGPT Custom Instructions / Projects (Developer Mode)
   - `templates/github_copilot_instructions.md` — `.github/copilot-instructions.md` for VS Code Copilot Agent mode
   - `templates/m365_copilot_instructions.md` — M365 Copilot declarative-agent `instructions` field
6. Every discipline template SHALL include a bolded directive against summarizing, paraphrasing, or reconstructing — the language is load-bearing and must be preserved across revisions

---

## Deferred Capabilities

The following capabilities are explicitly out of scope for the MVP and will be considered in future phases:

- Advanced procedural mining beyond basic pattern flagging
- Memify and semantic compaction of memory
- Broad connector network (7+ sources)
- Browser extensions for memory capture
- Git-style memory versioning and history
- Graph traversal beyond 2 hops
- Cold tier for archived memory
- Cross-namespace retrieval
- Fact-level embeddings (episode embeddings are sufficient initially)
- loom_inspect and loom_forget MCP tools
- Restorable compression in context output
- Advanced salience self-improvement algorithms
- Sleep-time memify scheduler
- ChatGPT / Copilot / Azure DevOps connectors
- Pack creation and namespace pack assignment via dashboard UI (handled via SQL or REST admin endpoint in MVP)