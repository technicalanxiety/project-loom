//! Database queries for the `loom_episodes` table.
//!
//! Provides CRUD operations, idempotent insertion, vector similarity search,
//! soft deletion, and processing lifecycle management. All functions accept a
//! `&PgPool` reference and return `Result<T, EpisodeError>`.

use chrono::{DateTime, Utc};
use pgvector::Vector;
use sqlx::PgPool;
use uuid::Uuid;

use crate::types::episode::Episode;

/// Errors that can occur during episode database operations.
#[derive(Debug, thiserror::Error)]
pub enum EpisodeError {
    /// An underlying sqlx / PostgreSQL error.
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
}

// ---------------------------------------------------------------------------
// Insert
// ---------------------------------------------------------------------------

/// Data required to insert a new episode.
///
/// Mirrors the canonical columns of `loom_episodes` that the caller must
/// provide. Derived columns (embedding, tags, processed) are set by the
/// database or later pipeline stages.
#[derive(Debug)]
pub struct NewEpisode {
    /// Source system identifier (e.g. "claude-code", "manual", "github").
    pub source: String,
    /// External system identifier.
    pub source_id: Option<String>,
    /// Deduplication key within the source.
    pub source_event_id: Option<String>,
    /// Raw episode text content.
    pub content: String,
    /// SHA-256 content hash for deduplication.
    pub content_hash: String,
    /// When the interaction happened.
    pub occurred_at: DateTime<Utc>,
    /// Namespace isolation boundary.
    pub namespace: String,
    /// Flexible source-specific metadata (JSONB).
    pub metadata: Option<serde_json::Value>,
    /// People involved in the interaction.
    pub participants: Option<Vec<String>>,
}

