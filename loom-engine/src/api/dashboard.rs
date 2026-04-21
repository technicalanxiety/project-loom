//! Dashboard API read-only endpoints.
//!
//! Serves data for the React dashboard SPA under `/dashboard/api/`.
//! All endpoints are read-only GET handlers protected by bearer token middleware.
//! Response types are defined here alongside the handlers.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::mcp::AppState;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur in dashboard handlers.
#[derive(Debug, thiserror::Error)]
pub enum DashboardError {
    /// Resource not found.
    #[error("not found")]
    NotFound,
    /// An underlying database error.
    #[error("database error: {0}")]
    Database(String),
    /// Invalid request parameters.
    #[error("invalid request: {0}")]
    InvalidRequest(String),
}

impl IntoResponse for DashboardError {
    fn into_response(self) -> Response {
        let status = match &self {
            DashboardError::NotFound => StatusCode::NOT_FOUND,
            DashboardError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
            DashboardError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
        };
        let body = serde_json::json!({ "error": self.to_string() });
        (status, Json(body)).into_response()
    }
}

impl From<sqlx::Error> for DashboardError {
    fn from(e: sqlx::Error) -> Self {
        DashboardError::Database(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Pagination defaults
// ---------------------------------------------------------------------------

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;

fn clamp_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT).max(1)
}

fn default_offset(offset: Option<i64>) -> i64 {
    offset.unwrap_or(0).max(0)
}

// ---------------------------------------------------------------------------
// Query parameter structs
// ---------------------------------------------------------------------------

/// Pagination query parameters.
#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    /// Maximum number of results to return (default 50, max 200).
    pub limit: Option<i64>,
    /// Number of results to skip (default 0).
    pub offset: Option<i64>,
}

/// Optional namespace filter.
#[derive(Debug, Deserialize)]
pub struct NamespaceFilter {
    /// Filter results to this namespace.
    pub namespace: Option<String>,
}

/// Entity search query parameters.
#[derive(Debug, Deserialize)]
pub struct EntitySearchParams {
    /// Filter by namespace.
    pub namespace: Option<String>,
    /// Filter by entity type.
    pub entity_type: Option<String>,
    /// Name search (case-insensitive substring match).
    pub q: Option<String>,
    /// Maximum number of results (default 50, max 200).
    pub limit: Option<i64>,
    /// Number of results to skip (default 0).
    pub offset: Option<i64>,
}

/// Fact listing filter parameters.
#[derive(Debug, Deserialize)]
pub struct FactFilterParams {
    /// Filter by namespace.
    pub namespace: Option<String>,
    /// Filter by predicate.
    pub predicate: Option<String>,
    /// Filter by evidence status.
    pub evidence_status: Option<String>,
    /// Maximum number of results (default 50, max 200).
    pub limit: Option<i64>,
    /// Number of results to skip (default 0).
    pub offset: Option<i64>,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// A key-count pair used in aggregate breakdowns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CountByKey {
    /// The grouping key (e.g. source name, entity type).
    pub key: String,
    /// The count for this key.
    pub count: i64,
}

/// Pipeline health overview response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineHealthResponse {
    /// Episode counts grouped by source system.
    pub episodes_by_source: Vec<CountByKey>,
    /// Episode counts grouped by namespace.
    pub episodes_by_namespace: Vec<CountByKey>,
    /// Entity counts grouped by entity type.
    pub entities_by_type: Vec<CountByKey>,
    /// Number of currently valid facts.
    pub facts_current: i64,
    /// Number of superseded facts.
    pub facts_superseded: i64,
    /// Number of unprocessed episodes in the queue.
    pub queue_depth: i64,
    /// Most recently used extraction model.
    pub extraction_model: Option<String>,
    /// Most recently used classification model.
    pub classification_model: Option<String>,
}

/// Namespace configuration info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceInfo {
    /// Namespace identifier.
    pub namespace: String,
    /// Hot tier token budget.
    pub hot_tier_budget: i32,
    /// Warm tier token budget.
    pub warm_tier_budget: i32,
    /// Active predicate packs for this namespace.
    pub predicate_packs: Vec<String>,
    /// Human-readable description.
    pub description: Option<String>,
}

/// Summary of a compilation trace entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompilationSummary {
    /// Audit log entry identifier.
    pub id: Uuid,
    /// When this compilation occurred.
    pub created_at: DateTime<Utc>,
    /// Namespace that was queried.
    pub namespace: String,
    /// The original query text.
    pub query_text: Option<String>,
    /// Classified task class.
    pub task_class: String,
    /// Primary classification confidence.
    pub primary_confidence: Option<f64>,
    /// Retrieval profiles that ran.
    pub profiles_executed: Option<Vec<String>>,
    /// Total candidates found.
    pub candidates_found: Option<i32>,
    /// Candidates included in the package.
    pub candidates_selected: Option<i32>,
    /// Total tokens compiled.
    pub compiled_tokens: Option<i32>,
    /// End-to-end latency in milliseconds.
    pub latency_total_ms: Option<i32>,
}

/// Full detail of a single compilation trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompilationDetail {
    /// Audit log entry identifier.
    pub id: Uuid,
    /// When this compilation occurred.
    pub created_at: DateTime<Utc>,
    /// Namespace that was queried.
    pub namespace: String,
    /// The original query text.
    pub query_text: Option<String>,
    /// Classified task class.
    pub task_class: String,
    /// Primary classification confidence.
    pub primary_confidence: Option<f64>,
    /// Retrieval profiles that ran.
    pub profiles_executed: Option<Vec<String>>,
    /// Total candidates found.
    pub candidates_found: Option<i32>,
    /// Candidates included in the package.
    pub candidates_selected: Option<i32>,
    /// Total tokens compiled.
    pub compiled_tokens: Option<i32>,
    /// End-to-end latency in milliseconds.
    pub latency_total_ms: Option<i32>,
    /// Secondary task class.
    pub secondary_class: Option<String>,
    /// Secondary classification confidence.
    pub secondary_confidence: Option<f64>,
    /// Selected items with score breakdowns (JSONB).
    pub selected_items: Option<serde_json::Value>,
    /// Rejected items with rejection reasons (JSONB).
    pub rejected_items: Option<serde_json::Value>,
    /// Output format: "structured" or "compact".
    pub output_format: Option<String>,
    /// Optional user feedback score.
    pub user_rating: Option<f64>,
    /// Intent classification stage latency.
    pub latency_classify_ms: Option<i32>,
    /// Retrieval profile execution stage latency.
    pub latency_retrieve_ms: Option<i32>,
    /// Ranking stage latency.
    pub latency_rank_ms: Option<i32>,
    /// Package compilation stage latency.
    pub latency_compile_ms: Option<i32>,
}

/// Summary of an entity for list views.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitySummary {
    /// Entity identifier.
    pub id: Uuid,
    /// Entity name.
    pub name: String,
    /// Entity type.
    pub entity_type: String,
    /// Namespace.
    pub namespace: String,
    /// Known aliases for this entity.
    pub aliases: Vec<String>,
    /// Tier placement: "hot" or "warm".
    pub tier: Option<String>,
    /// Salience score for ranking.
    pub salience_score: Option<f64>,
}

/// Full entity detail including facts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityDetail {
    /// Entity identifier.
    pub id: Uuid,
    /// Entity name.
    pub name: String,
    /// Entity type.
    pub entity_type: String,
    /// Namespace.
    pub namespace: String,
    /// Known aliases for this entity.
    pub aliases: Vec<String>,
    /// Tier placement: "hot" or "warm".
    pub tier: Option<String>,
    /// Salience score for ranking.
    pub salience_score: Option<f64>,
    /// Full entity properties JSONB.
    pub properties: serde_json::Value,
    /// Source episode UUIDs.
    pub source_episodes: Option<Vec<Uuid>>,
    /// When the entity was created.
    pub created_at: DateTime<Utc>,
    /// Facts where this entity is subject or object.
    pub facts: Vec<FactSummary>,
}

