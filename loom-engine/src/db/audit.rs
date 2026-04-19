//! Database queries for the `loom_audit_log` table.
//!
//! Provides audit entry insertion, paginated queries, single-entry lookup,
//! and user rating updates. All functions accept a `&PgPool` reference and
//! return `Result<T, AuditError>`.

use sqlx::PgPool;
use uuid::Uuid;

use crate::types::audit::AuditLogEntry;

/// Errors that can occur during audit log database operations.
#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    /// An underlying sqlx / PostgreSQL error.
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
}

/// Data required to insert a new audit log entry.
///
/// Mirrors the caller-provided columns of `loom_audit_log`. The database
/// assigns `id` and `created_at` via defaults.
#[derive(Debug)]
pub struct NewAuditEntry {
    /// Classified intent (debug, architecture, compliance, writing, chat).
    pub task_class: String,
    /// Which namespace was queried.
    pub namespace: String,
    /// The original query text.
    pub query_text: Option<String>,
    /// Which model the context was compiled for.
    pub target_model: Option<String>,
    /// Primary task class.
    pub primary_class: String,
    /// Secondary task class (if confidence gap < 0.3).
    pub secondary_class: Option<String>,
    /// Confidence score for primary class.
    pub primary_confidence: Option<f64>,
    /// Confidence score for secondary class.
    pub secondary_confidence: Option<f64>,
    /// Which retrieval profiles ran.
    pub profiles_executed: Option<Vec<String>>,
    /// Primary retrieval profile used.
    pub retrieval_profile: String,
    /// Total candidates from all profiles.
    pub candidates_found: Option<i32>,
    /// Candidates included in final package.
    pub candidates_selected: Option<i32>,
    /// Candidates excluded from final package.
    pub candidates_rejected: Option<i32>,
    /// Selected items with score breakdowns (JSONB).
    pub selected_items: Option<serde_json::Value>,
    /// Rejected items with rejection reasons (JSONB).
    pub rejected_items: Option<serde_json::Value>,
    /// Total tokens in compiled package.
    pub compiled_tokens: Option<i32>,
    /// Output format: "structured" or "compact".
    pub output_format: Option<String>,
    /// End-to-end latency in milliseconds.
    pub latency_total_ms: Option<i32>,
    /// Intent classification stage latency.
    pub latency_classify_ms: Option<i32>,
    /// Retrieval profile execution stage latency.
    pub latency_retrieve_ms: Option<i32>,
    /// Ranking and trimming stage latency.
    pub latency_rank_ms: Option<i32>,
    /// Package compilation stage latency.
    pub latency_compile_ms: Option<i32>,
}

// ---------------------------------------------------------------------------
// Insert
// ---------------------------------------------------------------------------

/// Insert a new audit log entry.
///
/// Records a full compilation trace including classification, retrieval,
/// candidate decisions, token counts, and latency breakdown.
pub async fn insert_audit_entry(
    pool: &PgPool,
    entry: &NewAuditEntry,
) -> Result<AuditLogEntry, AuditError> {
    let row = sqlx::query_as::<_, AuditLogEntry>(
        r#"
        INSERT INTO loom_audit_log (
            task_class, namespace, query_text, target_model,
            primary_class, secondary_class, primary_confidence, secondary_confidence,
            profiles_executed, retrieval_profile,
            candidates_found, candidates_selected, candidates_rejected,
            selected_items, rejected_items,
            compiled_tokens, output_format,
            latency_total_ms, latency_classify_ms, latency_retrieve_ms,
            latency_rank_ms, latency_compile_ms
        )
        VALUES (
            $1, $2, $3, $4,
            $5, $6, $7, $8,
            $9, $10,
            $11, $12, $13,
            $14, $15,
            $16, $17,
            $18, $19, $20,
            $21, $22
        )
        RETURNING *
        "#,
    )
    .bind(&entry.task_class)
    .bind(&entry.namespace)
    .bind(&entry.query_text)
    .bind(&entry.target_model)
    .bind(&entry.primary_class)
    .bind(&entry.secondary_class)
    .bind(entry.primary_confidence)
    .bind(entry.secondary_confidence)
    .bind(&entry.profiles_executed)
    .bind(&entry.retrieval_profile)
    .bind(entry.candidates_found)
    .bind(entry.candidates_selected)
    .bind(entry.candidates_rejected)
    .bind(&entry.selected_items)
    .bind(&entry.rejected_items)
    .bind(entry.compiled_tokens)
    .bind(&entry.output_format)
    .bind(entry.latency_total_ms)
    .bind(entry.latency_classify_ms)
    .bind(entry.latency_retrieve_ms)
    .bind(entry.latency_rank_ms)
    .bind(entry.latency_compile_ms)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

// ---------------------------------------------------------------------------
// Read
// ---------------------------------------------------------------------------

/// Paginated query of audit log entries for a namespace.
///
/// Returns entries ordered by `created_at DESC` with limit/offset pagination.
pub async fn query_audit_logs(
    pool: &PgPool,
    namespace: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<AuditLogEntry>, AuditError> {
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

/// Fetch a single audit log entry by its UUID.
///
/// Returns `None` if no entry with the given id exists.
pub async fn get_audit_entry(
    pool: &PgPool,
    id: Uuid,
) -> Result<Option<AuditLogEntry>, AuditError> {
    let row = sqlx::query_as::<_, AuditLogEntry>(
        "SELECT * FROM loom_audit_log WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

/// Update the user_rating field on an audit log entry.
///
/// Records optional user feedback for retrieval quality analysis.
pub async fn update_user_rating(
    pool: &PgPool,
    id: Uuid,
    rating: f64,
) -> Result<AuditLogEntry, AuditError> {
    let row = sqlx::query_as::<_, AuditLogEntry>(
        r#"
        UPDATE loom_audit_log
        SET user_rating = $2
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(rating)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_error_displays_message() {
        let err = AuditError::Sqlx(sqlx::Error::RowNotFound);
        assert!(err.to_string().contains("database error"));
    }
}
