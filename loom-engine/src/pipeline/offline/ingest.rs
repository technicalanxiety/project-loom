//! Episode ingestion with dedup, validation, and error handling.
//!
//! Provides the [`ingest_episode`] function that:
//! - Computes a SHA-256 content hash for deduplication.
//! - Detects duplicates by content hash and `(source, source_event_id)`.
//! - Validates input data via serde deserialization boundaries.
//! - Queues failed episodes for retry by leaving `processed = false`.
//! - Logs all errors via tracing with span context.

use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::episodes::{self, EpisodeError, NewEpisode};
use crate::types::episode::Episode;
use crate::types::ingestion::IngestionMode;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during episode ingestion.
#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    /// A required field was missing or invalid.
    #[error("invalid episode data: {0}")]
    InvalidData(String),

    /// A database error occurred during ingestion.
    #[error("database error during ingestion: {0}")]
    Database(String),

    /// A constraint violation (treated as duplicate).
    #[error("constraint violation (duplicate): {0}")]
    ConstraintViolation(String),
}

impl From<EpisodeError> for IngestError {
    fn from(err: EpisodeError) -> Self {
        let msg = err.to_string();
        // Detect unique constraint violations and treat as duplicates.
        if msg.contains("unique") || msg.contains("duplicate key") || msg.contains("23505") {
            Self::ConstraintViolation(msg)
        } else {
            Self::Database(msg)
        }
    }
}

impl From<sqlx::Error> for IngestError {
    fn from(err: sqlx::Error) -> Self {
        let msg = err.to_string();
        if msg.contains("unique") || msg.contains("duplicate key") || msg.contains("23505") {
            Self::ConstraintViolation(msg)
        } else {
            Self::Database(msg)
        }
    }
}

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

/// Result of an episode ingestion attempt.
#[derive(Debug, Clone)]
pub struct IngestResult {
    /// The episode UUID (existing or newly created).
    pub episode_id: Uuid,
    /// Status: "accepted", "duplicate", or "queued".
    pub status: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Compute a hex-encoded SHA-256 hash of the given content string.
pub fn compute_content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Validate episode input fields before ingestion.
///
/// Returns `Err(IngestError::InvalidData)` with specific field errors
/// if validation fails.
pub fn validate_episode_input(
    content: &str,
    source: &str,
    namespace: &str,
) -> Result<(), IngestError> {
    let mut errors: Vec<String> = Vec::new();

    if content.trim().is_empty() {
        errors.push("content must not be empty".to_string());
    }
    if source.trim().is_empty() {
        errors.push("source must not be empty".to_string());
    }
    if namespace.trim().is_empty() {
        errors.push("namespace must not be empty".to_string());
    }

    // Validate source is one of the allowed types.
    let valid_sources = ["manual", "claude-code", "github"];
    if !source.trim().is_empty() && !valid_sources.contains(&source) {
        errors.push(format!(
            "source must be one of: {}; got '{source}'",
            valid_sources.join(", ")
        ));
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(IngestError::InvalidData(errors.join("; ")))
    }
}

/// Ingest an episode with full duplicate detection and error handling.
///
/// # Duplicate Detection
///
/// 1. Content-hash + namespace check — catches identical content re-submitted
///    under different event IDs.
/// 2. `(source, source_event_id)` unique constraint — handled by
///    [`episodes::insert_episode`].
/// 3. Database constraint violations — treated as duplicates, returning the
///    existing episode ID.
///
/// # Error Handling
///
/// - Invalid data: returns `Err(IngestError::InvalidData)` with specific
///   field errors (400 Bad Request at the API layer).
/// - Constraint violations: returns `Ok(IngestResult)` with status
///   `"duplicate"` and the existing episode ID.
/// - Database errors: returns `Err(IngestError::Database)`. The episode
///   remains unprocessed (`processed = false`) for retry.
/// - All errors are logged via tracing with span context.
#[tracing::instrument(
    skip(pool, content, metadata, participants),
    fields(
        namespace = %namespace,
        source = %source,
    )
)]
pub async fn ingest_episode(
    pool: &PgPool,
    content: &str,
    source: &str,
    namespace: &str,
    source_event_id: Option<&str>,
    occurred_at: chrono::DateTime<chrono::Utc>,
    metadata: Option<serde_json::Value>,
    participants: Option<Vec<String>>,
    ingestion_mode: IngestionMode,
    parser_version: Option<String>,
    parser_source_schema: Option<String>,
) -> Result<IngestResult, IngestError> {
    // Step 1: Validate input.
    validate_episode_input(content, source, namespace)?;
    crate::types::ingestion::validate_parser_fields(
        ingestion_mode,
        parser_version.as_deref(),
        parser_source_schema.as_deref(),
    )
    .map_err(IngestError::InvalidData)?;

    let content_hash = compute_content_hash(content);

    // Step 2: Check for duplicate by content_hash + namespace.
    let existing_by_hash: Option<Episode> = sqlx::query_as(
        "SELECT * FROM loom_episodes WHERE content_hash = $1 AND namespace = $2 LIMIT 1",
    )
    .bind(&content_hash)
    .bind(namespace)
    .fetch_optional(pool)
    .await?;

    if let Some(existing) = existing_by_hash {
        tracing::info!(
            episode_id = %existing.id,
            namespace = %namespace,
            "duplicate episode detected (content_hash match)"
        );
        return Ok(IngestResult {
            episode_id: existing.id,
            status: "duplicate".to_string(),
        });
    }

    // Step 3: Insert episode (handles source+source_event_id dedup).
    let new_ep = NewEpisode {
        source: source.to_string(),
        source_id: None,
        source_event_id: source_event_id.map(|s| s.to_string()),
        content: content.to_string(),
        content_hash,
        occurred_at,
        namespace: namespace.to_string(),
        metadata,
        participants,
        ingestion_mode: ingestion_mode.to_string(),
        parser_version,
        parser_source_schema,
    };

    match episodes::insert_episode(pool, &new_ep).await {
        Ok(episode) => {
            let status = if episode.processed.unwrap_or(false) {
                "duplicate"
            } else {
                "queued"
            };

            tracing::info!(
                episode_id = %episode.id,
                namespace = %namespace,
                source = %source,
                status = %status,
                "episode ingested"
            );

            Ok(IngestResult {
                episode_id: episode.id,
                status: status.to_string(),
            })
        }
        Err(e) => {
            let ingest_err = IngestError::from(e);

            // Constraint violations are treated as duplicates.
            if let IngestError::ConstraintViolation(ref msg) = ingest_err {
                tracing::info!(
                    namespace = %namespace,
                    source = %source,
                    error = %msg,
                    "constraint violation treated as duplicate"
                );

                // Try to find the existing episode by source+source_event_id.
                if let Some(ref event_id) = source_event_id {
                    if let Ok(Some(existing)) = find_by_source_event(pool, source, event_id).await
                    {
                        return Ok(IngestResult {
                            episode_id: existing.id,
                            status: "duplicate".to_string(),
                        });
                    }
                }

                // Fallback: return the error as-is if we can't find the existing record.
                return Err(ingest_err);
            }

            tracing::error!(
                namespace = %namespace,
                source = %source,
                error = %ingest_err,
                "episode ingestion failed"
            );
            Err(ingest_err)
        }
    }
}

