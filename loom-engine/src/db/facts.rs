//! Database queries for the `loom_facts` and `loom_fact_state` tables.
//!
//! Provides fact CRUD operations, supersession management, temporal filtering,
//! entity-based lookups, and fact state upserts. All functions accept a
//! `&PgPool` reference and return `Result<T, FactError>`.

use chrono::{DateTime, Utc};
use pgvector::Vector;
use sqlx::PgPool;
use uuid::Uuid;

use crate::types::fact::{Fact, FactState};

/// Errors that can occur during fact database operations.
#[derive(Debug, thiserror::Error)]
pub enum FactError {
    /// An underlying sqlx / PostgreSQL error.
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
}

/// Data required to insert a new fact.
///
/// Mirrors the caller-provided columns of `loom_facts`. Derived columns
/// (id, valid_from, created_at) are set by the database.
#[derive(Debug)]
pub struct NewFact {
    /// Subject entity identifier.
    pub subject_id: Uuid,
    /// Relationship type (canonical or custom predicate).
    pub predicate: String,
    /// Object entity identifier.
    pub object_id: Uuid,
    /// Namespace isolation boundary.
    pub namespace: String,
    /// Which episodes this fact was extracted from.
    pub source_episodes: Vec<Uuid>,
    /// Reliability classification.
    pub evidence_status: String,
    /// Evidence strength: "explicit" or "implied".
    pub evidence_strength: Option<String>,
    /// Flexible additional properties (JSONB).
    pub properties: Option<serde_json::Value>,
}

// ---------------------------------------------------------------------------
// Insert
// ---------------------------------------------------------------------------