/// Summary of a fact for list views.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactSummary {
    /// Fact identifier.
    pub id: Uuid,
    /// Subject entity name.
    pub subject_name: String,
    /// Predicate (relationship type).
    pub predicate: String,
    /// Object entity name.
    pub object_name: String,
    /// Namespace.
    pub namespace: String,
    /// Evidence status.
    pub evidence_status: String,
    /// When this fact became valid.
    pub valid_from: DateTime<Utc>,
    /// When this fact stopped being valid (None = currently valid).
    pub valid_until: Option<DateTime<Utc>>,
    /// Tier placement: "hot" or "warm".
    pub tier: Option<String>,
}

/// Summary of an unresolved entity conflict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictSummary {
    /// Conflict identifier.
    pub id: Uuid,
    /// The entity name that triggered the conflict.
    pub entity_name: String,
    /// The entity type.
    pub entity_type: String,
    /// Namespace.
    pub namespace: String,
    /// Candidate matches as JSONB.
    pub candidates: serde_json::Value,
    /// Whether this conflict has been resolved.
    pub resolved: bool,
    /// Resolution decision if resolved.
    pub resolution: Option<String>,
    /// When the conflict was created.
    pub created_at: DateTime<Utc>,
}

/// Summary of a predicate candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredicateCandidateSummary {
    /// Candidate identifier.
    pub id: Uuid,
    /// The custom predicate text.
    pub predicate: String,
    /// How many facts use this predicate.
    pub occurrences: i32,
    /// Sample fact IDs for review.
    pub example_facts: Option<Vec<Uuid>>,
    /// If mapped to an existing canonical predicate.
    pub mapped_to: Option<String>,
    /// Target pack when promoted.
    pub promoted_to_pack: Option<String>,
    /// When this candidate was created.
    pub created_at: DateTime<Utc>,
    /// When the operator resolved this candidate.
    pub resolved_at: Option<DateTime<Utc>>,
}

/// Summary of a predicate pack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackSummary {
    /// Pack name.
    pub pack: String,
    /// Human-readable description.
    pub description: Option<String>,
    /// Number of predicates in this pack.
    pub predicate_count: i64,
}

/// Full detail of a predicate pack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackDetail {
    /// Pack name.
    pub pack: String,
    /// Human-readable description.
    pub description: Option<String>,
    /// Predicates belonging to this pack.
    pub predicates: Vec<PredicateInfo>,
}

/// Information about a single predicate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredicateInfo {
    /// Predicate name.
    pub predicate: String,
    /// Category: structural, temporal, decisional, operational, or regulatory.
    pub category: String,
    /// Inverse predicate name.
    pub inverse: Option<String>,
    /// Human-readable description.
    pub description: Option<String>,
    /// Number of times used in facts.
    pub usage_count: i32,
}

/// Active predicates for a namespace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivePredicatesResponse {
    /// Namespace identifier.
    pub namespace: String,
    /// Active predicate packs for this namespace.
    pub packs: Vec<String>,
    /// All predicates from the active packs.
    pub predicates: Vec<PredicateInfo>,
}

/// A date-value metric data point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyMetric {
    /// Date in "YYYY-MM-DD" format.
    pub date: String,
    /// Metric value for this date.
    pub value: f64,
}

/// Retrieval quality metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalMetrics {
    /// Daily precision over the last 30 days.
    pub daily_precision: Vec<DailyMetric>,
    /// 50th percentile latency in milliseconds.
    pub latency_p50: Option<f64>,
    /// 95th percentile latency in milliseconds.
    pub latency_p95: Option<f64>,
    /// 99th percentile latency in milliseconds.
    pub latency_p99: Option<f64>,
}

/// Per-model extraction metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMetric {
    /// Model name.
    pub model: String,
    /// Number of episodes processed by this model.
    pub episode_count: i64,
    /// Average entity count per episode.
    pub avg_entity_count: Option<f64>,
    /// Average fact count per episode.
    pub avg_fact_count: Option<f64>,
}

/// Extraction pipeline metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionMetrics {
    /// Episode counts and averages grouped by extraction model.
    pub by_model: Vec<ModelMetric>,
    /// Entity resolution method distribution.
    pub resolution_distribution: Vec<CountByKey>,
    /// Daily growth of custom predicate candidates.
    pub custom_predicate_growth: Vec<DailyMetric>,
}

/// A confidence score bucket for distribution charts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidenceBucket {
    /// Bucket label e.g. "0.0-0.2".
    pub bucket: String,
    /// Number of compilations in this bucket.
    pub count: i64,
}

/// Classification confidence distribution metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassificationMetrics {
    /// Distribution of primary confidence scores across buckets.
    pub confidence_distribution: Vec<ConfidenceBucket>,
    /// Compilation counts grouped by primary task class.
    pub class_distribution: Vec<CountByKey>,
}

/// Hot-tier utilization metrics for a single namespace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotTierNamespaceMetric {
    /// Namespace identifier.
    pub namespace: String,
    /// Number of entities in the hot tier.
    pub hot_entity_count: i64,
    /// Number of facts in the hot tier.
    pub hot_fact_count: i64,
    /// Configured hot tier token budget.
    pub budget_tokens: i32,
    /// Estimated utilization percentage.
    pub utilization_pct: f64,
}

/// Hot-tier utilization metrics across all namespaces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotTierMetrics {
    /// Per-namespace hot tier utilization.
    pub by_namespace: Vec<HotTierNamespaceMetric>,
}

/// Graph traversal response for entity neighborhood.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphResponse {
    /// The root entity that was traversed from.
    pub root_entity_id: Uuid,
    /// Discovered entities in the neighborhood.
    pub nodes: Vec<GraphNode>,
    /// Connecting facts between nodes.
    pub edges: Vec<GraphEdge>,
}

/// A node in the entity graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    /// Entity identifier.
    pub entity_id: Uuid,
    /// Entity name.
    pub entity_name: String,
    /// Entity type.
    pub entity_type: String,
    /// How many hops from the root entity.
    pub hop_depth: i32,
}

/// An edge in the entity graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    /// Fact identifier.
    pub fact_id: Uuid,
    /// Predicate (relationship type).
    pub predicate: String,
    /// Evidence status of the connecting fact.
    pub evidence_status: String,
}

// ---------------------------------------------------------------------------
// Internal row types for sqlx queries
// ---------------------------------------------------------------------------

#[derive(Debug, sqlx::FromRow)]
struct CountByKeyRow {
    key: String,
    count: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct ModelRow {
    model: String,
    episode_count: i64,
    avg_entity_count: Option<f64>,
    avg_fact_count: Option<f64>,
}

#[derive(Debug, sqlx::FromRow)]
struct DailyMetricRow {
    date: Option<chrono::NaiveDate>,
    value: Option<f64>,
}

#[derive(Debug, sqlx::FromRow)]
struct LatencyPercentilesRow {
    p50: Option<f64>,
    p95: Option<f64>,
    p99: Option<f64>,
}

#[derive(Debug, sqlx::FromRow)]
struct NamespaceInfoRow {
    namespace: String,
    hot_tier_budget: Option<i32>,
    warm_tier_budget: Option<i32>,
    predicate_packs: Option<Vec<String>>,
    description: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct CompilationSummaryRow {
    id: Uuid,
    created_at: Option<DateTime<Utc>>,
    namespace: String,
    query_text: Option<String>,
    task_class: String,
    primary_confidence: Option<f64>,
    profiles_executed: Option<Vec<String>>,
    candidates_found: Option<i32>,
    candidates_selected: Option<i32>,
    compiled_tokens: Option<i32>,
    latency_total_ms: Option<i32>,
}

#[derive(Debug, sqlx::FromRow)]
struct CompilationDetailRow {
    id: Uuid,
    created_at: Option<DateTime<Utc>>,
    namespace: String,
    query_text: Option<String>,
    task_class: String,
    primary_confidence: Option<f64>,
    profiles_executed: Option<Vec<String>>,
    candidates_found: Option<i32>,
    candidates_selected: Option<i32>,
    compiled_tokens: Option<i32>,
    latency_total_ms: Option<i32>,
    secondary_class: Option<String>,
    secondary_confidence: Option<f64>,
    selected_items: Option<serde_json::Value>,
    rejected_items: Option<serde_json::Value>,
    output_format: Option<String>,
    user_rating: Option<f64>,
    latency_classify_ms: Option<i32>,
    latency_retrieve_ms: Option<i32>,
    latency_rank_ms: Option<i32>,
    latency_compile_ms: Option<i32>,
}

#[derive(Debug, sqlx::FromRow)]
struct EntitySummaryRow {
    id: Uuid,
    name: String,
    entity_type: String,
    namespace: String,
    properties: Option<serde_json::Value>,
    tier: Option<String>,
    salience_score: Option<f64>,
}

#[derive(Debug, sqlx::FromRow)]
struct EntityDetailRow {
    id: Uuid,
    name: String,
    entity_type: String,
    namespace: String,
    properties: Option<serde_json::Value>,
    tier: Option<String>,
    salience_score: Option<f64>,
    source_episodes: Option<Vec<Uuid>>,
    created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, sqlx::FromRow)]
struct FactSummaryRow {
    id: Uuid,
    subject_name: String,
    predicate: String,
    object_name: String,
    namespace: String,
    evidence_status: String,
    valid_from: DateTime<Utc>,
    valid_until: Option<DateTime<Utc>>,
    tier: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct ConflictRow {
    id: Uuid,
    entity_name: String,
    entity_type: String,
    namespace: String,
    candidates: serde_json::Value,
    resolved: Option<bool>,
    resolution: Option<String>,
    created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, sqlx::FromRow)]
struct PredicateCandidateRow {
    id: Uuid,
    predicate: String,
    occurrences: Option<i32>,
    example_facts: Option<Vec<Uuid>>,
    mapped_to: Option<String>,
    promoted_to_pack: Option<String>,
    created_at: Option<DateTime<Utc>>,
    resolved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, sqlx::FromRow)]
