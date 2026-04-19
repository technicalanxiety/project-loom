//! Database queries for the `loom_procedures` table.
//!
//! Provides procedure insertion, promoted-procedure lookups, observation
//! tracking, and namespace-scoped queries. All functions accept a `&PgPool`
//! reference and return `Result<T, ProcedureError>`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

/// Errors that can occur during procedure database operations.
#[derive(Debug, thiserror::Error)]
pub enum ProcedureError {
    /// An underlying sqlx / PostgreSQL error.
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
}

/// A procedure record matching the `loom_procedures` table schema.
///
/// Represents a recurring behavioral pattern observed across multiple
/// episodes. Procedures require multiple observations before promotion.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Procedure {
    /// Unique procedure identifier.
    pub id: Uuid,
    /// Description of the behavioral pattern.
    pub pattern: String,
    /// Optional categorization.
    pub category: Option<String>,
    /// Namespace isolation boundary.
    pub namespace: String,
    /// Which episodes this pattern was observed in.
    pub source_episodes: Vec<Uuid>,
    /// When this pattern was first observed.
    pub first_observed: Option<DateTime<Utc>>,
    /// When this pattern was last observed.
    pub last_observed: Option<DateTime<Utc>>,
    /// How many times this pattern was seen.
    pub observation_count: Option<i32>,
    /// Reliability classification: extracted, promoted, or deprecated.
    pub evidence_status: String,
    /// Confidence score, increases with observations.
    pub confidence: Option<f64>,
    /// 768-dimension embedding from nomic-embed-text.
    #[serde(skip)]
    pub embedding: Option<pgvector::Vector>,
    /// Tier placement: "hot" or "warm".
    pub tier: Option<String>,
    /// Soft-delete timestamp.
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Data required to insert a new procedure.
///
/// Mirrors the caller-provided columns of `loom_procedures`. The database
/// assigns `id`, `first_observed`, `last_observed`, and defaults via schema.
#[derive(Debug)]
pub struct NewProcedure {
    /// Description of the behavioral pattern.
    pub pattern: String,
    /// Optional categorization.
    pub category: Option<String>,
    /// Namespace isolation boundary.
    pub namespace: String,
    /// Initial source episode(s).
    pub source_episodes: Vec<Uuid>,
    /// Reliability classification.
    pub evidence_status: String,
    /// Initial confidence score.
    pub confidence: f64,
}

// ---------------------------------------------------------------------------
// Insert
// ---------------------------------------------------------------------------

/// Insert a new procedure record.
///
/// Creates a new behavioral pattern entry with the provided fields.
/// The database assigns `id`, `first_observed`, and `last_observed` defaults.
pub async fn insert_procedure(
    pool: &PgPool,
    proc: &NewProcedure,
) -> Result<Procedure, ProcedureError> {
    let row = sqlx::query_as::<_, Procedure>(
        r#"
        INSERT INTO loom_procedures (
            pattern, category, namespace, source_episodes,
            evidence_status, confidence
        )
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING *
        "#,
    )
    .bind(&proc.pattern)
    .bind(&proc.category)
    .bind(&proc.namespace)
    .bind(&proc.source_episodes)
    .bind(&proc.evidence_status)
    .bind(proc.confidence)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

// ---------------------------------------------------------------------------
// Read
// ---------------------------------------------------------------------------

/// Query promoted procedures meeting a minimum confidence threshold.
///
/// Returns procedures with `evidence_status = 'promoted'` and
/// `confidence >= min_confidence` in the given namespace. Excludes
/// soft-deleted procedures.
pub async fn get_promoted_procedures(
    pool: &PgPool,
    namespace: &str,
    min_confidence: f64,
) -> Result<Vec<Procedure>, ProcedureError> {
    let rows = sqlx::query_as::<_, Procedure>(
        r#"
        SELECT *
        FROM loom_procedures
        WHERE namespace = $1
          AND evidence_status = 'promoted'
          AND confidence >= $2
          AND deleted_at IS NULL
        ORDER BY confidence DESC
        "#,
    )
    .bind(namespace)
    .bind(min_confidence)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Query all non-deleted procedures in a namespace.
///
/// Returns procedures ordered by `last_observed DESC`.
pub async fn query_by_namespace(
    pool: &PgPool,
    namespace: &str,
) -> Result<Vec<Procedure>, ProcedureError> {
    let rows = sqlx::query_as::<_, Procedure>(
        r#"
        SELECT *
        FROM loom_procedures
        WHERE namespace = $1
          AND deleted_at IS NULL
        ORDER BY last_observed DESC
        "#,
    )
    .bind(namespace)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

/// Record a new observation of an existing procedure.
///
/// Increments `observation_count`, appends `episode_id` to `source_episodes`,
/// and updates `last_observed` to the current time.
pub async fn update_observation(
    pool: &PgPool,
    procedure_id: Uuid,
    episode_id: Uuid,
) -> Result<(), ProcedureError> {
    sqlx::query(
        r#"
        UPDATE loom_procedures
        SET observation_count = observation_count + 1,
            source_episodes = array_append(source_episodes, $2),
            last_observed = now()
        WHERE id = $1
        "#,
    )
    .bind(procedure_id)
    .bind(episode_id)
    .execute(pool)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn procedure_error_displays_message() {
        let err = ProcedureError::Sqlx(sqlx::Error::RowNotFound);
        assert!(err.to_string().contains("database error"));
    }
}