/// Insert a new fact with provenance tracking.
///
/// Accepts a pool reference and a `NewFact` struct. The database assigns
/// `id`, `valid_from`, and `created_at` via defaults. Returns the inserted
/// `Fact` row.
pub async fn insert_fact(
    pool: &PgPool,
    fact: &NewFact,
) -> Result<Fact, FactError> {
    let row = sqlx::query_as::<_, Fact>(
        r#"
        INSERT INTO loom_facts (
            subject_id, predicate, object_id, namespace,
            source_episodes, evidence_status, evidence_strength, properties
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        RETURNING *
        "#,
    )
    .bind(fact.subject_id)
    .bind(&fact.predicate)
    .bind(fact.object_id)
    .bind(&fact.namespace)
    .bind(&fact.source_episodes)
    .bind(&fact.evidence_status)
    .bind(&fact.evidence_strength)
    .bind(&fact.properties)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

// ---------------------------------------------------------------------------
// Read
// ---------------------------------------------------------------------------

/// Fetch a single fact by its UUID.
///
/// Returns `None` if no fact with the given id exists.
pub async fn get_fact_by_id(
    pool: &PgPool,
    id: Uuid,
) -> Result<Option<Fact>, FactError> {
    let row = sqlx::query_as::<_, Fact>(
        "SELECT * FROM loom_facts WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

/// Query currently valid facts for a given namespace.
///
/// Returns facts where `valid_until IS NULL` and `deleted_at IS NULL`,
/// ordered by `created_at DESC`. Uses the `idx_facts_current` partial index.
pub async fn query_current_facts_by_namespace(
    pool: &PgPool,
    namespace: &str,
    limit: i64,
) -> Result<Vec<Fact>, FactError> {
    let rows = sqlx::query_as::<_, Fact>(
        r#"
        SELECT *
        FROM loom_facts
        WHERE namespace = $1
          AND valid_until IS NULL
          AND deleted_at IS NULL
        ORDER BY created_at DESC
        LIMIT $2
        "#,
    )
    .bind(namespace)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Query facts where a given entity is either the subject or object.
///
/// Filters to currently valid facts (`valid_until IS NULL`, `deleted_at IS NULL`)
/// within the specified namespace. Returns `Vec<Fact>`.
pub async fn query_facts_by_entity(
    pool: &PgPool,
    entity_id: Uuid,
    namespace: &str,
) -> Result<Vec<Fact>, FactError> {
    let rows = sqlx::query_as::<_, Fact>(
        r#"
        SELECT *
        FROM loom_facts
        WHERE (subject_id = $1 OR object_id = $1)
          AND namespace = $2
          AND valid_until IS NULL
          AND deleted_at IS NULL
        "#,
    )
    .bind(entity_id)
    .bind(namespace)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Query facts with the same subject and predicate in a namespace.
///
/// Filters to currently valid facts (`valid_until IS NULL`, `deleted_at IS NULL`).
/// Used for supersession detection — finding existing facts that a new fact
/// may contradict.
pub async fn query_facts_by_subject_and_predicate(
    pool: &PgPool,
    subject_id: Uuid,
    predicate: &str,
    namespace: &str,
) -> Result<Vec<Fact>, FactError> {
    let rows = sqlx::query_as::<_, Fact>(
        r#"
        SELECT *
        FROM loom_facts
        WHERE subject_id = $1
          AND predicate = $2
          AND namespace = $3
          AND valid_until IS NULL
          AND deleted_at IS NULL
        "#,
    )
    .bind(subject_id)
    .bind(predicate)
    .bind(namespace)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

// ---------------------------------------------------------------------------
// Update — Supersession
// ---------------------------------------------------------------------------

/// Supersede an old fact by setting temporal and status fields.
///
/// Sets `valid_until` to `now()`, `superseded_by` to the new fact's id, and
/// `evidence_status` to `'superseded'` on the old fact. Returns the updated
/// `Fact` row.
pub async fn supersede_fact(
    pool: &PgPool,
    old_fact_id: Uuid,
    new_fact_id: Uuid,
) -> Result<Fact, FactError> {
    let row = sqlx::query_as::<_, Fact>(
        r#"
        UPDATE loom_facts
        SET valid_until = now(),
            superseded_by = $2,
            evidence_status = 'superseded'
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(old_fact_id)
    .bind(new_fact_id)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

/// Supersede an old fact using a specific `valid_until` timestamp.
///
/// Like [`supersede_fact`] but sets `valid_until` to the provided timestamp
/// (typically the new fact's `valid_from`) instead of `now()`. This ensures
/// the temporal chain accurately reflects when the old fact stopped being
/// current.
pub async fn supersede_fact_at(
    pool: &PgPool,
    old_fact_id: Uuid,
    new_fact_id: Uuid,
    valid_until: DateTime<Utc>,
) -> Result<Fact, FactError> {
    let row = sqlx::query_as::<_, Fact>(
        r#"
        UPDATE loom_facts
        SET valid_until = $3,
            superseded_by = $2,
            evidence_status = 'superseded'
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(old_fact_id)
    .bind(new_fact_id)
    .bind(valid_until)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

// ---------------------------------------------------------------------------
// Update — Soft delete
// ---------------------------------------------------------------------------

/// Soft-delete a fact by setting `deleted_at` to the current time.
///
/// The row remains in the database but is excluded from normal queries.
/// Returns the updated `Fact` row.
pub async fn soft_delete_fact(
    pool: &PgPool,
    id: Uuid,
) -> Result<Fact, FactError> {
    let row = sqlx::query_as::<_, Fact>(
        r#"
        UPDATE loom_facts
        SET deleted_at = now()
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(id)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

// ---------------------------------------------------------------------------
// Fact state upsert
// ---------------------------------------------------------------------------

/// Upsert fact serving state into `loom_fact_state`.
///
/// Inserts a new row or updates the existing row on conflict with `fact_id`.
/// Sets embedding, salience_score, tier, access_count, last_accessed, pinned,
/// and updated_at.
pub async fn update_fact_state(
    pool: &PgPool,
    fact_id: Uuid,
    embedding: Option<&Vector>,
    salience_score: f64,
    tier: &str,
    access_count: i32,
    last_accessed: Option<DateTime<Utc>>,
    pinned: bool,
) -> Result<FactState, FactError> {
    let row = sqlx::query_as::<_, FactState>(
        r#"
        INSERT INTO loom_fact_state (
            fact_id, embedding, salience_score, tier,
            access_count, last_accessed, pinned, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, now())
        ON CONFLICT (fact_id) DO UPDATE SET
            embedding = COALESCE($2, loom_fact_state.embedding),
            salience_score = $3,
            tier = $4,
            access_count = $5,
            last_accessed = $6,
            pinned = $7,
            updated_at = now()
        RETURNING *
        "#,
    )
    .bind(fact_id)
    .bind(embedding)
    .bind(salience_score)
    .bind(tier)
    .bind(access_count)
    .bind(last_accessed)
    .bind(pinned)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fact_error_displays_message() {
        let err = FactError::Sqlx(sqlx::Error::RowNotFound);
        assert!(err.to_string().contains("database error"));
    }

    #[test]
    fn new_fact_debug_format() {
        let fact = NewFact {
            subject_id: Uuid::new_v4(),
            predicate: "uses".to_string(),
            object_id: Uuid::new_v4(),
            namespace: "default".to_string(),
            source_episodes: vec![Uuid::new_v4()],
            evidence_status: "extracted".to_string(),
            evidence_strength: Some("explicit".to_string()),
            properties: None,
        };
        let debug = format!("{fact:?}");
        assert!(debug.contains("uses"));
    }
}