struct PackSummaryRow {
    pack: String,
    description: Option<String>,
    predicate_count: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct PredicateInfoRow {
    predicate: String,
    category: String,
    inverse: Option<String>,
    description: Option<String>,
    usage_count: Option<i32>,
}

#[derive(Debug, sqlx::FromRow)]
struct HotTierRow {
    namespace: String,
    hot_entity_count: i64,
    hot_fact_count: i64,
    budget_tokens: Option<i32>,
}

#[derive(Debug, sqlx::FromRow)]
struct ConfidenceBucketRow {
    bucket: String,
    count: i64,
}

// ---------------------------------------------------------------------------
// Helper: extract aliases from entity properties JSONB
// ---------------------------------------------------------------------------

fn extract_aliases(properties: &Option<serde_json::Value>) -> Vec<String> {
    properties
        .as_ref()
        .and_then(|p| p.get("aliases"))
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// GET /dashboard/api/health
// ---------------------------------------------------------------------------

/// Pipeline health overview: episode counts by source/namespace, entity counts
/// by type, fact counts, queue depth, and most recent model names.
pub async fn handle_dashboard_health(
    State(state): State<AppState>,
) -> Result<Json<PipelineHealthResponse>, DashboardError> {
    let pool = &state.pools.online;

    // Episodes by source
    let episodes_by_source: Vec<CountByKeyRow> = sqlx::query_as(
        "SELECT source AS key, COUNT(*) AS count FROM loom_episodes WHERE deleted_at IS NULL GROUP BY source ORDER BY count DESC",
    )
    .fetch_all(pool)
    .await?;

    // Episodes by namespace
    let episodes_by_namespace: Vec<CountByKeyRow> = sqlx::query_as(
        "SELECT namespace AS key, COUNT(*) AS count FROM loom_episodes WHERE deleted_at IS NULL GROUP BY namespace ORDER BY count DESC",
    )
    .fetch_all(pool)
    .await?;

    // Entities by type
    let entities_by_type: Vec<CountByKeyRow> = sqlx::query_as(
        "SELECT entity_type AS key, COUNT(*) AS count FROM loom_entities WHERE deleted_at IS NULL GROUP BY entity_type ORDER BY count DESC",
    )
    .fetch_all(pool)
    .await?;

    // Current facts (valid_until IS NULL)
    let facts_current: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM loom_facts WHERE valid_until IS NULL AND deleted_at IS NULL",
    )
    .fetch_one(pool)
    .await?;

    // Superseded facts (valid_until IS NOT NULL)
    let facts_superseded: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM loom_facts WHERE valid_until IS NOT NULL AND deleted_at IS NULL",
    )
    .fetch_one(pool)
    .await?;

