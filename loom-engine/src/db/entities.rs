//! Database queries for the `loom_entities` and `loom_entity_state` tables.
//!
//! Provides entity CRUD operations, three-pass resolution queries (exact match,
//! alias lookup, embedding similarity), alias management, and entity state
//! upserts. All functions accept a `&PgPool` reference and return
//! `Result<T, EntityError>`.

use chrono::{DateTime, Utc};
use pgvector::Vector;
use sqlx::PgPool;
use uuid::Uuid;

use crate::types::entity::{Entity, EntityState};

/// Errors that can occur during entity database operations.
#[derive(Debug, thiserror::Error)]
pub enum EntityError {
    /// An underlying sqlx / PostgreSQL error.
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
}

/// Data required to insert a new entity.
///
/// Mirrors the caller-provided columns of `loom_entities`. Derived columns
/// (created_at) are set by the database.
#[derive(Debug)]
pub struct NewEntity {
    /// Most specific common name.
    pub name: String,
    /// Constrained entity type (one of 10 types).
    pub entity_type: String,
    /// Namespace isolation boundary.
    pub namespace: String,
    /// Flexible properties including aliases array (JSONB).
    pub properties: Option<serde_json::Value>,
    /// Which episodes mentioned this entity.
    pub source_episodes: Option<Vec<Uuid>>,
}

/// An entity row joined with its similarity score from a vector search.
///
/// Used by `query_entities_by_embedding_similarity` to return ranked results.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EntityWithScore {
    /// Unique entity identifier.
    pub id: Uuid,
    /// Most specific common name.
    pub name: String,
    /// Constrained entity type.
    pub entity_type: String,
    /// Namespace isolation boundary.
    pub namespace: String,
    /// Flexible properties including aliases array (JSONB).
    pub properties: Option<serde_json::Value>,
    /// When the entity was created.
    pub created_at: Option<DateTime<Utc>>,
    /// Which episodes mentioned this entity.
    pub source_episodes: Option<Vec<Uuid>>,
    /// Soft-delete timestamp.
    pub deleted_at: Option<DateTime<Utc>>,
    /// Cosine similarity score (1.0 = identical, 0.0 = orthogonal).
    pub similarity: f64,
}

// ---------------------------------------------------------------------------
// Insert
// ---------------------------------------------------------------------------

