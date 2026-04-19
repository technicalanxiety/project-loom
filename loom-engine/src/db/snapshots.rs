//! Database queries for the `loom_snapshots` table.
//!
//! Provides snapshot insertion, latest-snapshot lookup, and paginated listing.
//! Snapshots capture the hot tier state at a point in time for audit and
//! debugging purposes. All functions accept a `&PgPool` reference and return
//! `Result<T, SnapshotError>`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

/// Errors that can occur during snapshot database operations.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    /// An underlying sqlx / PostgreSQL error.
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
}

/// A snapshot record matching the `loom_snapshots` table schema.
///
/// Captures the hot tier contents at a point in time for a namespace.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Snapshot {
    /// Unique snapshot identifier.
    pub id: Uuid,
    /// When the snapshot was taken.
    pub snapshot_at: Option<DateTime<Utc>>,
    /// Which namespace this snapshot covers.
    pub namespace: String,
    /// Hot tier entities at snapshot time (JSONB).
    pub hot_entities: Option<serde_json::Value>,
    /// Hot tier facts at snapshot time (JSONB).
    pub hot_facts: Option<serde_json::Value>,
    /// Hot tier procedures at snapshot time (JSONB).
    pub hot_procedures: Option<serde_json::Value>,
    /// Total token count of hot tier at snapshot time.
    pub total_tokens: Option<i32>,
}

/// Data required to insert a new snapshot.
///
/// Mirrors the caller-provided columns of `loom_snapshots`. The database
/// assigns `id` and `snapshot_at` via defaults.
#[derive(Debug)]
pub struct NewSnapshot {
    /// Which namespace this snapshot covers.
    pub namespace: String,
    /// Hot tier entities at snapshot time (JSONB).
    pub hot_entities: Option<serde_json::Value>,
    /// Hot tier facts at snapshot time (JSONB).
    pub hot_facts: Option<serde_json::Value>,
    /// Hot tier procedures at snapshot time (JSONB).
    pub hot_procedures: Option<serde_json::Value>,
    /// Total token count of hot tier at snapshot time.
    pub total_tokens: Option<i32>,
}

// ---------------------------------------------------------------------------
// Insert
// ---------------------------------------------------------------------------

/// Insert a new hot tier snapshot.
///
/// Captures the current state of a namespace's hot tier for audit purposes.
pub async fn insert_snapshot(
    pool: &PgPool,
    snapshot: &NewSnapshot,
) -> Result<Snapshot, SnapshotError> {
    let row = sqlx::query_as::<_, Snapshot>(
        r#"
        INSERT INTO loom_snapshots (
            namespace, hot_entities, hot_facts, hot_procedures, total_tokens
        )
        VALUES ($1, $2, $3, $4, $5)
        RETURNING *
        "#,
    )
    .bind(&snapshot.namespace)
    .bind(&snapshot.hot_entities)
    .bind(&snapshot.hot_facts)
    .bind(&snapshot.hot_procedures)
    .bind(snapshot.total_tokens)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

// ---------------------------------------------------------------------------
// Read
// ---------------------------------------------------------------------------

/// Get the most recent snapshot for a namespace.
///
/// Returns `None` if no snapshots exist for the given namespace.
pub async fn get_latest_snapshot(
    pool: &PgPool,
    namespace: &str,
) -> Result<Option<Snapshot>, SnapshotError> {
    let row = sqlx::query_as::<_, Snapshot>(
        r#"
        SELECT *
        FROM loom_snapshots
        WHERE namespace = $1
        ORDER BY snapshot_at DESC
        LIMIT 1
        "#,
    )
    .bind(namespace)
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

/// List snapshots for a namespace with pagination.
///
/// Returns snapshots ordered by `snapshot_at DESC`.
pub async fn list_snapshots(
    pool: &PgPool,
    namespace: &str,
    limit: i64,
) -> Result<Vec<Snapshot>, SnapshotError> {
    let rows = sqlx::query_as::<_, Snapshot>(
        r#"
        SELECT *
        FROM loom_snapshots
        WHERE namespace = $1
        ORDER BY snapshot_at DESC
        LIMIT $2
        "#,
    )
    .bind(namespace)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_error_displays_message() {
        let err = SnapshotError::Sqlx(sqlx::Error::RowNotFound);
        assert!(err.to_string().contains("database error"));
    }
}
