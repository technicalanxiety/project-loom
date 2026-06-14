//! Database queries for the `loom_consolidation_log` table.
//!
//! Provides consolidation run logging, status queries, and diagnostics.
//! All functions accept a `&PgPool` reference and return
//! `Result<T, ConsolidationError>`.

use sqlx::PgPool;
use uuid::Uuid;

use crate::types::consolidation::ConsolidationLog;

/// Errors that can occur during consolidation log database operations.
#[derive(Debug, thiserror::Error)]
pub enum ConsolidationError {
    /// An underlying sqlx / PostgreSQL error.
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
}

/// Data required to insert a new consolidation log entry.
///
/// Mirrors the caller-provided columns of `loom_consolidation_log`. The database
/// assigns `id` and `started_at` via defaults.
#[derive(Debug)]
pub struct NewConsolidationLog {
    /// Namespace this run executed in.
    pub namespace: String,
    /// Type of run: 'consolidation' or 'pruning'.
    pub run_type: String,
    /// Number of clusters found (consolidation only).
    pub clusters_found: Option<i32>,
    /// Number of new summaries created (consolidation only).
    pub summaries_created: Option<i32>,
    /// Number of existing summaries refreshed (consolidation only).
    pub summaries_refreshed: Option<i32>,
    /// Number of procedures pruned (pruning only).
    pub procedures_pruned: Option<i32>,
    /// Number of conflicts auto-resolved (pruning only).
    pub conflicts_resolved: Option<i32>,
    /// Number of summaries invalidated (pruning only).
    pub summaries_invalidated: Option<i32>,
    /// Error message if the run failed.
    pub error_detail: Option<String>,
    /// Duration in milliseconds.
    pub duration_ms: Option<i32>,
}

// ---------------------------------------------------------------------------
// Insert
// ---------------------------------------------------------------------------

/// Insert a new consolidation log entry at the start of a run.
///
/// Called at the beginning of a consolidation or pruning cycle to record
/// that a run has started. Status defaults to 'running'.
pub async fn insert_consolidation_log(
    pool: &PgPool,
    log: &NewConsolidationLog,
) -> Result<ConsolidationLog, ConsolidationError> {
    let row = sqlx::query_as::<_, ConsolidationLog>(
        r#"
        INSERT INTO loom_consolidation_log (
            namespace, run_type,
            clusters_found, summaries_created, summaries_refreshed,
            procedures_pruned, conflicts_resolved, summaries_invalidated,
            error_detail, duration_ms
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        RETURNING *
        "#,
    )
    .bind(&log.namespace)
    .bind(&log.run_type)
    .bind(log.clusters_found)
    .bind(log.summaries_created)
    .bind(log.summaries_refreshed)
    .bind(log.procedures_pruned)
    .bind(log.conflicts_resolved)
    .bind(log.summaries_invalidated)
    .bind(&log.error_detail)
    .bind(log.duration_ms)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

// ---------------------------------------------------------------------------
// Read
// ---------------------------------------------------------------------------

/// Fetch a consolidation log entry by its UUID.
///
/// Returns `None` if no entry with the given id exists.
pub async fn get_consolidation_log(
    pool: &PgPool,
    id: Uuid,
) -> Result<Option<ConsolidationLog>, ConsolidationError> {
    let row = sqlx::query_as::<_, ConsolidationLog>(
        "SELECT * FROM loom_consolidation_log WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

/// Query recent consolidation runs for a namespace.
///
/// Returns completed or failed consolidation runs (not pruning), ordered by
/// start time descending, with pagination support.
pub async fn query_consolidation_runs(
    pool: &PgPool,
    namespace: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<ConsolidationLog>, ConsolidationError> {
    let rows = sqlx::query_as::<_, ConsolidationLog>(
        r#"
        SELECT *
        FROM loom_consolidation_log
        WHERE namespace = $1
          AND run_type = 'consolidation'
        ORDER BY started_at DESC
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

/// Query recent pruning runs for a namespace.
///
/// Returns pruning activities ordered by start time descending.
pub async fn query_pruning_runs(
    pool: &PgPool,
    namespace: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<ConsolidationLog>, ConsolidationError> {
    let rows = sqlx::query_as::<_, ConsolidationLog>(
        r#"
        SELECT *
        FROM loom_consolidation_log
        WHERE namespace = $1
          AND run_type = 'pruning'
        ORDER BY started_at DESC
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

/// Get the most recent consolidation run for a namespace.
///
/// Returns `None` if no consolidation runs have occurred in this namespace.
pub async fn get_latest_consolidation_run(
    pool: &PgPool,
    namespace: &str,
) -> Result<Option<ConsolidationLog>, ConsolidationError> {
    let row = sqlx::query_as::<_, ConsolidationLog>(
        r#"
        SELECT *
        FROM loom_consolidation_log
        WHERE namespace = $1
          AND run_type = 'consolidation'
        ORDER BY started_at DESC
        LIMIT 1
        "#,
    )
    .bind(namespace)
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

/// Query consolidation runs that failed (status = 'failed').
///
/// Returns failed runs ordered by start time descending for debugging.
pub async fn query_failed_consolidation_runs(
    pool: &PgPool,
    namespace: &str,
    limit: i64,
) -> Result<Vec<ConsolidationLog>, ConsolidationError> {
    let rows = sqlx::query_as::<_, ConsolidationLog>(
        r#"
        SELECT *
        FROM loom_consolidation_log
        WHERE namespace = $1
          AND status = 'failed'
        ORDER BY started_at DESC
        LIMIT $2
        "#,
    )
    .bind(namespace)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

/// Mark a consolidation log entry as completed.
///
/// Updates status to 'completed', sets completed_at, and records final metrics
/// and duration.
pub async fn complete_consolidation_log(
    pool: &PgPool,
    id: Uuid,
    clusters_found: Option<i32>,
    summaries_created: Option<i32>,
    summaries_refreshed: Option<i32>,
    procedures_pruned: Option<i32>,
    conflicts_resolved: Option<i32>,
    summaries_invalidated: Option<i32>,
    duration_ms: i32,
) -> Result<ConsolidationLog, ConsolidationError> {
    let row = sqlx::query_as::<_, ConsolidationLog>(
        r#"
        UPDATE loom_consolidation_log
        SET status = 'completed',
            completed_at = now(),
            clusters_found = $2,
            summaries_created = $3,
            summaries_refreshed = $4,
            procedures_pruned = $5,
            conflicts_resolved = $6,
            summaries_invalidated = $7,
            duration_ms = $8
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(clusters_found)
    .bind(summaries_created)
    .bind(summaries_refreshed)
    .bind(procedures_pruned)
    .bind(conflicts_resolved)
    .bind(summaries_invalidated)
    .bind(duration_ms)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

/// Mark a consolidation log entry as failed.
///
/// Updates status to 'failed', sets completed_at, records the error detail,
/// and final duration.
pub async fn fail_consolidation_log(
    pool: &PgPool,
    id: Uuid,
    error_detail: &str,
    duration_ms: i32,
) -> Result<ConsolidationLog, ConsolidationError> {
    let row = sqlx::query_as::<_, ConsolidationLog>(
        r#"
        UPDATE loom_consolidation_log
        SET status = 'failed',
            completed_at = now(),
            error_detail = $2,
            duration_ms = $3
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(error_detail)
    .bind(duration_ms)
    .fetch_one(pool)
    .await?;

    Ok(row)
}
