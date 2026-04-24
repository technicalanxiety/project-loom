/**
 * TypeScript types mirroring loom-engine Rust dashboard API response types.
 *
 * These interfaces match the serialized JSON output from the
 * `/dashboard/api/*` endpoints defined in `loom-engine/src/api/dashboard.rs`.
 */

// ---------------------------------------------------------------------------
// Base domain types (kept for backward compatibility)
// ---------------------------------------------------------------------------

/** An ingested episode record. */
export interface Episode {
  id: string;
  source: string;
  content: string;
  content_hash: string;
  occurred_at: string;
  namespace: string;
  processed: boolean;
}

/** A knowledge-graph entity. */
export interface Entity {
  id: string;
  name: string;
  entity_type: string;
  namespace: string;
  properties: Record<string, unknown>;
}

/** A knowledge-graph fact (relationship between two entities). */
export interface Fact {
  id: string;
  subject_id: string;
  predicate: string;
  object_id: string;
  namespace: string;
  valid_from: string;
  valid_until: string | null;
  evidence_status: string;
}

/** A compilation audit-log entry. */
export interface AuditLogEntry {
  id: string;
  created_at: string;
  task_class: string;
  namespace: string;
  query_text: string;
  compiled_tokens: number;
  latency_total_ms: number;
}

/** An unresolved entity-resolution conflict. */
export interface ResolutionConflict {
  id: string;
  entity_name: string;
  entity_type: string;
  namespace: string;
  candidates: unknown;
  resolved: boolean;
}

/** A custom predicate candidate awaiting operator review. */
export interface PredicateCandidate {
  id: string;
  predicate: string;
  occurrences: number;
  mapped_to: string | null;
}

// ---------------------------------------------------------------------------
// Dashboard API response types
// ---------------------------------------------------------------------------

/** A key-count pair used in aggregate breakdowns. */
export interface CountByKey {
  key: string;
  count: number;
}

/** Pipeline health overview response. */
export interface PipelineHealthResponse {
  episodes_by_source: CountByKey[];
  episodes_by_namespace: CountByKey[];
  entities_by_type: CountByKey[];
  facts_current: number;
  facts_superseded: number;
  queue_depth: number;
  extraction_model: string | null;
  classification_model: string | null;
}

/** Namespace configuration info. */
export interface NamespaceInfo {
  namespace: string;
  hot_tier_budget: number;
  warm_tier_budget: number;
  predicate_packs: string[];
  description: string | null;
}

/** Summary of a compilation trace entry. */
export interface CompilationSummary {
  id: string;
  created_at: string;
  namespace: string;
  query_text: string | null;
  task_class: string;
  primary_confidence: number | null;
  profiles_executed: string[] | null;
  candidates_found: number | null;
  candidates_selected: number | null;
  compiled_tokens: number | null;
  latency_total_ms: number | null;
}

/** Full detail of a single compilation trace. */
export interface CompilationDetail extends CompilationSummary {
  secondary_class: string | null;
  secondary_confidence: number | null;
  selected_items: unknown;
  rejected_items: unknown;
  output_format: string | null;
  user_rating: number | null;
  latency_classify_ms: number | null;
  latency_retrieve_ms: number | null;
  latency_rank_ms: number | null;
  latency_compile_ms: number | null;
}

/** Summary of an entity for list views. */
export interface EntitySummary {
  id: string;
  name: string;
  entity_type: string;
  namespace: string;
  aliases: string[];
  tier: string | null;
  salience_score: number | null;
}

/** Full entity detail including facts. */
export interface EntityDetail extends EntitySummary {
  properties: Record<string, unknown>;
  source_episodes: string[] | null;
  created_at: string;
  facts: FactSummary[];
}

/** Summary of a fact for list views. */
export interface FactSummary {
  id: string;
  subject_name: string;
  predicate: string;
  object_name: string;
  namespace: string;
  evidence_status: string;
  valid_from: string;
  valid_until: string | null;
  tier: string | null;
}

/** Summary of an unresolved entity conflict. */
export interface ConflictSummary {
  id: string;
  entity_name: string;
  entity_type: string;
  namespace: string;
  candidates: unknown;
  resolved: boolean;
  resolution: string | null;
  created_at: string;
}

