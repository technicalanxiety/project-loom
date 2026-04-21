//! MCP (Model Context Protocol) endpoint handler.
//!
//! Exposes three tools for AI assistant integration:
//!
//! - **loom_learn**: Ingest an episode. Returns immediately with status
//!   "queued" or "duplicate". Extraction runs asynchronously via the
//!   offline pipeline.
//! - **loom_think**: Compile a context package. Runs the full online
//!   pipeline (classify → retrieve → weight → rank → compile) and returns
//!   the assembled context with token count and compilation ID.
//! - **loom_recall**: Direct fact lookup for named entities. Bypasses
//!   classification and retrieval profiles.
//!
//! All endpoints require a valid bearer token (enforced by the auth
//! middleware applied at the router level).

use std::str::FromStr;
use std::time::Instant;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::AppConfig;
use crate::db::audit;
use crate::db::episodes::{self, NewEpisode};
use crate::db::facts;
use crate::db::pool::DbPools;
use crate::llm::client::LlmClient;
use crate::llm::embeddings;
use crate::pipeline::online::{
    classify::{self, ClassifyStageOutput},
    compile::{self, CompilationInput, HotTierItem},
    rank,
    retrieve::{self, RetrievalResult},
    weight,
};
use crate::types::classification::TaskClass;
use crate::types::compilation::OutputFormat;
use crate::types::mcp::{
    LearnRequest, LearnResponse, RecallRequest, RecallResponse, ThinkRequest, ThinkResponse,
};

// ---------------------------------------------------------------------------
// Shared application state
// ---------------------------------------------------------------------------

/// Application state shared across all handlers via axum `State`.
#[derive(Clone)]
pub struct AppState {
    /// Online + offline database connection pools.
    pub pools: DbPools,
    /// LLM HTTP client (Ollama + Azure OpenAI fallback).
    pub llm_client: LlmClient,
    /// Application configuration.
    pub config: AppConfig,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur in MCP handlers.
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("database error: {0}")]
    Database(String),

    #[error("classification error: {0}")]
    Classification(String),

    #[error("retrieval error: {0}")]
    Retrieval(String),

    #[error("embedding error: {0}")]
    Embedding(String),

    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("service unavailable: {0}")]
    ServiceUnavailable(String),
}

