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
    /// Provenance class — one of `user_authored_seed`, `vendor_import`,
    /// `live_mcp_capture`. Enforced by CHECK constraint in migration 015.
    pub ingestion_mode: String,
    /// Parser semantic version. Required when `ingestion_mode = 'vendor_import'`,
    /// must be `None` otherwise (enforced by `chk_parser_fields_vendor_import`).
    pub parser_version: Option<String>,
    /// Vendor export schema version asserted against. Required when
    /// `ingestion_mode = 'vendor_import'`, must be `None` otherwise.
    pub parser_source_schema: Option<String>,
}

/// Insert a new episode with idempotency check.
///
/// If an episode with the same `(source, source_event_id)` already exists the
/// existing row is returned instead of inserting a duplicate.
pub async fn insert_episode(pool: &PgPool, ep: &NewEpisode) -> Result<Episode, EpisodeError> {
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
            occurred_at, namespace, metadata, participants,
            ingestion_mode, parser_version, parser_source_schema
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
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
    .bind(&ep.ingestion_mode)
    .bind(&ep.parser_version)
    .bind(&ep.parser_source_schema)
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
pub async fn get_episode_by_id(pool: &PgPool, id: Uuid) -> Result<Option<Episode>, EpisodeError> {
    let row = sqlx::query_as::<_, Episode>("SELECT * FROM loom_episodes WHERE id = $1")
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

/// List episodes ready for processing, honoring exponential backoff.
///
/// Returns rows with `processing_status = 'pending'` and `deleted_at IS NULL`
/// whose last attempt (if any) occurred more than `base_backoff_secs *
/// 2^processing_attempts` seconds ago. Episodes currently in `processing`,
/// `completed`, or `failed` state are skipped — the worker only picks up
/// `pending` rows and atomically claims them via
/// [`claim_episode_for_processing`] before running the pipeline.
///
/// Ordering prioritizes never-attempted episodes (fair to first-time work)
/// then oldest-last-attempt ones.
pub async fn list_unprocessed_episodes(
    pool: &PgPool,
    limit: i64,
    base_backoff_secs: i64,
) -> Result<Vec<Episode>, EpisodeError> {
    let rows = sqlx::query_as::<_, Episode>(
        r#"
        SELECT *
        FROM loom_episodes
        WHERE processing_status = 'pending'
          AND deleted_at IS NULL
          AND (
            processing_last_attempt IS NULL
            OR processing_last_attempt
               + ($1 * (1::bigint << LEAST(processing_attempts, 20))) * interval '1 second'
               < NOW()
          )
        ORDER BY processing_last_attempt NULLS FIRST, ingested_at ASC
        LIMIT $2
        "#,
    )
    .bind(base_backoff_secs)
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
/// Sets `processing_status = 'completed'`, clears `processing_last_error`,
/// and writes the provided `extraction_metrics` JSONB value on the episode
/// row. `processed = true` is kept in sync for back-compat with read-side
/// tooling that still consults the legacy boolean.
pub async fn mark_episode_processed(
    pool: &PgPool,
    id: Uuid,
    extraction_metrics: &serde_json::Value,
) -> Result<Episode, EpisodeError> {
    let row = sqlx::query_as::<_, Episode>(
        r#"
        UPDATE loom_episodes
        SET processed = true,
            processing_status = 'completed',
            processing_last_error = NULL,
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

// ---------------------------------------------------------------------------
// Processing lifecycle
// ---------------------------------------------------------------------------

/// Maximum characters of an error message persisted in `processing_last_error`.
///
/// Ollama 400 bodies can be long; truncating keeps the column readable in
/// dashboards and bounds storage growth for episodes that fail repeatedly.
const PROCESSING_ERROR_MAX_LEN: usize = 2000;

/// Atomically claim an episode for processing.
///
/// Transitions the row from `pending` to `processing`, increments
/// `processing_attempts`, and stamps `processing_last_attempt = NOW()`.
/// Uses a single conditional UPDATE so two workers cannot both pick up the
/// same episode: only one of them will see a returned row.
///
/// Returns `None` if the episode does not exist, is already in another
/// state, or has been soft-deleted between polling and claim. Callers
/// should treat `None` as "skip — someone else got it" and move on.
pub async fn claim_episode_for_processing(
    pool: &PgPool,
    id: Uuid,
) -> Result<Option<Episode>, EpisodeError> {
    let row = sqlx::query_as::<_, Episode>(
        r#"
        UPDATE loom_episodes
        SET processing_status       = 'processing',
            processing_attempts     = processing_attempts + 1,
            processing_last_attempt = NOW()
        WHERE id = $1
          AND processing_status = 'pending'
          AND deleted_at IS NULL
        RETURNING *
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

/// Record a processing failure and transition back to a retryable state.
///
/// If `processing_attempts >= max_attempts`, sets `processing_status =
/// 'failed'` and stops further retries. Otherwise returns the row to
/// `pending` so the next poll cycle reconsiders it under backoff.
/// `processing_last_error` is populated with a truncated copy of `error`.
pub async fn record_processing_failure(
    pool: &PgPool,
    id: Uuid,
    error: &str,
    max_attempts: i32,
) -> Result<Episode, EpisodeError> {
    let truncated = if error.len() > PROCESSING_ERROR_MAX_LEN {
        // Slice on a char boundary so we never produce invalid UTF-8.
        let mut cut = PROCESSING_ERROR_MAX_LEN;
        while !error.is_char_boundary(cut) && cut > 0 {
            cut -= 1;
        }
        &error[..cut]
    } else {
        error
    };

    let row = sqlx::query_as::<_, Episode>(
        r#"
        UPDATE loom_episodes
        SET processing_status = CASE
                WHEN processing_attempts >= $3 THEN 'failed'
                ELSE 'pending'
            END,
            processing_last_error = $2
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(id)
    .bind(truncated)
    .bind(max_attempts)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

/// Reset an episode to `pending` with `processing_attempts = 0`.
///
/// Escape hatch for operators: after fixing a root cause (e.g. swapping
/// the embedding model to one with a larger context window), clear the
/// failure state so the worker reprocesses the episode on its next poll.
/// Clears `processing_last_error` as well so the dashboard does not show
/// stale failure context.
///
/// Returns `None` if no episode with `id` exists.
pub async fn requeue_episode(pool: &PgPool, id: Uuid) -> Result<Option<Episode>, EpisodeError> {
    let row = sqlx::query_as::<_, Episode>(
        r#"
        UPDATE loom_episodes
        SET processing_status       = 'pending',
            processing_attempts     = 0,
            processing_last_attempt = NULL,
            processing_last_error   = NULL,
            processed               = false
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

/// Count episodes in `failed` state, excluding soft-deleted ones.
pub async fn count_failed_episodes(pool: &PgPool) -> Result<i64, EpisodeError> {
    let count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM loom_episodes
        WHERE processing_status = 'failed'
          AND deleted_at IS NULL
        "#,
    )
    .fetch_one(pool)
    .await?;

    Ok(count)
}

/// List failed episodes for dashboard surfacing, most recent failure first.
///
/// Returns rows with `processing_status = 'failed'` and `deleted_at IS
/// NULL`, ordered by `processing_last_attempt DESC`.
pub async fn list_failed_episodes(pool: &PgPool, limit: i64) -> Result<Vec<Episode>, EpisodeError> {
    let rows = sqlx::query_as::<_, Episode>(
        r#"
        SELECT *
        FROM loom_episodes
        WHERE processing_status = 'failed'
          AND deleted_at IS NULL
        ORDER BY processing_last_attempt DESC NULLS LAST
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows)
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
            classification_model = COALESCE(NULLIF($3, ''), classification_model),
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