    // Queue depth: unprocessed episodes
    let queue_depth: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM loom_episodes WHERE processed = false AND deleted_at IS NULL",
    )
    .fetch_one(pool)
    .await?;

    // Most recent extraction model
    let extraction_model: Option<String> = sqlx::query_scalar(
        "SELECT extraction_model FROM loom_episodes WHERE extraction_model IS NOT NULL ORDER BY ingested_at DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?
    .flatten();

    // Most recent classification model
    let classification_model: Option<String> = sqlx::query_scalar(
        "SELECT classification_model FROM loom_episodes WHERE classification_model IS NOT NULL ORDER BY ingested_at DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?
    .flatten();

    Ok(Json(PipelineHealthResponse {
        episodes_by_source: episodes_by_source
            .into_iter()
            .map(|r| CountByKey { key: r.key, count: r.count })
            .collect(),
        episodes_by_namespace: episodes_by_namespace
            .into_iter()
            .map(|r| CountByKey { key: r.key, count: r.count })
            .collect(),
        entities_by_type: entities_by_type
            .into_iter()
            .map(|r| CountByKey { key: r.key, count: r.count })
            .collect(),
        facts_current,
        facts_superseded,
        queue_depth,
        extraction_model,
        classification_model,
    }))
}

// ---------------------------------------------------------------------------
// GET /dashboard/api/namespaces
// ---------------------------------------------------------------------------

/// List all configured namespaces with their tier budgets and predicate packs.
pub async fn handle_namespaces(
    State(state): State<AppState>,
) -> Result<Json<Vec<NamespaceInfo>>, DashboardError> {
    let pool = &state.pools.online;

    let rows: Vec<NamespaceInfoRow> = sqlx::query_as(
        r#"
        SELECT namespace, hot_tier_budget, warm_tier_budget, predicate_packs, description
        FROM loom_namespace_config
        ORDER BY namespace
        "#,
    )
    .fetch_all(pool)
    .await?;

    let namespaces = rows
        .into_iter()
        .map(|r| NamespaceInfo {
            namespace: r.namespace,
            hot_tier_budget: r.hot_tier_budget.unwrap_or(500),
            warm_tier_budget: r.warm_tier_budget.unwrap_or(3000),
            predicate_packs: r.predicate_packs.unwrap_or_else(|| vec!["core".to_string()]),
            description: r.description,
        })
        .collect();

    Ok(Json(namespaces))
}

// ---------------------------------------------------------------------------
// GET /dashboard/api/compilations
// ---------------------------------------------------------------------------

/// Paginated list of compilation traces ordered by most recent first.
pub async fn handle_compilations(
    State(state): State<AppState>,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<CompilationSummary>>, DashboardError> {
    let pool = &state.pools.online;
    let limit = clamp_limit(params.limit);
    let offset = default_offset(params.offset);

    let rows: Vec<CompilationSummaryRow> = sqlx::query_as(
        r#"
        SELECT id, created_at, namespace, query_text, task_class, primary_confidence,
               profiles_executed, candidates_found, candidates_selected, compiled_tokens,
               latency_total_ms
        FROM loom_audit_log
        ORDER BY created_at DESC
        LIMIT $1 OFFSET $2
        "#,
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    let summaries = rows
        .into_iter()
        .map(|r| CompilationSummary {
            id: r.id,
            created_at: r.created_at.unwrap_or_else(Utc::now),
            namespace: r.namespace,
            query_text: r.query_text,
            task_class: r.task_class,
            primary_confidence: r.primary_confidence,
            profiles_executed: r.profiles_executed,
            candidates_found: r.candidates_found,
            candidates_selected: r.candidates_selected,
            compiled_tokens: r.compiled_tokens,
            latency_total_ms: r.latency_total_ms,
        })
        .collect();

    Ok(Json(summaries))
}

// ---------------------------------------------------------------------------
// GET /dashboard/api/compilations/:id
// ---------------------------------------------------------------------------

/// Full detail of a single compilation trace including score breakdowns.
pub async fn handle_compilation_detail(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<CompilationDetail>, DashboardError> {
    let pool = &state.pools.online;

    let row: Option<CompilationDetailRow> = sqlx::query_as(
        r#"
        SELECT id, created_at, namespace, query_text, task_class, primary_confidence,
               profiles_executed, candidates_found, candidates_selected, compiled_tokens,
               latency_total_ms, secondary_class, secondary_confidence,
               selected_items, rejected_items, output_format, user_rating,
               latency_classify_ms, latency_retrieve_ms, latency_rank_ms, latency_compile_ms
        FROM loom_audit_log
        WHERE id = $1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    let r = row.ok_or(DashboardError::NotFound)?;

    Ok(Json(CompilationDetail {
        id: r.id,
        created_at: r.created_at.unwrap_or_else(Utc::now),
        namespace: r.namespace,
        query_text: r.query_text,
        task_class: r.task_class,
        primary_confidence: r.primary_confidence,
        profiles_executed: r.profiles_executed,
        candidates_found: r.candidates_found,
        candidates_selected: r.candidates_selected,
        compiled_tokens: r.compiled_tokens,
        latency_total_ms: r.latency_total_ms,
        secondary_class: r.secondary_class,
        secondary_confidence: r.secondary_confidence,
        selected_items: r.selected_items,
        rejected_items: r.rejected_items,
        output_format: r.output_format,
        user_rating: r.user_rating,
        latency_classify_ms: r.latency_classify_ms,
        latency_retrieve_ms: r.latency_retrieve_ms,
        latency_rank_ms: r.latency_rank_ms,
        latency_compile_ms: r.latency_compile_ms,
    }))
}

// ---------------------------------------------------------------------------
// GET /dashboard/api/entities
// ---------------------------------------------------------------------------

/// Entity search with optional namespace, type, and name filters.
pub async fn handle_entities(
    State(state): State<AppState>,
    Query(params): Query<EntitySearchParams>,
) -> Result<Json<Vec<EntitySummary>>, DashboardError> {
    let pool = &state.pools.online;
    let limit = clamp_limit(params.limit);
    let offset = default_offset(params.offset);

    let rows: Vec<EntitySummaryRow> = sqlx::query_as(
        r#"
        SELECT e.id, e.name, e.entity_type, e.namespace, e.properties,
               es.tier, es.salience_score
        FROM loom_entities e
        LEFT JOIN loom_entity_state es ON es.entity_id = e.id
        WHERE e.deleted_at IS NULL
          AND ($1::text IS NULL OR e.namespace = $1)
          AND ($2::text IS NULL OR e.entity_type = $2)
          AND ($3::text IS NULL OR e.name ILIKE '%' || $3 || '%')
        ORDER BY e.name
        LIMIT $4 OFFSET $5
        "#,
    )
    .bind(&params.namespace)
    .bind(&params.entity_type)
    .bind(&params.q)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    let summaries = rows
        .into_iter()
        .map(|r| {
            let aliases = extract_aliases(&r.properties);
            EntitySummary {
                id: r.id,
                name: r.name,
                entity_type: r.entity_type,
                namespace: r.namespace,
                aliases,
                tier: r.tier,
                salience_score: r.salience_score,
            }
        })
        .collect();

    Ok(Json(summaries))
}

// ---------------------------------------------------------------------------
// GET /dashboard/api/entities/:id
// ---------------------------------------------------------------------------

/// Full entity detail including properties, aliases, and related facts.
pub async fn handle_entity_detail(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<EntityDetail>, DashboardError> {
    let pool = &state.pools.online;

    let row: Option<EntityDetailRow> = sqlx::query_as(
        r#"
        SELECT e.id, e.name, e.entity_type, e.namespace, e.properties,
               es.tier, es.salience_score, e.source_episodes, e.created_at
        FROM loom_entities e
        LEFT JOIN loom_entity_state es ON es.entity_id = e.id
        WHERE e.id = $1 AND e.deleted_at IS NULL
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    let entity = row.ok_or(DashboardError::NotFound)?;

    // Fetch facts where this entity is subject or object
    let fact_rows: Vec<FactSummaryRow> = sqlx::query_as(
        r#"
        SELECT f.id, e_s.name AS subject_name, f.predicate, e_o.name AS object_name,
               f.namespace, f.evidence_status, f.valid_from, f.valid_until,
               fs.tier
        FROM loom_facts f
        JOIN loom_entities e_s ON e_s.id = f.subject_id
        JOIN loom_entities e_o ON e_o.id = f.object_id
        LEFT JOIN loom_fact_state fs ON fs.fact_id = f.id
        WHERE f.deleted_at IS NULL
          AND (f.subject_id = $1 OR f.object_id = $1)
        ORDER BY f.valid_from DESC
        LIMIT 100
        "#,
    )
    .bind(id)
    .fetch_all(pool)
    .await?;

    let facts = fact_rows
        .into_iter()
        .map(|r| FactSummary {
            id: r.id,
            subject_name: r.subject_name,
            predicate: r.predicate,
            object_name: r.object_name,
            namespace: r.namespace,
            evidence_status: r.evidence_status,
            valid_from: r.valid_from,
            valid_until: r.valid_until,
            tier: r.tier,
        })
        .collect();

    let aliases = extract_aliases(&entity.properties);
    let properties = entity.properties.unwrap_or(serde_json::Value::Object(Default::default()));

    Ok(Json(EntityDetail {
        id: entity.id,
        name: entity.name,
        entity_type: entity.entity_type,
        namespace: entity.namespace,
        aliases,
        tier: entity.tier,
        salience_score: entity.salience_score,
        properties,
        source_episodes: entity.source_episodes,
        created_at: entity.created_at.unwrap_or_else(Utc::now),
        facts,
    }))
}

// ---------------------------------------------------------------------------
// GET /dashboard/api/entities/:id/graph
// ---------------------------------------------------------------------------

/// 1-2 hop neighborhood graph for an entity via loom_traverse.
pub async fn handle_entity_graph(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(params): Query<NamespaceFilter>,
) -> Result<Json<GraphResponse>, DashboardError> {
    let pool = &state.pools.online;

    // Resolve namespace: use query param or fall back to entity's namespace
    let namespace = match params.namespace {
        Some(ns) => ns,
        None => {
            let ns: Option<String> = sqlx::query_scalar(
                "SELECT namespace FROM loom_entities WHERE id = $1 AND deleted_at IS NULL",
            )
            .bind(id)
            .fetch_optional(pool)
            .await?
            .flatten();
            ns.ok_or(DashboardError::NotFound)?
        }
    };

    let rows = crate::db::traverse::traverse(pool, id, 2, &namespace)
        .await
        .map_err(|e| DashboardError::Database(e.to_string()))?;

    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    for row in rows {
        nodes.push(GraphNode {
            entity_id: row.entity_id,
            entity_name: row.entity_name,
            entity_type: row.entity_type,
            hop_depth: row.hop_depth,
        });
        if let (Some(fact_id), Some(predicate), Some(evidence_status)) =
            (row.fact_id, row.predicate, row.evidence_status)
        {
            edges.push(GraphEdge {
                fact_id,
                predicate,
                evidence_status,
            });
        }
    }

    // Deduplicate edges by fact_id
    edges.sort_by_key(|e| e.fact_id);
    edges.dedup_by_key(|e| e.fact_id);

    Ok(Json(GraphResponse {
        root_entity_id: id,
        nodes,
        edges,
    }))
}

// ---------------------------------------------------------------------------
// GET /dashboard/api/facts
// ---------------------------------------------------------------------------

/// Fact listing with optional namespace, predicate, and evidence status filters.
pub async fn handle_facts(
    State(state): State<AppState>,
    Query(params): Query<FactFilterParams>,
) -> Result<Json<Vec<FactSummary>>, DashboardError> {
    let pool = &state.pools.online;
    let limit = clamp_limit(params.limit);
    let offset = default_offset(params.offset);

    let rows: Vec<FactSummaryRow> = sqlx::query_as(
        r#"
        SELECT f.id, e_s.name AS subject_name, f.predicate, e_o.name AS object_name,
               f.namespace, f.evidence_status, f.valid_from, f.valid_until,
               fs.tier
        FROM loom_facts f
        JOIN loom_entities e_s ON e_s.id = f.subject_id
        JOIN loom_entities e_o ON e_o.id = f.object_id
        LEFT JOIN loom_fact_state fs ON fs.fact_id = f.id
        WHERE f.deleted_at IS NULL
          AND ($1::text IS NULL OR f.namespace = $1)
          AND ($2::text IS NULL OR f.predicate = $2)
          AND ($3::text IS NULL OR f.evidence_status = $3)
        ORDER BY f.valid_from DESC
        LIMIT $4 OFFSET $5
        "#,
    )
    .bind(&params.namespace)
    .bind(&params.predicate)
    .bind(&params.evidence_status)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    let facts = rows
        .into_iter()
        .map(|r| FactSummary {
            id: r.id,
            subject_name: r.subject_name,
            predicate: r.predicate,
            object_name: r.object_name,
            namespace: r.namespace,
            evidence_status: r.evidence_status,
            valid_from: r.valid_from,
            valid_until: r.valid_until,
            tier: r.tier,
        })
        .collect();

    Ok(Json(facts))
}

// ---------------------------------------------------------------------------
// GET /dashboard/api/conflicts
// ---------------------------------------------------------------------------

/// Unresolved entity resolution conflicts.
pub async fn handle_conflicts(
    State(state): State<AppState>,
) -> Result<Json<Vec<ConflictSummary>>, DashboardError> {
    let pool = &state.pools.online;

    let rows: Vec<ConflictRow> = sqlx::query_as(
        r#"
        SELECT id, entity_name, entity_type, namespace, candidates,
               resolved, resolution, created_at
        FROM loom_resolution_conflicts
        WHERE resolved = false OR resolved IS NULL
        ORDER BY created_at DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    let conflicts = rows
        .into_iter()
        .map(|r| ConflictSummary {
            id: r.id,
            entity_name: r.entity_name,
            entity_type: r.entity_type,
            namespace: r.namespace,
            candidates: r.candidates,
            resolved: r.resolved.unwrap_or(false),
            resolution: r.resolution,
            created_at: r.created_at.unwrap_or_else(Utc::now),
        })
        .collect();

    Ok(Json(conflicts))
}

// ---------------------------------------------------------------------------
// GET /dashboard/api/predicates/candidates
// ---------------------------------------------------------------------------

/// Custom predicate candidates with occurrence counts.
pub async fn handle_predicate_candidates(
    State(state): State<AppState>,
) -> Result<Json<Vec<PredicateCandidateSummary>>, DashboardError> {
    let pool = &state.pools.online;

    let rows: Vec<PredicateCandidateRow> = sqlx::query_as(
        r#"
        SELECT id, predicate, occurrences, example_facts, mapped_to,
               promoted_to_pack, created_at, resolved_at
        FROM loom_predicate_candidates
        ORDER BY occurrences DESC, created_at DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    let candidates = rows
        .into_iter()
        .map(|r| PredicateCandidateSummary {
            id: r.id,
            predicate: r.predicate,
            occurrences: r.occurrences.unwrap_or(0),
            example_facts: r.example_facts,
            mapped_to: r.mapped_to,
            promoted_to_pack: r.promoted_to_pack,
            created_at: r.created_at.unwrap_or_else(Utc::now),
            resolved_at: r.resolved_at,
        })
        .collect();

    Ok(Json(candidates))
}

// ---------------------------------------------------------------------------
// GET /dashboard/api/predicates/packs
// ---------------------------------------------------------------------------

/// All predicate packs with predicate counts.
pub async fn handle_predicate_packs(
    State(state): State<AppState>,
) -> Result<Json<Vec<PackSummary>>, DashboardError> {
    let pool = &state.pools.online;

    let rows: Vec<PackSummaryRow> = sqlx::query_as(
        r#"
        SELECT pp.pack, pp.description,
               COUNT(p.predicate) AS predicate_count
        FROM loom_predicate_packs pp
        LEFT JOIN loom_predicates p ON p.pack = pp.pack
        GROUP BY pp.pack, pp.description
        ORDER BY pp.pack
        "#,
    )
    .fetch_all(pool)
    .await?;

    let packs = rows
        .into_iter()
        .map(|r| PackSummary {
            pack: r.pack,
            description: r.description,
            predicate_count: r.predicate_count,
        })
        .collect();

    Ok(Json(packs))
}

// ---------------------------------------------------------------------------
// GET /dashboard/api/predicates/packs/:pack
// ---------------------------------------------------------------------------

/// Pack detail with all predicates, categories, and usage counts.
pub async fn handle_pack_detail(
    State(state): State<AppState>,
    Path(pack): Path<String>,
) -> Result<Json<PackDetail>, DashboardError> {
    let pool = &state.pools.online;

    // Fetch pack metadata
    let pack_row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT pack, description FROM loom_predicate_packs WHERE pack = $1",
    )
    .bind(&pack)
    .fetch_optional(pool)
    .await?;

    let (pack_name, description) = pack_row.ok_or(DashboardError::NotFound)?;

    // Fetch predicates for this pack
    let predicate_rows: Vec<PredicateInfoRow> = sqlx::query_as(
        r#"
        SELECT predicate, category, inverse, description, usage_count
        FROM loom_predicates
        WHERE pack = $1
        ORDER BY category, predicate
        "#,
    )
    .bind(&pack_name)
    .fetch_all(pool)
    .await?;

    let predicates = predicate_rows
        .into_iter()
        .map(|r| PredicateInfo {
            predicate: r.predicate,
            category: r.category,
            inverse: r.inverse,
            description: r.description,
            usage_count: r.usage_count.unwrap_or(0),
        })
        .collect();

    Ok(Json(PackDetail {
        pack: pack_name,
        description,
        predicates,
    }))
}

// ---------------------------------------------------------------------------
// GET /dashboard/api/predicates/active/:namespace
// ---------------------------------------------------------------------------

/// Active predicates for a namespace based on its configured predicate packs.
pub async fn handle_active_predicates(
    State(state): State<AppState>,
    Path(namespace): Path<String>,
) -> Result<Json<ActivePredicatesResponse>, DashboardError> {
    let pool = &state.pools.online;

    // Fetch the namespace's active packs
    let packs: Option<Vec<String>> = sqlx::query_scalar(
        "SELECT predicate_packs FROM loom_namespace_config WHERE namespace = $1",
    )
    .bind(&namespace)
    .fetch_optional(pool)
    .await?
    .flatten();

    let active_packs = packs.unwrap_or_else(|| vec!["core".to_string()]);

    // Fetch all predicates from the active packs
    let predicate_rows: Vec<PredicateInfoRow> = sqlx::query_as(
        r#"
        SELECT predicate, category, inverse, description, usage_count
        FROM loom_predicates
        WHERE pack = ANY($1)
        ORDER BY category, predicate
        "#,
    )
    .bind(&active_packs)
    .fetch_all(pool)
    .await?;

    let predicates = predicate_rows
        .into_iter()
        .map(|r| PredicateInfo {
            predicate: r.predicate,
            category: r.category,
            inverse: r.inverse,
            description: r.description,
            usage_count: r.usage_count.unwrap_or(0),
        })
        .collect();

    Ok(Json(ActivePredicatesResponse {
        namespace,
        packs: active_packs,
        predicates,
    }))
}

// ---------------------------------------------------------------------------
// GET /dashboard/api/metrics/retrieval
// ---------------------------------------------------------------------------

/// Retrieval quality metrics: daily precision over 30 days and latency percentiles.
pub async fn handle_metrics_retrieval(
    State(state): State<AppState>,
) -> Result<Json<RetrievalMetrics>, DashboardError> {
    let pool = &state.pools.online;

    // Daily precision: avg(selected/found) per day over last 30 days
    let precision_rows: Vec<DailyMetricRow> = sqlx::query_as(
        r#"
        SELECT DATE(created_at) AS date,
               AVG(CASE WHEN candidates_found > 0
                   THEN candidates_selected::float / candidates_found::float
                   ELSE 0 END) AS value
        FROM loom_audit_log
        WHERE created_at > NOW() - INTERVAL '30 days'
        GROUP BY DATE(created_at)
        ORDER BY DATE(created_at)
        "#,
    )
    .fetch_all(pool)
    .await?;

    // Latency percentiles
    let latency_row: Option<LatencyPercentilesRow> = sqlx::query_as(
        r#"
        SELECT
          PERCENTILE_CONT(0.50) WITHIN GROUP (ORDER BY latency_total_ms) AS p50,
          PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY latency_total_ms) AS p95,
          PERCENTILE_CONT(0.99) WITHIN GROUP (ORDER BY latency_total_ms) AS p99
        FROM loom_audit_log
        WHERE latency_total_ms IS NOT NULL
        "#,
    )
    .fetch_optional(pool)
    .await?;

    let daily_precision = precision_rows
        .into_iter()
        .filter_map(|r| {
            Some(DailyMetric {
                date: r.date?.to_string(),
                value: r.value.unwrap_or(0.0),
            })
        })
        .collect();

    let (latency_p50, latency_p95, latency_p99) = latency_row
        .map(|r| (r.p50, r.p95, r.p99))
        .unwrap_or((None, None, None));

    Ok(Json(RetrievalMetrics {
        daily_precision,
        latency_p50,
        latency_p95,
        latency_p99,
    }))
}

// ---------------------------------------------------------------------------
// GET /dashboard/api/metrics/extraction
// ---------------------------------------------------------------------------

/// Extraction pipeline metrics: model comparison, resolution distribution,
/// and custom predicate growth.
pub async fn handle_metrics_extraction(
    State(state): State<AppState>,
) -> Result<Json<ExtractionMetrics>, DashboardError> {
    let pool = &state.pools.online;

    // Per-model episode counts and averages from extraction_metrics JSONB
    let model_rows: Vec<ModelRow> = sqlx::query_as(
        r#"
        SELECT
          COALESCE(extraction_model, 'unknown') AS model,
          COUNT(*) AS episode_count,
          AVG((extraction_metrics->>'extracted')::float) AS avg_entity_count,
          AVG((extraction_metrics->>'facts_extracted')::float) AS avg_fact_count
        FROM loom_episodes
        WHERE deleted_at IS NULL AND processed = true
        GROUP BY extraction_model
        ORDER BY episode_count DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    // Resolution method distribution from extraction_metrics JSONB
    // Each processed episode stores resolved_exact, resolved_alias,
    // resolved_semantic, and new counts in extraction_metrics.
    let resolution_rows: Vec<CountByKeyRow> = sqlx::query_as(
        r#"
        SELECT key, SUM(cnt)::bigint AS count FROM (
          SELECT 'exact' AS key,
                 COALESCE((extraction_metrics->>'resolved_exact')::bigint, 0) AS cnt
          FROM loom_episodes WHERE processed = true AND deleted_at IS NULL
          UNION ALL
          SELECT 'alias' AS key,
                 COALESCE((extraction_metrics->>'resolved_alias')::bigint, 0) AS cnt
          FROM loom_episodes WHERE processed = true AND deleted_at IS NULL
          UNION ALL
          SELECT 'semantic' AS key,
                 COALESCE((extraction_metrics->>'resolved_semantic')::bigint, 0) AS cnt
          FROM loom_episodes WHERE processed = true AND deleted_at IS NULL
          UNION ALL
          SELECT 'new' AS key,
                 COALESCE((extraction_metrics->>'new')::bigint, 0) AS cnt
          FROM loom_episodes WHERE processed = true AND deleted_at IS NULL
        ) sub
        GROUP BY key
        ORDER BY count DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    // Custom predicate candidate growth over last 30 days
    let growth_rows: Vec<DailyMetricRow> = sqlx::query_as(
        r#"
        SELECT DATE(created_at) AS date,
               COUNT(*)::float AS value
        FROM loom_predicate_candidates
        WHERE created_at > NOW() - INTERVAL '30 days'
        GROUP BY DATE(created_at)
        ORDER BY DATE(created_at)
        "#,
    )
    .fetch_all(pool)
    .await?;

    let by_model = model_rows
        .into_iter()
        .map(|r| ModelMetric {
            model: r.model,
            episode_count: r.episode_count,
            avg_entity_count: r.avg_entity_count,
            avg_fact_count: r.avg_fact_count,
        })
        .collect();

    let resolution_distribution = resolution_rows
        .into_iter()
        .map(|r| CountByKey { key: r.key, count: r.count })
        .collect();

    let custom_predicate_growth = growth_rows
        .into_iter()
        .filter_map(|r| {
            Some(DailyMetric {
                date: r.date?.to_string(),
                value: r.value.unwrap_or(0.0),
            })
        })
        .collect();

    Ok(Json(ExtractionMetrics {
        by_model,
        resolution_distribution,
        custom_predicate_growth,
    }))
}

// ---------------------------------------------------------------------------
// GET /dashboard/api/metrics/classification
// ---------------------------------------------------------------------------

/// Classification confidence distribution and class distribution.
pub async fn handle_metrics_classification(
    State(state): State<AppState>,
) -> Result<Json<ClassificationMetrics>, DashboardError> {
    let pool = &state.pools.online;

    // Confidence distribution in 0.2-wide buckets
    let bucket_rows: Vec<ConfidenceBucketRow> = sqlx::query_as(
        r#"
        SELECT
          CASE
            WHEN primary_confidence < 0.2 THEN '0.0-0.2'
            WHEN primary_confidence < 0.4 THEN '0.2-0.4'
            WHEN primary_confidence < 0.6 THEN '0.4-0.6'
            WHEN primary_confidence < 0.8 THEN '0.6-0.8'
            ELSE '0.8-1.0'
          END AS bucket,
          COUNT(*) AS count
        FROM loom_audit_log
        WHERE primary_confidence IS NOT NULL
        GROUP BY 1
        ORDER BY 1
        "#,
    )
    .fetch_all(pool)
    .await?;

    // Class distribution
    let class_rows: Vec<CountByKeyRow> = sqlx::query_as(
        r#"
        SELECT primary_class AS key, COUNT(*) AS count
        FROM loom_audit_log
        GROUP BY primary_class
        ORDER BY count DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    let confidence_distribution = bucket_rows
        .into_iter()
        .map(|r| ConfidenceBucket {
            bucket: r.bucket,
            count: r.count,
        })
        .collect();

    let class_distribution = class_rows
        .into_iter()
        .map(|r| CountByKey { key: r.key, count: r.count })
        .collect();

    Ok(Json(ClassificationMetrics {
        confidence_distribution,
        class_distribution,
    }))
}

// ---------------------------------------------------------------------------
// GET /dashboard/api/metrics/hot-tier
// ---------------------------------------------------------------------------

/// Hot-tier utilization per namespace.
pub async fn handle_metrics_hot_tier(
    State(state): State<AppState>,
) -> Result<Json<HotTierMetrics>, DashboardError> {
    let pool = &state.pools.online;

    let rows: Vec<HotTierRow> = sqlx::query_as(
        r#"
        WITH hot_entities AS (
          SELECT e.namespace, COUNT(*) AS cnt
          FROM loom_entities e
          JOIN loom_entity_state es ON es.entity_id = e.id
          WHERE e.deleted_at IS NULL AND es.tier = 'hot'
          GROUP BY e.namespace
        ),
        hot_facts AS (
          SELECT f.namespace, COUNT(*) AS cnt
          FROM loom_facts f
          JOIN loom_fact_state fs ON fs.fact_id = f.id
          WHERE f.deleted_at IS NULL AND f.valid_until IS NULL AND fs.tier = 'hot'
          GROUP BY f.namespace
        ),
        namespaces AS (
          SELECT DISTINCT namespace FROM loom_entities WHERE deleted_at IS NULL
          UNION
          SELECT DISTINCT namespace FROM loom_facts WHERE deleted_at IS NULL
        )
        SELECT
          n.namespace,
          COALESCE(he.cnt, 0) AS hot_entity_count,
          COALESCE(hf.cnt, 0) AS hot_fact_count,
          COALESCE(nc.hot_tier_budget, 500) AS budget_tokens
        FROM namespaces n
        LEFT JOIN hot_entities he ON he.namespace = n.namespace
        LEFT JOIN hot_facts hf ON hf.namespace = n.namespace
        LEFT JOIN loom_namespace_config nc ON nc.namespace = n.namespace
        ORDER BY n.namespace
        "#,
    )
    .fetch_all(pool)
    .await?;

    let by_namespace = rows
        .into_iter()
        .map(|r| {
            let budget = r.budget_tokens.unwrap_or(500);
            // Estimate utilization: assume ~50 tokens per hot entity on average
            let estimated_tokens = r.hot_entity_count * 50 + r.hot_fact_count * 30;
            let utilization_pct = if budget > 0 {
                (estimated_tokens as f64 / budget as f64 * 100.0).min(100.0)
            } else {
                0.0
            };
            HotTierNamespaceMetric {
                namespace: r.namespace,
                hot_entity_count: r.hot_entity_count,
                hot_fact_count: r.hot_fact_count,
                budget_tokens: budget,
                utilization_pct,
            }
        })
        .collect();

    Ok(Json(HotTierMetrics { by_namespace }))
}

// ---------------------------------------------------------------------------
// POST /dashboard/api/conflicts/:id/resolve
// ---------------------------------------------------------------------------

/// Request body for resolving an entity conflict.
#[derive(Debug, Deserialize)]
pub struct ResolveConflictRequest {
    /// Resolution decision: "merged", "kept_separate", or "split".
    pub resolution: String,
    /// For "merged" resolution: the entity UUID to merge into.
    pub merged_into: Option<uuid::Uuid>,
}

/// Response after resolving an entity conflict.
#[derive(Debug, Serialize)]
pub struct ResolveConflictResponse {
    /// Conflict identifier.
    pub id: uuid::Uuid,
    /// Whether the conflict is now resolved.
    pub resolved: bool,
    /// The recorded resolution string.
    pub resolution: String,
    /// When the resolution was recorded.
    pub resolved_at: chrono::DateTime<chrono::Utc>,
}

/// Internal row returned by the conflict resolution UPDATE … RETURNING.
#[derive(Debug, sqlx::FromRow)]
struct ResolveConflictRow {
    id: uuid::Uuid,
    resolved: Option<bool>,
    resolution: Option<String>,
    resolved_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Resolve an entity conflict by recording a merge, keep-separate, or split decision.
///
/// Validates the resolution type, builds the resolution string, and persists
/// the decision with a timestamp. Returns 400 for invalid input, 404 if the
/// conflict ID does not exist.
pub async fn handle_resolve_conflict(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<ResolveConflictRequest>,
) -> Result<Json<ResolveConflictResponse>, DashboardError> {
    // Validate resolution value.
    let resolution_str = match req.resolution.as_str() {
        "merged" => {
            let merged_into = req.merged_into.ok_or_else(|| {
                DashboardError::InvalidRequest(
                    "merged_into is required when resolution is \"merged\"".into(),
                )
            })?;
            format!("merged:{merged_into}")
        }
        "kept_separate" => "kept_separate".to_string(),
        "split" => "split".to_string(),
        other => {
            return Err(DashboardError::InvalidRequest(format!(
                "invalid resolution \"{other}\": must be one of \"merged\", \"kept_separate\", \"split\""
            )));
        }
    };

    let pool = &state.pools.online;

    let row: Option<ResolveConflictRow> = sqlx::query_as(
        r#"
        UPDATE loom_resolution_conflicts
        SET resolved    = true,
            resolution  = $2,
            resolved_at = NOW()
        WHERE id = $1
        RETURNING id, resolved, resolution, resolved_at
        "#,
    )
    .bind(id)
    .bind(&resolution_str)
    .fetch_optional(pool)
    .await?;

    let r = row.ok_or(DashboardError::NotFound)?;

    tracing::info!(
        conflict_id = %id,
        resolution = %resolution_str,
        "dashboard: conflict resolved"
    );

    Ok(Json(ResolveConflictResponse {
        id: r.id,
        resolved: r.resolved.unwrap_or(true),
        resolution: r.resolution.unwrap_or(resolution_str),
        resolved_at: r.resolved_at.unwrap_or_else(Utc::now),
    }))
}

// ---------------------------------------------------------------------------
// POST /dashboard/api/predicates/candidates/:id/resolve
// ---------------------------------------------------------------------------

/// Request body for resolving a predicate candidate.
#[derive(Debug, Deserialize)]
pub struct ResolvePredicateCandidateRequest {
    /// "map" to map to an existing canonical predicate, or "promote" to create a new canonical.
    pub action: String,
    /// For action="map": the canonical predicate to map to.
    pub mapped_to: Option<String>,
    /// For action="promote": the target predicate pack (required).
    pub target_pack: Option<String>,
    /// For action="promote": the category for the new canonical predicate (defaults to "structural").
    pub category: Option<String>,
    /// For action="promote": optional human-readable description.
    pub description: Option<String>,
    /// For action="promote": optional inverse predicate name.
    pub inverse: Option<String>,
}

/// Response after resolving a predicate candidate.
#[derive(Debug, Serialize)]
pub struct ResolvePredicateCandidateResponse {
    /// Candidate identifier.
    pub id: uuid::Uuid,
    /// The custom predicate text.
    pub predicate: String,
    /// The action taken: "map" or "promote".
    pub action: String,
    /// Canonical predicate this was mapped to (if action="map").
    pub mapped_to: Option<String>,
    /// Pack the predicate was promoted into (if action="promote").
    pub promoted_to_pack: Option<String>,
    /// When the resolution was recorded.
    pub resolved_at: chrono::DateTime<chrono::Utc>,
}

/// Internal row for the candidate lookup.
#[derive(Debug, sqlx::FromRow)]
struct CandidateLookupRow {
    id: uuid::Uuid,
    predicate: String,
}

/// Valid predicate categories.
const VALID_CATEGORIES: &[&str] = &[
    "structural",
    "temporal",
    "decisional",
    "operational",
    "regulatory",
];

/// Resolve a predicate candidate by mapping it to an existing canonical predicate
/// or promoting it to canonical status in a chosen predicate pack.
///
/// For "map": validates the target canonical predicate exists, then records the
/// mapping. For "promote": validates the target pack exists, inserts the new
/// canonical predicate (idempotent), and records the promotion. Returns 400 for
/// invalid input, 404 if the candidate or referenced resources do not exist.
pub async fn handle_resolve_predicate_candidate(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Json(req): Json<ResolvePredicateCandidateRequest>,
) -> Result<Json<ResolvePredicateCandidateResponse>, DashboardError> {
    // Validate action.
    if req.action != "map" && req.action != "promote" {
        return Err(DashboardError::InvalidRequest(format!(
            "invalid action \"{}\": must be \"map\" or \"promote\"",
            req.action
        )));
    }

    let pool = &state.pools.online;

    // Fetch the candidate to get its predicate text.
    let candidate: Option<CandidateLookupRow> = sqlx::query_as(
        "SELECT id, predicate FROM loom_predicate_candidates WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    let candidate = candidate.ok_or(DashboardError::NotFound)?;

    let resolved_at = Utc::now();

    match req.action.as_str() {
        "map" => {
            let mapped_to = req.mapped_to.as_deref().ok_or_else(|| {
                DashboardError::InvalidRequest(
                    "mapped_to is required when action is \"map\"".into(),
                )
            })?;

            // Validate the target canonical predicate exists.
            let exists: Option<(String,)> = sqlx::query_as(
                "SELECT predicate FROM loom_predicates WHERE predicate = $1",
            )
            .bind(mapped_to)
            .fetch_optional(pool)
            .await?;

            if exists.is_none() {
                return Err(DashboardError::InvalidRequest(format!(
                    "canonical predicate \"{mapped_to}\" does not exist in loom_predicates"
                )));
            }

            // Update candidate.
            sqlx::query(
                r#"
                UPDATE loom_predicate_candidates
                SET mapped_to = $2, resolved_at = $3
                WHERE id = $1
                "#,
            )
            .bind(id)
            .bind(mapped_to)
            .bind(resolved_at)
            .execute(pool)
            .await?;

            tracing::info!(
                candidate_id = %id,
                predicate = %candidate.predicate,
                mapped_to,
                "dashboard: predicate candidate mapped"
            );

            Ok(Json(ResolvePredicateCandidateResponse {
                id: candidate.id,
                predicate: candidate.predicate,
                action: "map".to_string(),
                mapped_to: Some(mapped_to.to_string()),
                promoted_to_pack: None,
                resolved_at,
            }))
        }

        "promote" => {
            let target_pack = req.target_pack.as_deref().ok_or_else(|| {
                DashboardError::InvalidRequest(
                    "target_pack is required when action is \"promote\"".into(),
                )
            })?;

            // Validate target_pack exists.
            let pack_exists: Option<(String,)> = sqlx::query_as(
                "SELECT pack FROM loom_predicate_packs WHERE pack = $1",
            )
            .bind(target_pack)
            .fetch_optional(pool)
            .await?;

            if pack_exists.is_none() {
                return Err(DashboardError::InvalidRequest(format!(
                    "predicate pack \"{target_pack}\" does not exist in loom_predicate_packs"
                )));
            }

            // Validate category if provided.
            let category = req.category.as_deref().unwrap_or("structural");
            if !VALID_CATEGORIES.contains(&category) {
                return Err(DashboardError::InvalidRequest(format!(
                    "invalid category \"{category}\": must be one of {}",
                    VALID_CATEGORIES.join(", ")
                )));
            }

            // Insert new canonical predicate (idempotent).
            sqlx::query(
                r#"
                INSERT INTO loom_predicates (predicate, category, pack, inverse, description)
                VALUES ($1, $2, $3, $4, $5)
                ON CONFLICT (predicate) DO NOTHING
                "#,
            )
            .bind(&candidate.predicate)
            .bind(category)
            .bind(target_pack)
            .bind(req.inverse.as_deref())
            .bind(req.description.as_deref())
            .execute(pool)
            .await?;

            // Update candidate with promoted_to_pack.
            sqlx::query(
                r#"
                UPDATE loom_predicate_candidates
                SET promoted_to_pack = $2, resolved_at = $3
                WHERE id = $1
                "#,
            )
            .bind(id)
            .bind(target_pack)
            .bind(resolved_at)
            .execute(pool)
            .await?;

            tracing::info!(
                candidate_id = %id,
                predicate = %candidate.predicate,
                target_pack,
                category,
                "dashboard: predicate candidate promoted"
            );

            Ok(Json(ResolvePredicateCandidateResponse {
                id: candidate.id,
                predicate: candidate.predicate,
                action: "promote".to_string(),
                mapped_to: None,
                promoted_to_pack: Some(target_pack.to_string()),
                resolved_at,
            }))
        }

        // Unreachable: validated above.
        _ => unreachable!("action already validated"),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
// Benchmark endpoints
// ---------------------------------------------------------------------------

/// List all benchmark runs ordered by most recent first.
pub async fn handle_benchmark_runs(
    State(state): State<AppState>,
) -> Result<Json<Vec<crate::types::benchmark::BenchmarkRunSummary>>, DashboardError> {
    let pool = &state.pools.online;
    let runs = crate::pipeline::benchmark::list_benchmark_runs(pool)
        .await
        .map_err(|e| DashboardError::Database(e.to_string()))?;
    Ok(Json(runs))
}

/// Get full benchmark comparison detail for a specific run.
pub async fn handle_benchmark_detail(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<crate::types::benchmark::BenchmarkComparison>, DashboardError> {
    let pool = &state.pools.online;
    let comparison = crate::pipeline::benchmark::get_benchmark_detail(pool, id)
        .await
        .map_err(|e| match e {
            crate::pipeline::benchmark::BenchmarkError::NotFound(_) => DashboardError::NotFound,
            crate::pipeline::benchmark::BenchmarkError::Database(e) => {
                DashboardError::Database(e.to_string())
            }
        })?;
    Ok(Json(comparison))
}

/// Trigger a new benchmark run. Executes all 10+ tasks across A/B/C conditions.
pub async fn handle_run_benchmark(
    State(state): State<AppState>,
) -> Result<Json<crate::types::benchmark::BenchmarkRunSummary>, DashboardError> {
    let pool = &state.pools.offline;
    let run = crate::pipeline::benchmark::execute_benchmark(pool)
        .await
        .map_err(|e| DashboardError::Database(e.to_string()))?;
    tracing::info!(run_id = %run.id, name = %run.name, "benchmark run completed");
    Ok(Json(run))
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_limit_defaults_to_50() {
        assert_eq!(clamp_limit(None), 50);
    }

    #[test]
    fn clamp_limit_respects_max() {
        assert_eq!(clamp_limit(Some(500)), 200);
    }

    #[test]
    fn clamp_limit_uses_provided_value() {
        assert_eq!(clamp_limit(Some(25)), 25);
    }

    #[test]
    fn clamp_limit_minimum_is_one() {
        assert_eq!(clamp_limit(Some(0)), 1);
        assert_eq!(clamp_limit(Some(-5)), 1);
    }

    #[test]
    fn default_offset_returns_zero_for_none() {
        assert_eq!(default_offset(None), 0);
    }

    #[test]
    fn default_offset_clamps_negative() {
        assert_eq!(default_offset(Some(-10)), 0);
    }

    #[test]
    fn default_offset_uses_provided_value() {
        assert_eq!(default_offset(Some(100)), 100);
    }

    #[test]
    fn extract_aliases_from_properties() {
        let props = serde_json::json!({ "aliases": ["Rust", "rust-lang"] });
        let aliases = extract_aliases(&Some(props));
        assert_eq!(aliases, vec!["Rust", "rust-lang"]);
    }

    #[test]
    fn extract_aliases_returns_empty_when_no_aliases_key() {
        let props = serde_json::json!({ "other": "value" });
        let aliases = extract_aliases(&Some(props));
        assert!(aliases.is_empty());
    }

    #[test]
    fn extract_aliases_returns_empty_for_none_properties() {
        let aliases = extract_aliases(&None);
        assert!(aliases.is_empty());
    }

    #[test]
    fn dashboard_error_not_found_is_404() {
        let err = DashboardError::NotFound;
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn dashboard_error_database_is_500() {
        let err = DashboardError::Database("connection refused".into());
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn dashboard_error_invalid_request_is_400() {
        let err = DashboardError::InvalidRequest("bad param".into());
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn dashboard_error_display_messages() {
        assert_eq!(DashboardError::NotFound.to_string(), "not found");
        assert!(DashboardError::Database("x".into()).to_string().contains("database error"));
        assert!(DashboardError::InvalidRequest("y".into()).to_string().contains("invalid request"));
    }

    // -- ResolveConflictRequest validation ----------------------------------

    #[test]
    fn valid_categories_contains_all_five() {
        assert_eq!(VALID_CATEGORIES.len(), 5);
        assert!(VALID_CATEGORIES.contains(&"structural"));
        assert!(VALID_CATEGORIES.contains(&"temporal"));
        assert!(VALID_CATEGORIES.contains(&"decisional"));
        assert!(VALID_CATEGORIES.contains(&"operational"));
        assert!(VALID_CATEGORIES.contains(&"regulatory"));
    }

    #[test]
    fn resolve_conflict_response_serializes() {
        let resp = ResolveConflictResponse {
            id: uuid::Uuid::nil(),
            resolved: true,
            resolution: "kept_separate".to_string(),
            resolved_at: chrono::Utc::now(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("kept_separate"));
        assert!(json.contains("resolved"));
    }

    #[test]
    fn resolve_predicate_candidate_response_serializes() {
        let resp = ResolvePredicateCandidateResponse {
            id: uuid::Uuid::nil(),
            predicate: "custom_rel".to_string(),
            action: "map".to_string(),
            mapped_to: Some("knows".to_string()),
            promoted_to_pack: None,
            resolved_at: chrono::Utc::now(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("custom_rel"));
        assert!(json.contains("knows"));
    }

    #[test]
    fn resolve_conflict_request_deserializes_merged() {
        let json = r#"{"resolution":"merged","merged_into":"00000000-0000-0000-0000-000000000001"}"#;
        let req: ResolveConflictRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.resolution, "merged");
        assert!(req.merged_into.is_some());
    }

    #[test]
    fn resolve_conflict_request_deserializes_kept_separate() {
        let json = r#"{"resolution":"kept_separate"}"#;
        let req: ResolveConflictRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.resolution, "kept_separate");
        assert!(req.merged_into.is_none());
    }

    #[test]
    fn resolve_predicate_candidate_request_deserializes_promote() {
        let json = r#"{"action":"promote","target_pack":"core","category":"temporal"}"#;
        let req: ResolvePredicateCandidateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.action, "promote");
        assert_eq!(req.target_pack.as_deref(), Some("core"));
        assert_eq!(req.category.as_deref(), Some("temporal"));
    }
}