/// Insert a new entity with unique constraint handling.
///
/// If an entity with the same `(name, entity_type, namespace)` already exists,
/// the existing row is returned instead of failing. This mirrors the
/// idempotent insert pattern used for episodes.
pub async fn insert_entity(
    pool: &PgPool,
    entity: &NewEntity,
) -> Result<Entity, EntityError> {
    // Check for existing entity by unique constraint columns.
    let existing = get_entity_by_name_type_namespace(
        pool,
        &entity.name,
        &entity.entity_type,
        &entity.namespace,
    )
    .await?;

    if let Some(row) = existing {
        return Ok(row);
    }

    let row = sqlx::query_as::<_, Entity>(
        r#"
        INSERT INTO loom_entities (name, entity_type, namespace, properties, source_episodes)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING *
        "#,
    )
    .bind(&entity.name)
    .bind(&entity.entity_type)
    .bind(&entity.namespace)
    .bind(&entity.properties)
    .bind(&entity.source_episodes)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

// ---------------------------------------------------------------------------
// Read
// ---------------------------------------------------------------------------

/// Fetch a single entity by its UUID.
///
/// Returns `None` if no entity with the given id exists.
pub async fn get_entity_by_id(
    pool: &PgPool,
    id: Uuid,
) -> Result<Option<Entity>, EntityError> {
    let row = sqlx::query_as::<_, Entity>(
        "SELECT * FROM loom_entities WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

/// Exact match on LOWER(name), entity_type, and namespace.
///
/// Filters out soft-deleted entities. Used as Pass 1 of the three-pass
/// entity resolution algorithm.
pub async fn get_entity_by_name_type_namespace(
    pool: &PgPool,
    name: &str,
    entity_type: &str,
    namespace: &str,
) -> Result<Option<Entity>, EntityError> {
    let row = sqlx::query_as::<_, Entity>(
        r#"
        SELECT *
        FROM loom_entities
        WHERE LOWER(name) = LOWER($1)
          AND entity_type = $2
          AND namespace = $3
          AND deleted_at IS NULL
        "#,
    )
    .bind(name)
    .bind(entity_type)
    .bind(namespace)
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

/// Query entities where a given alias appears in the properties->'aliases' JSONB array.
///
/// Uses the GIN index on `(properties->'aliases')` via the `@>` containment
/// operator. Filters by entity_type and namespace, and excludes soft-deleted
/// entities. Used as Pass 2 of the three-pass entity resolution algorithm.
pub async fn query_entities_by_alias(
    pool: &PgPool,
    alias: &str,
    entity_type: &str,
    namespace: &str,
) -> Result<Vec<Entity>, EntityError> {
    // Build a JSON array containing the alias for the @> containment check.
    let alias_json = serde_json::json!([alias]);

    let rows = sqlx::query_as::<_, Entity>(
        r#"
        SELECT *
        FROM loom_entities
        WHERE properties->'aliases' @> $1::jsonb
          AND entity_type = $2
          AND namespace = $3
          AND deleted_at IS NULL
        "#,
    )
    .bind(&alias_json)
    .bind(entity_type)
    .bind(namespace)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Query entities by embedding cosine similarity using pgvector.
///
/// Joins `loom_entity_state` with `loom_entities` and ranks by cosine
/// similarity using the `<=>` operator. Filters by entity_type, namespace,
/// and a minimum similarity threshold. Excludes soft-deleted entities.
/// Used as Pass 3 of the three-pass entity resolution algorithm.
pub async fn query_entities_by_embedding_similarity(
    pool: &PgPool,
    embedding: &Vector,
    entity_type: &str,
    namespace: &str,
    threshold: f64,
    limit: i64,
) -> Result<Vec<EntityWithScore>, EntityError> {
    let rows = sqlx::query_as::<_, EntityWithScore>(
        r#"
        SELECT e.*,
               1.0 - (es.embedding <=> $1::vector) AS similarity
        FROM loom_entities e
        JOIN loom_entity_state es ON es.entity_id = e.id
        WHERE e.entity_type = $2
          AND e.namespace = $3
          AND e.deleted_at IS NULL
          AND es.embedding IS NOT NULL
          AND 1.0 - (es.embedding <=> $1::vector) >= $4
        ORDER BY es.embedding <=> $1::vector ASC
        LIMIT $5
        "#,
    )
    .bind(embedding)
    .bind(entity_type)
    .bind(namespace)
    .bind(threshold)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

// ---------------------------------------------------------------------------
// Update
// ---------------------------------------------------------------------------

/// Append new aliases to the properties->'aliases' JSONB array for a given entity.
///
/// Merges the new aliases with any existing aliases using `COALESCE` and
/// `jsonb_set`. Deduplication is handled by converting to a set via
/// `jsonb_array_elements_text` and `DISTINCT`.
pub async fn update_entity_aliases(
    pool: &PgPool,
    entity_id: Uuid,
    new_aliases: &[String],
) -> Result<Entity, EntityError> {
    let aliases_json = serde_json::json!(new_aliases);

    let row = sqlx::query_as::<_, Entity>(
        r#"
        UPDATE loom_entities
        SET properties = jsonb_set(
            COALESCE(properties, '{}'),
            '{aliases}',
            (
                SELECT COALESCE(jsonb_agg(DISTINCT alias), '[]'::jsonb)
                FROM (
                    SELECT jsonb_array_elements_text(
                        COALESCE(properties->'aliases', '[]'::jsonb)
                    ) AS alias
                    UNION
                    SELECT jsonb_array_elements_text($2::jsonb) AS alias
                ) combined
            )
        )
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(entity_id)
    .bind(&aliases_json)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

/// Upsert entity serving state into `loom_entity_state`.
///
/// Inserts a new row or updates the existing row on conflict with
/// `entity_id`. Sets embedding, salience_score, tier, access_count,
/// last_accessed, and updated_at.
pub async fn update_entity_state(
    pool: &PgPool,
    entity_id: Uuid,
    embedding: Option<&Vector>,
    salience_score: f64,
    tier: &str,
    access_count: i32,
    last_accessed: Option<DateTime<Utc>>,
) -> Result<EntityState, EntityError> {
    let row = sqlx::query_as::<_, EntityState>(
        r#"
        INSERT INTO loom_entity_state (
            entity_id, embedding, salience_score, tier,
            access_count, last_accessed, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, now())
        ON CONFLICT (entity_id) DO UPDATE SET
            embedding = COALESCE($2, loom_entity_state.embedding),
            salience_score = $3,
            tier = $4,
            access_count = $5,
            last_accessed = $6,
            updated_at = now()
        RETURNING *
        "#,
    )
    .bind(entity_id)
    .bind(embedding)
    .bind(salience_score)
    .bind(tier)
    .bind(access_count)
    .bind(last_accessed)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

/// Append an episode UUID to the entity's source_episodes array.
///
/// Uses `array_append` with `COALESCE` to handle the case where
/// source_episodes is NULL.
pub async fn append_source_episode(
    pool: &PgPool,
    entity_id: Uuid,
    episode_id: Uuid,
) -> Result<Entity, EntityError> {
    let row = sqlx::query_as::<_, Entity>(
        r#"
        UPDATE loom_entities
        SET source_episodes = array_append(
            COALESCE(source_episodes, ARRAY[]::uuid[]),
            $2
        )
        WHERE id = $1
        RETURNING *
        "#,
    )
    .bind(entity_id)
    .bind(episode_id)
    .fetch_one(pool)
    .await?;

    Ok(row)
}

// ---------------------------------------------------------------------------
// Soft delete
// ---------------------------------------------------------------------------

/// Soft-delete an entity by setting `deleted_at` to the current time.
///
/// The row remains in the database but is excluded from normal queries
/// (all retrieval functions filter `WHERE deleted_at IS NULL`). Returns
/// the updated `Entity` row.
pub async fn soft_delete_entity(
    pool: &PgPool,
    id: Uuid,
    deletion_reason: Option<&str>,
) -> Result<Entity, EntityError> {
    // We store the deletion reason in the properties JSONB field since
    // loom_entities doesn't have a dedicated deletion_reason column.
    let row = if let Some(reason) = deletion_reason {
        sqlx::query_as::<_, Entity>(
            r#"
            UPDATE loom_entities
            SET deleted_at = now(),
                properties = jsonb_set(
                    COALESCE(properties, '{}'),
                    '{deletion_reason}',
                    to_jsonb($2::text)
                )
            WHERE id = $1
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(reason)
        .fetch_one(pool)
        .await?
    } else {
        sqlx::query_as::<_, Entity>(
            r#"
            UPDATE loom_entities
            SET deleted_at = now()
            WHERE id = $1
            RETURNING *
            "#,
        )
        .bind(id)
        .fetch_one(pool)
        .await?
    };

    Ok(row)
}

/// Query soft-deleted entities for audit purposes.
///
/// Returns entities where `deleted_at IS NOT NULL` in the given namespace,
/// ordered by deletion time descending.
pub async fn query_deleted_entities(
    pool: &PgPool,
    namespace: &str,
    limit: i64,
) -> Result<Vec<Entity>, EntityError> {
    let rows = sqlx::query_as::<_, Entity>(
        r#"
        SELECT *
        FROM loom_entities
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
    fn entity_error_displays_message() {
        let err = EntityError::Sqlx(sqlx::Error::RowNotFound);
        assert!(err.to_string().contains("database error"));
    }

    #[test]
    fn new_entity_debug_format() {
        let entity = NewEntity {
            name: "test".to_string(),
            entity_type: "person".to_string(),
            namespace: "default".to_string(),
            properties: None,
            source_episodes: None,
        };
        let debug = format!("{entity:?}");
        assert!(debug.contains("test"));
    }

    #[test]
    fn entity_with_score_has_similarity() {
        let score = EntityWithScore {
            id: Uuid::new_v4(),
            name: "test".to_string(),
            entity_type: "person".to_string(),
            namespace: "default".to_string(),
            properties: None,
            created_at: None,
            source_episodes: None,
            deleted_at: None,
            similarity: 0.95,
        };
        assert!(score.similarity > 0.9);
    }
}