/** Summary of a predicate candidate. */
export interface PredicateCandidateSummary {
  id: string;
  predicate: string;
  occurrences: number;
  example_facts: string[] | null;
  mapped_to: string | null;
  promoted_to_pack: string | null;
  created_at: string;
  resolved_at: string | null;
}

/** Summary of a predicate pack. */
export interface PackSummary {
  pack: string;
  description: string | null;
  predicate_count: number;
}

/** Full detail of a predicate pack. */
export interface PackDetail {
  pack: string;
  description: string | null;
  predicates: PredicateInfo[];
}

/** Information about a single predicate. */
export interface PredicateInfo {
  predicate: string;
  category: string;
  inverse: string | null;
  description: string | null;
  usage_count: number;
}

/** Active predicates for a namespace. */
export interface ActivePredicatesResponse {
  namespace: string;
  packs: string[];
  predicates: PredicateInfo[];
}

// ---------------------------------------------------------------------------
// Metrics types
// ---------------------------------------------------------------------------

/** A date-value metric data point. */
export interface DailyMetric {
  date: string;
  value: number;
}

/** Retrieval quality metrics. */
export interface RetrievalMetrics {
  daily_precision: DailyMetric[];
  latency_p50: number | null;
  latency_p95: number | null;
  latency_p99: number | null;
}

/** Per-model extraction metrics. */
export interface ModelMetric {
  model: string;
  episode_count: number;
  avg_entity_count: number | null;
  avg_fact_count: number | null;
}

/** Extraction pipeline metrics. */
export interface ExtractionMetrics {
  by_model: ModelMetric[];
  resolution_distribution: CountByKey[];
  custom_predicate_growth: DailyMetric[];
}

/** A confidence score bucket for distribution charts. */
export interface ConfidenceBucket {
  bucket: string;
  count: number;
}

/** Classification confidence distribution metrics. */
export interface ClassificationMetrics {
  confidence_distribution: ConfidenceBucket[];
  class_distribution: CountByKey[];
}

/** Hot-tier utilization metrics for a single namespace. */
export interface HotTierNamespaceMetric {
  namespace: string;
  hot_entity_count: number;
  hot_fact_count: number;
  budget_tokens: number;
  utilization_pct: number;
}

/** Hot-tier utilization metrics across all namespaces. */
export interface HotTierMetrics {
  by_namespace: HotTierNamespaceMetric[];
}

// ---------------------------------------------------------------------------
// Ingestion-mode observability types (ADR 004)
// ---------------------------------------------------------------------------

/** One row in the parser-health view: how many episodes a given
 *  (parser_version, parser_source_schema) pair has produced, and when. */
export interface ParserHealthRow {
  parser_version: string;
  parser_source_schema: string;
  episode_count: number;
  last_ingested_at: string | null;
}

/** Parser-health response payload. */
export interface ParserHealthMetrics {
  parsers: ParserHealthRow[];
}

/** One cell of the ingestion-mode distribution grid. */
export interface IngestionDistributionRow {
  namespace: string;
  ingestion_mode: 'user_authored_seed' | 'vendor_import' | 'live_mcp_capture';
  episode_count: number;
}

/** A namespace whose episodes are 100% user_authored_seed — every compiled
 *  fact will carry sole_source=true. */
export interface SeedOnlyNamespace {
  namespace: string;
  seed_episode_count: number;
}

/** Ingestion-distribution response payload. */
export interface IngestionDistributionMetrics {
  rows: IngestionDistributionRow[];
  seed_only_namespaces: SeedOnlyNamespace[];
}

// ---------------------------------------------------------------------------
// Graph types
// ---------------------------------------------------------------------------

/** Graph traversal response for entity neighborhood. */
export interface GraphResponse {
  root_entity_id: string;
  nodes: GraphNode[];
  edges: GraphEdge[];
}

/** A node in the entity graph. */
export interface GraphNode {
  entity_id: string;
  entity_name: string;
  entity_type: string;
  hop_depth: number;
}

/** An edge in the entity graph. */
export interface GraphEdge {
  fact_id: string;
  predicate: string;
  evidence_status: string;
}