/// Insert a new episode with idempotency check.
///
/// If an episode with the same `(source, source_event_id)` already exists the
/// existing row is returned instead of inserting a duplicate.
pub async fn insert_episode(
    pool: &PgPool,
    ep: &NewEpisode,
) -> Result<Episode, EpisodeError> {
    // Check for existing episode by (source, source_event_id) when both are present.
    if let Some(ref event_id) = ep.source_event_id {
        let existing: Option<Episode> = sqlx::query_as::<_, Episode>(
            "SELECT * FROM loom_episodes WHERE source = $1 AND source_event_id = $2",
        )
        .bind(&ep.source)
        .bind(event_id)
        .fetch_optional(pool)
        .await?;

        if let Some(row) = existing {
            return Ok(row);
        }
    }

    let row = sqlx::query_as::<_, Episode>(
        r#"
        INSERT INTO loom_episodes (
            source, source_id, source_event_id, content, content_hash,
            occurred_at, namespace, metadata, participants
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        RETURNING *
        "#,
    )
    .bind(&ep.source)
    .bind(&ep.source_id)
    .bind(&ep.source_event_id)
    .bind(&ep.content)
    .bind(&ep.content_hash)
    .bind(ep.occurred_at)
    .bind(&ep.namespace)
    .bind(&ep.metadata)
    .bind(&ep.participants)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

// ---------------------------------------------------------------------------
// Read
// ---------------------------------------------------------------------------

/// Fetch a single episode by its UUID.
///
/// Returns `None` if no episode with the given id exists.
pub async fn get_episode_by_id(
    pool: &PgPool,
    id: Uuid,
) -> Result<Option<Episode>, EpisodeError> {
    let row = sqlx::query_as::<_, Episode>(
        "SELECT * FROM loom_episodes WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

/// Query episodes by namespace with optional vector similarity search.
///
/// When `embedding` is provided the results are ordered by cosine similarity
/// (closest first) using pgvector's `<=>` operator. Otherwise results are
/// ordered by `occurred_at DESC`. Soft-deleted episodes are always excluded.
pub async fn query_episodes_by_namespace(
    pool: &PgPool,
    namespace: &str,
    embedding: Option<&Vector>,
    limit: i64,
) -> Result<Vec<Episode>, EpisodeError> {
    let rows = match embedding {
        Some(vec) => {
            sqlx::query_as::<_, Episode>(
                r#"
                SELECT *
                FROM loom_episodes
                WHERE namespace = $1
                  AND deleted_at IS NULL
                ORDER BY embedding <=> $2
                LIMIT $3
                "#,
            )
            .bind(namespace)
            .bind(vec)
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query_as::<_, Episode>(
                r#"
                SELECT *
                FROM loom_episodes
                WHERE namespace = $1
                  AND deleted_at IS NULL
                ORDER BY occurred_at DESC
                LIMIT $2
                "#,
            )
            .bind(namespace)
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
    };

    Ok(rows)
}

/// List unprocessed episodes ordered by ingestion time (oldest first).
///
/// Only returns episodes where `processed = false` and `deleted_at IS NULL`.
pub async fn list_unprocessed_episodes(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<Episode>, EpisodeError> {
    let rows = sqlx::query_as::<_, Episode>(
        r#"
        SELECT *
        FROM loom_episodes
        WHERE processed = false
          AND deleted_at IS NULL
        ORDER BY ingested_at ASC
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

/// Store a 768-dimension embedding on an episode record.
///
/// Called by the offline pipeline after generating the episode embedding
/// via nomic-embed-text. This is a separate step from `mark_episode_processed`
/// because embedding generation happens before entity/fact extraction.
pub async fn store_episode_embedding(
    pool: &PgPool,
    id: Uuid,
    embedding: &Vector,
) -> Result<Episode, EpisodeError> {
    let row = sqlx::query_as::<_, Episode>(
        r#"
        UPDATE loom_episodes
        SET embedding = $2
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(embedding)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

/// Mark an episode as processed and store extraction metrics.
///
/// Sets `processed = true` and writes the provided `extraction_metrics` JSONB
/// value on the episode row.
pub async fn mark_episode_processed(
    pool: &PgPool,
    id: Uuid,
    extraction_metrics: &serde_json::Value,
) -> Result<Episode, EpisodeError> {
    let row = sqlx::query_as::<_, Episode>(
        r#"
        UPDATE loom_episodes
        SET processed = true,
            extraction_metrics = $2
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(extraction_metrics)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

/// Update extraction lineage fields on an episode.
///
/// Sets `extraction_model`, `classification_model`, and `extraction_metrics`
/// JSONB in a single statement.
pub async fn update_extraction_metrics(
    pool: &PgPool,
    id: Uuid,
    extraction_model: &str,
    classification_model: &str,
    extraction_metrics: &serde_json::Value,
) -> Result<Episode, EpisodeError> {
    let row = sqlx::query_as::<_, Episode>(
        r#"
        UPDATE loom_episodes
        SET extraction_model = $2,
            classification_model = $3,
            extraction_metrics = $4
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(extraction_model)
    .bind(classification_model)
    .bind(extraction_metrics)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

// ---------------------------------------------------------------------------
// Soft delete
// ---------------------------------------------------------------------------

/// Soft-delete an episode by setting `deleted_at` and `deletion_reason`.
///
/// The row remains in the database but is excluded from normal queries.
pub async fn soft_delete_episode(
    pool: &PgPool,
    id: Uuid,
    deletion_reason: &str,
) -> Result<Episode, EpisodeError> {
    let row = sqlx::query_as::<_, Episode>(
        r#"
        UPDATE loom_episodes
        SET deleted_at = now(),
            deletion_reason = $2
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(deletion_reason)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

/// Query soft-deleted episodes for audit purposes.
///
/// Returns episodes where `deleted_at IS NOT NULL` in the given namespace,
/// ordered by deletion time descending.
pub async fn query_deleted_episodes(
    pool: &PgPool,
    namespace: &str,
    limit: i64,
) -> Result<Vec<Episode>, EpisodeError> {
    let rows = sqlx::query_as::<_, Episode>(
        r#"
        SELECT *
        FROM loom_episodes
        WHERE namespace = $1
          AND deleted_at IS NOT NULL
        ORDER BY deleted_at DESC
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
    fn episode_error_displays_message() {
        let err = EpisodeError::Sqlx(sqlx::Error::RowNotFound);
        assert!(err.to_string().contains("database error"));
    }
}