impl IntoResponse for McpError {
    fn into_response(self) -> Response {
        let status = match &self {
            McpError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            McpError::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = serde_json::json!({ "error": self.to_string() });
        (status, Json(body)).into_response()
    }
}

// ---------------------------------------------------------------------------
// SHA-256 content hash
// ---------------------------------------------------------------------------

/// Compute a hex-encoded SHA-256 hash of the given content string.
///
/// Used for episode deduplication — two episodes with identical content
/// produce the same hash regardless of source metadata.
pub fn compute_content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

// ---------------------------------------------------------------------------
// 15.1 — loom_learn
// ---------------------------------------------------------------------------

/// Handle a `loom_learn` request.
///
/// Stores the episode immediately and returns. Extraction runs asynchronously
/// via the offline pipeline (tokio spawned task in the background worker).
///
/// # Idempotency
///
/// Duplicate detection uses two checks in order:
/// 1. `(source, source_event_id)` unique constraint — catches re-deliveries
///    from the same source system.
/// 2. `content_hash` — catches identical content submitted under different
///    event IDs.
///
/// Both checks return the existing episode ID with status `"duplicate"`.
#[tracing::instrument(skip(state, req), fields(endpoint = "loom_learn"))]
pub async fn handle_loom_learn(
    State(state): State<AppState>,
    Json(req): Json<LearnRequest>,
) -> Result<Json<LearnResponse>, McpError> {
    // Validate required fields.
    if req.content.trim().is_empty() {
        return Err(McpError::InvalidRequest("content must not be empty".into()));
    }
    if req.source.trim().is_empty() {
        return Err(McpError::InvalidRequest("source must not be empty".into()));
    }
    if req.namespace.trim().is_empty() {
        return Err(McpError::InvalidRequest("namespace must not be empty".into()));
    }

    let content_hash = compute_content_hash(&req.content);
    let occurred_at = req.occurred_at.unwrap_or_else(chrono::Utc::now);

    // Check for duplicate by content_hash before attempting insert.
    let existing_by_hash: Option<crate::types::episode::Episode> = sqlx::query_as(
        "SELECT * FROM loom_episodes WHERE content_hash = $1 AND namespace = $2 LIMIT 1",
    )
    .bind(&content_hash)
    .bind(&req.namespace)
    .fetch_optional(&state.pools.offline)
    .await
    .map_err(|e| McpError::Database(e.to_string()))?;

    if let Some(existing) = existing_by_hash {
        tracing::info!(
            episode_id = %existing.id,
            namespace = %req.namespace,
            "loom_learn: duplicate episode (content_hash match)"
        );
        return Ok(Json(LearnResponse {
            episode_id: existing.id,
            status: "duplicate".to_string(),
        }));
    }

    // Insert episode (insert_episode handles (source, source_event_id) dedup).
    let new_ep = NewEpisode {
        source: req.source.clone(),
        source_id: None,
        source_event_id: req.source_event_id.clone(),
        content: req.content.clone(),
        content_hash: content_hash.clone(),
        occurred_at,
        namespace: req.namespace.clone(),
        metadata: req.metadata.clone(),
        participants: req.participants.clone(),
    };

    let episode = episodes::insert_episode(&state.pools.offline, &new_ep)
        .await
        .map_err(|e| McpError::Database(e.to_string()))?;

    // Determine status: if insert_episode returned an existing row (idempotent
    // on source+source_event_id), the episode was already ingested.
    let status = if episode.processed.unwrap_or(false) {
        "duplicate"
    } else {
        "queued"
    };

    tracing::info!(
        episode_id = %episode.id,
        namespace = %req.namespace,
        source = %req.source,
        status,
        "loom_learn: episode ingested"
    );

    Ok(Json(LearnResponse {
        episode_id: episode.id,
        status: status.to_string(),
    }))
}

// ---------------------------------------------------------------------------
// 15.2 — loom_think
// ---------------------------------------------------------------------------

/// Handle a `loom_think` request.
///
/// Runs the full online pipeline:
/// 1. Classify intent (or apply override).
/// 2. Map to retrieval profiles.
/// 3. Generate query embedding.
/// 4. Execute profiles in parallel via `tokio::join!`.
/// 5. Apply memory weight modifiers.
/// 6. Rank on four dimensions.
/// 7. Compile context package (structured XML or compact JSON).
/// 8. Write audit log entry.
/// 9. Return context package with token count and compilation ID.
#[tracing::instrument(skip(state, req), fields(endpoint = "loom_think"))]
pub async fn handle_loom_think(
    State(state): State<AppState>,
    Json(req): Json<ThinkRequest>,
) -> Result<Json<ThinkResponse>, McpError> {
    let total_start = Instant::now();

    if req.query.trim().is_empty() {
        return Err(McpError::InvalidRequest("query must not be empty".into()));
    }
    if req.namespace.trim().is_empty() {
        return Err(McpError::InvalidRequest("namespace must not be empty".into()));
    }

    let target_model = req
        .target_model
        .clone()
        .unwrap_or_else(|| "claude".to_string());

    // Determine output format from target model name.
    let format = if target_model.to_lowercase().contains("claude") {
        OutputFormat::Structured
    } else {
        OutputFormat::Compact
    };

    // -----------------------------------------------------------------------
    // Stage 1: Intent classification
    // -----------------------------------------------------------------------
    let classify_start = Instant::now();

    let classify_output: ClassifyStageOutput = if let Some(ref override_str) = req.task_class_override {
        match TaskClass::from_str(override_str) {
            Ok(tc) => classify::apply_override(tc),
            Err(_) => {
                return Err(McpError::InvalidRequest(format!(
                    "unknown task_class_override: {override_str}"
                )));
            }
        }
    } else {
        match classify::classify_query(&state.llm_client, &state.config.llm, &req.query).await {
            Ok(output) => output,
            Err(e) => {
                // Classification failure: default to TaskClass::Chat.
                tracing::warn!(
                    error = %e,
                    query = %req.query,
                    "classification failed, defaulting to TaskClass::Chat"
                );
                classify::apply_override(TaskClass::Chat)
            }
        }
    };

    let latency_classify_ms = classify_start.elapsed().as_millis() as i32;
    let task_class = &classify_output.result.primary_class;
    let secondary_class = classify_output.result.secondary_class.as_ref();

    // -----------------------------------------------------------------------
    // Stage 2: Resolve retrieval profiles
    // -----------------------------------------------------------------------
    let profiles = retrieve::merge_profiles(task_class, secondary_class);
    let profile_names = retrieve::profile_names(&profiles);

    // -----------------------------------------------------------------------
    // Stage 3: Generate query embedding
    // -----------------------------------------------------------------------
    let query_embedding_vec = embeddings::generate_embedding(
        &state.llm_client,
        &state.config.llm,
        &req.query,
    )
    .await
    .map_err(|e| McpError::Embedding(e.to_string()))?;

    let query_embedding = pgvector::Vector::from(query_embedding_vec);

    // Tokenize query for entity name matching / boosting.
    let query_terms: Vec<String> = req
        .query
        .split_whitespace()
        .filter(|t| t.len() >= 3)
        .map(|t| t.to_lowercase())
        .collect();

    // -----------------------------------------------------------------------
    // Stage 4: Execute retrieval profiles in parallel
    // -----------------------------------------------------------------------
    let retrieve_start = Instant::now();

    let retrieval_result: RetrievalResult = retrieve::execute_profiles(
        &state.pools.online,
        &profiles,
        &query_embedding,
        &req.namespace,
        &query_terms,
        task_class,
    )
    .await
    .map_err(|e| McpError::Retrieval(e.to_string()))?;

    let latency_retrieve_ms = retrieve_start.elapsed().as_millis() as i32;

    // -----------------------------------------------------------------------
    // Stage 5: Apply memory weight modifiers
    // -----------------------------------------------------------------------
    let rank_start = Instant::now();

    let weighted = weight::apply_weights(retrieval_result.candidates, task_class);

    // -----------------------------------------------------------------------
    // Stage 6: Four-dimension ranking
    // -----------------------------------------------------------------------
    let ranked = rank::rank_candidates(weighted);
    let latency_rank_ms = rank_start.elapsed().as_millis() as i32;

    // -----------------------------------------------------------------------
    // Stage 7: Compile context package
    // -----------------------------------------------------------------------
    let compile_start = Instant::now();

    // Fetch hot tier items for this namespace.
    let hot_tier_items = fetch_hot_tier_items(&state.pools.online, &req.namespace)
        .await
        .unwrap_or_default();

    // Fetch namespace warm tier budget (default 3000).
    let warm_tier_budget = fetch_warm_tier_budget(&state.pools.online, &req.namespace)
        .await
        .unwrap_or(compile::DEFAULT_WARM_TIER_BUDGET);

    let compilation_input = CompilationInput {
        namespace: req.namespace.clone(),
        task_class: task_class.clone(),
        target_model: target_model.clone(),
        format,
        warm_tier_budget,
        hot_tier_items,
        ranked_candidates: ranked,
    };

    let compilation_result = compile::compile_package(compilation_input);
    let latency_compile_ms = compile_start.elapsed().as_millis() as i32;
    let latency_total_ms = total_start.elapsed().as_millis() as i32;

    // -----------------------------------------------------------------------
    // Stage 8: Update serving state (access counts)
    // -----------------------------------------------------------------------
    // Fire-and-forget: update access counts for selected candidates.
    // Errors here are non-fatal — we log and continue.
    let selected_ids: Vec<Uuid> = compilation_result
        .selected_items
        .iter()
        .map(|s| s.id)
        .collect();
    if !selected_ids.is_empty() {
        let pool = state.pools.online.clone();
        tokio::spawn(async move {
            update_access_counts(&pool, &selected_ids).await;
        });
    }

    // -----------------------------------------------------------------------
    // Stage 9: Write audit log
    // -----------------------------------------------------------------------
    let audit_entry = compile::build_audit_entry(
        &compilation_result,
        &req.namespace,
        task_class,
        Some(&req.query),
        Some(&target_model),
        &classify_output.result.primary_class.to_string(),
        classify_output
            .result
            .secondary_class
            .as_ref()
            .map(|c| c.to_string())
            .as_deref(),
        classify_output.result.primary_confidence.into(),
        classify_output.result.secondary_confidence,
        &profile_names,
        Some(latency_total_ms),
        Some(latency_classify_ms),
        Some(latency_retrieve_ms),
        Some(latency_rank_ms),
        Some(latency_compile_ms),
    );

    let pool = state.pools.online.clone();
    tokio::spawn(async move {
        if let Err(e) = audit::insert_audit_entry(&pool, &audit_entry).await {
            tracing::error!(error = %e, "failed to write audit log entry");
        }
    });

    tracing::info!(
        compilation_id = %compilation_result.package.compilation_id,
        token_count = compilation_result.package.token_count,
        latency_total_ms,
        namespace = %req.namespace,
        task_class = %task_class,
        "loom_think: compilation complete"
    );

    Ok(Json(ThinkResponse {
        context_package: compilation_result.package.context_package,
        token_count: compilation_result.package.token_count,
        compilation_id: compilation_result.package.compilation_id,
    }))
}

// ---------------------------------------------------------------------------
// 15.3 — loom_recall
// ---------------------------------------------------------------------------

/// Handle a `loom_recall` request.
///
/// Direct fact lookup for named entities. Bypasses intent classification and
/// retrieval profiles. Returns raw facts with provenance.
pub async fn handle_loom_recall(
    State(state): State<AppState>,
    Json(req): Json<RecallRequest>,
) -> Result<Json<RecallResponse>, McpError> {
    if req.entity_names.is_empty() {
        return Err(McpError::InvalidRequest(
            "entity_names must not be empty".into(),
        ));
    }
    if req.namespace.trim().is_empty() {
        return Err(McpError::InvalidRequest("namespace must not be empty".into()));
    }

    // Resolve entity IDs from names.
    let entity_ids = resolve_entity_ids(
        &state.pools.online,
        &req.entity_names,
        &req.namespace,
    )
    .await
    .map_err(|e| McpError::Database(e.to_string()))?;

    if entity_ids.is_empty() {
        tracing::debug!(
            namespace = %req.namespace,
            entity_names = ?req.entity_names,
            "loom_recall: no entities found for given names"
        );
        return Ok(Json(RecallResponse { facts: vec![] }));
    }

    // Fetch facts for each entity.
    let mut all_facts = Vec::new();
    for entity_id in &entity_ids {
        let entity_facts = if req.include_historical {
            // Include superseded and deleted facts.
            fetch_all_facts_by_entity(&state.pools.online, *entity_id, &req.namespace)
                .await
                .map_err(|e| McpError::Database(e.to_string()))?
        } else {
            // Current facts only (valid_until IS NULL, deleted_at IS NULL).
            facts::query_facts_by_entity(&state.pools.online, *entity_id, &req.namespace)
                .await
                .map_err(|e| McpError::Database(e.to_string()))?
        };
        all_facts.extend(entity_facts);
    }

    // Deduplicate by fact ID.
    let mut seen = std::collections::HashSet::new();
    all_facts.retain(|f| seen.insert(f.id));

    tracing::info!(
        namespace = %req.namespace,
        entity_count = entity_ids.len(),
        fact_count = all_facts.len(),
        include_historical = req.include_historical,
        "loom_recall: facts retrieved"
    );

    Ok(Json(RecallResponse { facts: all_facts }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve entity UUIDs from a list of entity names within a namespace.
async fn resolve_entity_ids(
    pool: &sqlx::PgPool,
    names: &[String],
    namespace: &str,
) -> Result<Vec<Uuid>, sqlx::Error> {
    let mut ids = Vec::new();
    for name in names {
        let row: Option<(Uuid,)> = sqlx::query_as(
            "SELECT id FROM loom_entities WHERE LOWER(name) = LOWER($1) AND namespace = $2 AND deleted_at IS NULL LIMIT 1",
        )
        .bind(name)
        .bind(namespace)
        .fetch_optional(pool)
        .await?;

        if let Some((id,)) = row {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// Fetch all facts for an entity including historical (superseded/deleted).
async fn fetch_all_facts_by_entity(
    pool: &sqlx::PgPool,
    entity_id: Uuid,
    namespace: &str,
) -> Result<Vec<crate::types::fact::Fact>, sqlx::Error> {
    sqlx::query_as(
        r#"
        SELECT *
        FROM loom_facts
        WHERE (subject_id = $1 OR object_id = $1)
          AND namespace = $2
        "#,
    )
    .bind(entity_id)
    .bind(namespace)
    .fetch_all(pool)
    .await
}

/// Fetch hot tier items for a namespace from entity and fact state tables.
///
/// Returns items where `tier = 'hot'` and `deleted_at IS NULL`.
async fn fetch_hot_tier_items(
    pool: &sqlx::PgPool,
    namespace: &str,
) -> Result<Vec<HotTierItem>, sqlx::Error> {
    use crate::pipeline::online::compile::{HotEntity, HotFact, HotTierPayload};
    use crate::pipeline::online::retrieve::MemoryType;

    let mut items = Vec::new();

    // Hot tier facts.
    let hot_facts: Vec<HotFactRow> = sqlx::query_as(
        r#"
        SELECT f.id, e_subj.name AS subject, f.predicate, e_obj.name AS object,
               f.evidence_status, f.valid_from
        FROM loom_facts f
        JOIN loom_fact_state fs ON fs.fact_id = f.id
        JOIN loom_entities e_subj ON e_subj.id = f.subject_id
        JOIN loom_entities e_obj ON e_obj.id = f.object_id
        WHERE f.namespace = $1
          AND f.valid_until IS NULL
          AND f.deleted_at IS NULL
          AND fs.tier = 'hot'
        "#,
    )
    .bind(namespace)
    .fetch_all(pool)
    .await?;

    for row in hot_facts {
        items.push(HotTierItem {
            id: row.id,
            memory_type: MemoryType::Semantic,
            payload: HotTierPayload::Fact(HotFact {
                subject: row.subject,
                predicate: row.predicate,
                object: row.object,
                evidence: row.evidence_status,
                observed: Some(row.valid_from.format("%Y-%m-%d").to_string()),
                source: row.id.to_string(),
            }),
        });
    }

    // Hot tier entities.
    let hot_entities: Vec<HotEntityRow> = sqlx::query_as(
        r#"
        SELECT e.id, e.name, e.entity_type, es.summary
        FROM loom_entities e
        JOIN loom_entity_state es ON es.entity_id = e.id
        WHERE e.namespace = $1
          AND e.deleted_at IS NULL
          AND es.tier = 'hot'
        "#,
    )
    .bind(namespace)
    .fetch_all(pool)
    .await?;

    for row in hot_entities {
        items.push(HotTierItem {
            id: row.id,
            memory_type: MemoryType::Semantic,
            payload: HotTierPayload::Entity(HotEntity {
                name: row.name,
                entity_type: row.entity_type,
                summary: row.summary,
            }),
        });
    }

    Ok(items)
}

/// Fetch the warm tier token budget for a namespace from `loom_namespace_config`.
///
/// Falls back to [`compile::DEFAULT_WARM_TIER_BUDGET`] if no config row exists.
async fn fetch_warm_tier_budget(
    pool: &sqlx::PgPool,
    namespace: &str,
) -> Result<usize, sqlx::Error> {
    let budget: Option<i32> = sqlx::query_scalar(
        "SELECT warm_tier_budget FROM loom_namespace_config WHERE namespace = $1",
    )
    .bind(namespace)
    .fetch_optional(pool)
    .await?;

    Ok(budget.map(|b| b as usize).unwrap_or(compile::DEFAULT_WARM_TIER_BUDGET))
}

/// Fire-and-forget update of access counts for selected candidates.
///
/// Increments `access_count` and sets `last_accessed = now()` on both
/// `loom_fact_state` and `loom_entity_state` for the given IDs.
async fn update_access_counts(pool: &sqlx::PgPool, ids: &[Uuid]) {
    for id in ids {
        // Try fact state first.
        let _ = sqlx::query(
            r#"
            UPDATE loom_fact_state
            SET access_count = access_count + 1,
                last_accessed = now()
            WHERE fact_id = $1
            "#,
        )
        .bind(id)
        .execute(pool)
        .await;

        // Also try entity state.
        let _ = sqlx::query(
            r#"
            UPDATE loom_entity_state
            SET access_count = access_count + 1,
                last_accessed = now()
            WHERE entity_id = $1
            "#,
        )
        .bind(id)
        .execute(pool)
        .await;
    }
}

// ---------------------------------------------------------------------------
// Internal row types for hot tier queries
// ---------------------------------------------------------------------------

#[derive(Debug, sqlx::FromRow)]
struct HotFactRow {
    id: Uuid,
    subject: String,
    predicate: String,
    object: String,
    evidence_status: String,
    valid_from: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, sqlx::FromRow)]
struct HotEntityRow {
    id: Uuid,
    name: String,
    entity_type: String,
    summary: Option<String>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- compute_content_hash -----------------------------------------------

    #[test]
    fn content_hash_is_hex_sha256() {
        let hash = compute_content_hash("hello world");
        // SHA-256 produces 64 hex characters.
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn same_content_produces_same_hash() {
        let h1 = compute_content_hash("test content");
        let h2 = compute_content_hash("test content");
        assert_eq!(h1, h2);
    }

    #[test]
    fn different_content_produces_different_hash() {
        let h1 = compute_content_hash("content A");
        let h2 = compute_content_hash("content B");
        assert_ne!(h1, h2);
    }

    #[test]
    fn empty_content_produces_valid_hash() {
        let hash = compute_content_hash("");
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn known_sha256_value() {
        // SHA-256("abc") = ba7816bf8f01cfea414140de5dae2ec73b00361bbef0469f492c347b8d4f8d
        // (first 8 chars for sanity check)
        let hash = compute_content_hash("abc");
        assert!(hash.starts_with("ba7816bf"));
    }

    // -- McpError display ---------------------------------------------------

    #[test]
    fn mcp_error_database_displays() {
        let err = McpError::Database("connection refused".into());
        assert!(err.to_string().contains("database error"));
        assert!(err.to_string().contains("connection refused"));
    }

    #[test]
    fn mcp_error_invalid_request_displays() {
        let err = McpError::InvalidRequest("content must not be empty".into());
        assert!(err.to_string().contains("invalid request"));
    }

    #[test]
    fn mcp_error_classification_displays() {
        let err = McpError::Classification("LLM timeout".into());
        assert!(err.to_string().contains("classification error"));
    }

    // -- McpError HTTP status codes -----------------------------------------

    #[tokio::test]
    async fn invalid_request_error_returns_400() {
        use axum::http::StatusCode;
        let err = McpError::InvalidRequest("bad input".into());
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn database_error_returns_500() {
        use axum::http::StatusCode;
        let err = McpError::Database("db down".into());
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
