//! Read-only dashboard data queries and two write endpoints.
//!
//! Provides aggregate pipeline health metrics, compilation trace listing,
//! conflict queue management, predicate candidate surfacing, and namespace
//! listing. All functions accept a `&PgPool` reference and return
//! `Result<T, DashboardError>`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::types::audit::AuditLogEntry;
use crate::types::entity::ResolutionConflict;
use crate::types::predicate::PredicateCandidate;

/// Errors that can occur during dashboard database operations.
#[derive(Debug, thiserror::Error)]
pub enum DashboardError {
    /// An underlying sqlx / PostgreSQL error.
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
}

/// Aggregate pipeline health metrics for the dashboard overview.
///
/// Combines counts from episodes, entities, and facts tables into a
/// single summary view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineHealth {
    /// Total number of episodes ingested.
    pub total_episodes: i64,
    /// Number of episodes that have been processed.
    pub processed_episodes: i64,
    /// Number of unprocessed episodes in the queue.
    pub unprocessed_queue_depth: i64,
    /// Total number of non-deleted entities.
    pub total_entities: i64,
    /// Entity counts broken down by type.
    pub entities_by_type: Vec<TypeCount>,
    /// Total number of currently valid facts.
    pub current_facts: i64,
    /// Total number of superseded facts.
    pub superseded_facts: i64,
}

/// A count grouped by a type label (e.g. entity type).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TypeCount {
    /// The type label.
    pub entity_type: String,
    /// The count for this type.
    pub count: i64,
}

/// Namespace information for the dashboard namespace selector.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct NamespaceInfo {
    /// Namespace identifier.
    pub namespace: String,
    /// Human-readable description.
    pub description: Option<String>,
    /// Hot tier token budget.
    pub hot_tier_budget: Option<i32>,
    /// Warm tier token budget.
    pub warm_tier_budget: Option<i32>,
    /// Active predicate packs for this namespace.
    pub predicate_packs: Option<Vec<String>>,
    /// When this namespace config was created.
    pub created_at: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// Aggregate counts (helper struct for single-row results)
// ---------------------------------------------------------------------------

/// Helper for fetching a single count from an aggregate query.
#[derive(Debug, sqlx::FromRow)]
struct CountRow {
    /// The aggregate count value.
    count: i64,
}

// ---------------------------------------------------------------------------
// Pipeline health
// ---------------------------------------------------------------------------

/// Aggregate pipeline health metrics across all tables.
///
/// Runs multiple count queries to build a comprehensive health summary
/// for the dashboard overview panel.
pub async fn get_pipeline_health(
    pool: &PgPool,
) -> Result<PipelineHealth, DashboardError> {
    let total_episodes = sqlx::query_as::<_, CountRow>(
        "SELECT COUNT(*) AS count FROM loom_episodes WHERE deleted_at IS NULL",
    )
    .fetch_one(pool)
    .await?
    .count;

    let processed_episodes = sqlx::query_as::<_, CountRow>(
        "SELECT COUNT(*) AS count FROM loom_episodes WHERE processed = true AND deleted_at IS NULL",
    )
    .fetch_one(pool)
    .await?
    .count;

    let unprocessed_queue_depth = total_episodes - processed_episodes;

    let total_entities = sqlx::query_as::<_, CountRow>(
        "SELECT COUNT(*) AS count FROM loom_entities WHERE deleted_at IS NULL",
    )
    .fetch_one(pool)
    .await?
    .count;

    let entities_by_type = sqlx::query_as::<_, TypeCount>(
        r#"
        SELECT entity_type, COUNT(*) AS count
        FROM loom_entities
        WHERE deleted_at IS NULL
        GROUP BY entity_type
        ORDER BY count DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    let current_facts = sqlx::query_as::<_, CountRow>(
        r#"
        SELECT COUNT(*) AS count
        FROM loom_facts
        WHERE valid_until IS NULL AND deleted_at IS NULL
        "#,
    )
    .fetch_one(pool)
    .await?
    .count;

    let superseded_facts = sqlx::query_as::<_, CountRow>(
        r#"
        SELECT COUNT(*) AS count
        FROM loom_facts
        WHERE superseded_by IS NOT NULL AND deleted_at IS NULL
        "#,
    )
    .fetch_one(pool)
    .await?
    .count;

    Ok(PipelineHealth {
        total_episodes,
        processed_episodes,
        unprocessed_queue_depth,
        total_entities,
        entities_by_type,
        current_facts,
        superseded_facts,
    })
}

// ---------------------------------------------------------------------------
// Compilation traces
// ---------------------------------------------------------------------------

/// Paginated audit log entries for a namespace.
///
/// Used by the compilation trace viewer in the dashboard.
pub async fn get_compilation_traces(
    pool: &PgPool,
    namespace: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<AuditLogEntry>, DashboardError> {
    let rows = sqlx::query_as::<_, AuditLogEntry>(
        r#"
        SELECT *
        FROM loom_audit_log
        WHERE namespace = $1
        ORDER BY created_at DESC
        LIMIT $2 OFFSET $3
        "#,
    )
    .bind(namespace)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

// ---------------------------------------------------------------------------
// Conflict queue
// ---------------------------------------------------------------------------

/// Unresolved entity resolution conflicts.
///
/// Returns conflicts ordered by creation time (newest first) for the
/// dashboard conflict review queue.
pub async fn get_conflict_queue(
    pool: &PgPool,
) -> Result<Vec<ResolutionConflict>, DashboardError> {
    let rows = sqlx::query_as::<_, ResolutionConflict>(
        r#"
        SELECT *
        FROM loom_resolution_conflicts
        WHERE resolved = false
        ORDER BY created_at DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Mark a resolution conflict as resolved.
///
/// One of the two dashboard write endpoints. Sets `resolved = true`,
/// records the resolution decision, and timestamps the resolution.
pub async fn resolve_conflict(
    pool: &PgPool,
    conflict_id: Uuid,
    resolution: &str,
) -> Result<ResolutionConflict, DashboardError> {
    let row = sqlx::query_as::<_, ResolutionConflict>(
        r#"
        UPDATE loom_resolution_conflicts
        SET resolved = true,
            resolution = $2,
            resolved_at = now()
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(conflict_id)
    .bind(resolution)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

// ---------------------------------------------------------------------------
// Predicate candidates
// ---------------------------------------------------------------------------

/// Unresolved predicate candidates with 5 or more occurrences.
///
/// Surfaces candidates that have been seen frequently enough to warrant
/// operator review in the dashboard.
pub async fn get_predicate_candidates(
    pool: &PgPool,
) -> Result<Vec<PredicateCandidate>, DashboardError> {
    let rows = sqlx::query_as::<_, PredicateCandidate>(
        r#"
        SELECT *
        FROM loom_predicate_candidates
        WHERE resolved_at IS NULL
          AND occurrences >= 5
        ORDER BY occurrences DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

// ---------------------------------------------------------------------------
// Namespace listing
// ---------------------------------------------------------------------------

/// List all namespaces with their configuration.
///
/// Returns namespace info from `loom_namespace_config` for the dashboard
/// namespace selector.
pub async fn get_namespace_list(
    pool: &PgPool,
) -> Result<Vec<NamespaceInfo>, DashboardError> {
    let rows = sqlx::query_as::<_, NamespaceInfo>(
        r#"
        SELECT namespace, description, hot_tier_budget, warm_tier_budget,
               predicate_packs, created_at
        FROM loom_namespace_config
        ORDER BY namespace
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashboard_error_displays_message() {
        let err = DashboardError::Sqlx(sqlx::Error::RowNotFound);
        assert!(err.to_string().contains("database error"));
    }
}