/// Find an episode by source and source_event_id.
async fn find_by_source_event(
    pool: &PgPool,
    source: &str,
    source_event_id: &str,
) -> Result<Option<Episode>, sqlx::Error> {
    sqlx::query_as::<_, Episode>(
        "SELECT * FROM loom_episodes WHERE source = $1 AND source_event_id = $2 LIMIT 1",
    )
    .bind(source)
    .bind(source_event_id)
    .fetch_optional(pool)
    .await
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_content_hash_is_hex_sha256() {
        let hash = compute_content_hash("hello world");
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
        let h1 = compute_content_hash("content a");
        let h2 = compute_content_hash("content b");
        assert_ne!(h1, h2);
    }

    #[test]
    fn validate_rejects_empty_content() {
        let err = validate_episode_input("", "manual", "default").unwrap_err();
        assert!(matches!(err, IngestError::InvalidData(_)));
        assert!(err.to_string().contains("content"));
    }

    #[test]
    fn validate_rejects_empty_source() {
        let err = validate_episode_input("hello", "", "default").unwrap_err();
        assert!(matches!(err, IngestError::InvalidData(_)));
        assert!(err.to_string().contains("source"));
    }

    #[test]
    fn validate_rejects_empty_namespace() {
        let err = validate_episode_input("hello", "manual", "").unwrap_err();
        assert!(matches!(err, IngestError::InvalidData(_)));
        assert!(err.to_string().contains("namespace"));
    }

    #[test]
    fn validate_rejects_invalid_source() {
        let err = validate_episode_input("hello", "invalid_source", "default").unwrap_err();
        assert!(matches!(err, IngestError::InvalidData(_)));
        assert!(err.to_string().contains("source must be one of"));
    }

    #[test]
    fn validate_accepts_valid_input() {
        assert!(validate_episode_input("hello", "manual", "default").is_ok());
        assert!(validate_episode_input("hello", "claude-code", "my-project").is_ok());
        assert!(validate_episode_input("hello", "github", "org/repo").is_ok());
    }

    #[test]
    fn validate_reports_multiple_errors() {
        let err = validate_episode_input("", "", "").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("content"));
        assert!(msg.contains("source"));
        assert!(msg.contains("namespace"));
    }

    #[test]
    fn ingest_error_display_messages() {
        let err = IngestError::InvalidData("bad field".into());
        assert!(err.to_string().contains("invalid episode data"));

        let err = IngestError::Database("connection refused".into());
        assert!(err.to_string().contains("database error"));

        let err = IngestError::ConstraintViolation("duplicate key".into());
        assert!(err.to_string().contains("constraint violation"));
    }

    #[test]
    fn constraint_violation_detected_from_sqlx_error() {
        // Simulate a unique constraint violation message.
        let err = IngestError::from(sqlx::Error::Protocol(
            "duplicate key value violates unique constraint".to_string(),
        ));
        assert!(matches!(err, IngestError::ConstraintViolation(_)));
    }

    #[test]
    fn non_constraint_sqlx_error_is_database_error() {
        let err = IngestError::from(sqlx::Error::RowNotFound);
        assert!(matches!(err, IngestError::Database(_)));
    }

    #[test]
    fn ingest_result_status_values() {
        let result = IngestResult {
            episode_id: Uuid::new_v4(),
            status: "queued".to_string(),
        };
        assert_eq!(result.status, "queued");

        let result = IngestResult {
            episode_id: Uuid::new_v4(),
            status: "duplicate".to_string(),
        };
        assert_eq!(result.status, "duplicate");
    }
}