// ---------------------------------------------------------------------------
// Request / response types for mutations
// ---------------------------------------------------------------------------

/** Request body for resolving an entity conflict. */
export interface ResolveConflictRequest {
  resolution: string;
  merged_into?: string;
}

/** Response after resolving an entity conflict. */
export interface ResolveConflictResponse {
  id: string;
  resolved: boolean;
  resolution: string;
  resolved_at: string;
}

/** Request body for resolving a predicate candidate. */
export interface ResolvePredicateCandidateRequest {
  action: string;
  mapped_to?: string;
  target_pack?: string;
  category?: string;
  description?: string;
  inverse?: string;
}

/** Response after resolving a predicate candidate. */
export interface ResolvePredicateCandidateResponse {
  id: string;
  predicate: string;
  action: string;
  mapped_to: string | null;
  promoted_to_pack: string | null;
  resolved_at: string;
}

// ---------------------------------------------------------------------------
// Benchmark types
// ---------------------------------------------------------------------------

/** A benchmark evaluation run. */
export interface BenchmarkRun {
  /** Run identifier. */
  id: string;
  /** Human-readable run name. */
  name: string;
  /** When the run was created. */
  created_at: string;
  /** Current status: pending, running, completed, failed. */
  status: string;
}

/** A single benchmark task result for one condition. */
export interface BenchmarkTaskResult {
  /** Result identifier. */
  id: string;
  /** Name of the benchmark task. */
  task_name: string;
  /** Condition tested: A, B, or C. */
  condition: string;
  /** Precision: relevant retrieved / total retrieved. */
  precision: number;
  /** Number of tokens in the compiled context. */
  token_count: number;
  /** Whether the task was considered successful. */
  task_success: boolean;
  /** End-to-end latency in milliseconds. */
  latency_ms: number;
}

/** Aggregated metrics for a single benchmark condition. */
export interface ConditionSummary {
  /** Average precision across all tasks. */
  avg_precision: number;
  /** Average token count across all tasks. */
  avg_token_count: number;
  /** Fraction of tasks that succeeded. */
  success_rate: number;
  /** Average latency in milliseconds. */
  avg_latency_ms: number;
}

/** Full benchmark comparison for the dashboard view. */
export interface BenchmarkComparison {
  /** The benchmark run metadata. */
  run: BenchmarkRun;
  /** All individual task results. */
  results: BenchmarkTaskResult[];
  /** Per-condition aggregated summaries. */
  summary: {
    condition_a: ConditionSummary;
    condition_b: ConditionSummary;
    condition_c: ConditionSummary;
  };
}

// ---------------------------------------------------------------------------
// Streaming telemetry (/dashboard/api/stream/telemetry)
// ---------------------------------------------------------------------------

/** A single (timestamp, value) sample for sparkline rendering. */
export interface DataPoint {
  /** Unix milliseconds. */
  ts: number;
  v: number;
}

/** A recent extraction failure surfaced on the Runtime page. */
export interface ExtractionError {
  episode_id: string;
  source: string;
  error: string;
  /** Unix milliseconds. */
  occurred_at: number;
}

/** Snapshot pushed over SSE once per second. */
export interface TelemetrySnapshot {
  ts: number;
  // Host
  cpu_pct: number;
  mem_used_mib: number;
  mem_total_mib: number;
  // Ollama
  ollama_model: string | null;
  ollama_on_gpu: boolean;
  ollama_vram_mib: number | null;
  // Pipeline stage latencies (p50, ms)
  latency_classify_p50_ms: number | null;
  latency_retrieve_p50_ms: number | null;
  latency_rank_p50_ms: number | null;
  latency_compile_p50_ms: number | null;
  latency_total_p50_ms: number | null;
  // Live counters
  active_ingestions: number;
  queue_depth: number;
  failed_episodes: number;
  // Sparklines (5-min rings, pushed every 5 s)
  sparkline_latency: DataPoint[];
  sparkline_ingestion_rate: DataPoint[];
  sparkline_compilation_rate: DataPoint[];
  // Error tail
  recent_errors: ExtractionError[];
}
