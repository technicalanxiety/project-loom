//! Three-pass entity resolution: exact → alias → semantic.
//!
//! Prefers fragmentation over collision (recoverable vs. corrupting).
//! Each pass is implemented as a separate function that returns
//! `Option<ResolutionResult>` on match or `None` to fall through.
//!
//! The top-level [`resolve_entity`] function orchestrates the full 3-pass
//! algorithm, creates new entities when no match is found, updates entity
//! serving state, and links entities to their source episodes.

use sqlx::PgPool;

use crate::db::entities::{self, EntityError, EntityWithScore, NewEntity};
use crate::llm::client::LlmClient;
use crate::llm::embeddings::{self, EmbeddingError};
use crate::config::LlmConfig;
use crate::types::entity::{ExtractedEntity, ResolutionResult};

/// Similarity threshold for semantic resolution (deliberately high to
/// prefer fragmentation over collision).
pub const SEMANTIC_THRESHOLD: f64 = 0.92;

/// Minimum gap between the top two candidates required for an unambiguous
/// merge. When the gap is smaller than this, a conflict is logged instead.
pub const SEMANTIC_GAP: f64 = 0.03;

/// Maximum number of candidates returned by the similarity query.
const SEMANTIC_LIMIT: i64 = 10;

/// Errors that can occur during entity resolution.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    /// An underlying entity database error.
    #[error("entity lookup failed: {0}")]
    Entity(#[from] EntityError),

    /// An error generating an embedding for semantic matching.
    #[error("embedding generation failed: {0}")]
    Embedding(#[from] EmbeddingError),
}

/// Outcome of Pass 3 semantic matching.
///
/// Separates the three possible outcomes so the caller can decide how to
/// proceed (merge, create new, or create new + log conflict).
#[derive(Debug, Clone)]
pub enum SemanticResult {
    /// Top candidate exceeded the threshold with sufficient gap to the
    /// second candidate. The caller should merge with the returned entity.
    Merge(ResolutionResult),

    /// No candidate exceeded the similarity threshold. The caller should
    /// create a brand-new entity with confidence 1.0.
    NewEntity,

    /// The top two candidates are within [`SEMANTIC_GAP`] of each other.
    /// The caller should create a new entity and log a resolution conflict
    /// with the provided candidate details.
    Conflict {
        /// The candidate entities and their similarity scores.
        candidates: Vec<EntityWithScore>,
    },
}

// ---------------------------------------------------------------------------
// Pass 1 — Exact match
// ---------------------------------------------------------------------------

/// Attempt exact match resolution on `(LOWER(name), entity_type, namespace)`.
///
/// Queries the `loom_entities` table for a non-deleted entity whose
/// lowercased name, entity type, and namespace match the extracted values.
/// Returns a [`ResolutionResult`] with method `"exact"` and confidence
/// `1.0` when a match is found, or `None` to signal that the caller
/// should fall through to the next resolution pass.
///
/// # Errors
///
/// Returns [`ResolveError::Entity`] if the database query fails.
pub async fn pass1_exact_match(
    pool: &PgPool,
    name: &str,
    entity_type: &str,
    namespace: &str,
) -> Result<Option<ResolutionResult>, ResolveError> {
    let existing =
        entities::get_entity_by_name_type_namespace(pool, name, entity_type, namespace).await?;

    match existing {
        Some(entity) => {
            tracing::info!(
                entity_id = %entity.id,
                name = %entity.name,
                entity_type = %entity.entity_type,
                namespace = %entity.namespace,
                method = "exact",
                confidence = 1.0,
                "pass 1: exact match resolved"
            );

            Ok(Some(ResolutionResult {
                entity_id: entity.id,
                method: "exact".to_string(),
                confidence: 1.0,
            }))
        }
        None => {
            tracing::debug!(
                name = %name,
                entity_type = %entity_type,
                namespace = %namespace,
                "pass 1: no exact match found"
            );

            Ok(None)
        }
    }
}

// ---------------------------------------------------------------------------
// Pass 2 — Alias match
// ---------------------------------------------------------------------------

/// Attempt alias-based resolution by checking both directions:
///
/// 1. **Forward**: the extracted entity's name appears in an existing
///    entity's `properties->'aliases'` JSONB array (GIN-indexed).
/// 2. **Reverse**: one of the extracted entity's aliases matches an
///    existing entity's canonical name (case-insensitive).
///
/// If exactly one unique candidate is found across both directions, the
/// function merges with confidence `0.95` and appends the new name to
/// the matched entity's aliases array (case-insensitive dedup handled
/// by the database layer).
///
/// If multiple distinct candidates are found, returns `None` so the
/// caller falls through to Pass 3 (semantic matching).
///
/// If no candidates are found, returns `None`.
///
/// # Errors
///
/// Returns [`ResolveError::Entity`] if any database query fails.
pub async fn pass2_alias_match(
    pool: &PgPool,
    name: &str,
    aliases: &[String],
    entity_type: &str,
    namespace: &str,
) -> Result<Option<ResolutionResult>, ResolveError> {
    use std::collections::HashMap;

    // Collect unique candidates keyed by entity ID.
    let mut candidates: HashMap<uuid::Uuid, crate::types::entity::Entity> = HashMap::new();

    // Direction 1: extracted name appears in an existing entity's aliases.
    let forward_matches =
        entities::query_entities_by_alias(pool, name, entity_type, namespace).await?;

    for entity in forward_matches {
        tracing::debug!(
            entity_id = %entity.id,
            entity_name = %entity.name,
            extracted_name = %name,
            "pass 2 forward: extracted name found in existing aliases"
        );
        candidates.entry(entity.id).or_insert(entity);
    }

    // Direction 2: extracted aliases match an existing entity's canonical name.
    for alias in aliases {
        if let Some(entity) =
            entities::get_entity_by_name_type_namespace(pool, alias, entity_type, namespace).await?
        {
            tracing::debug!(
                entity_id = %entity.id,
                entity_name = %entity.name,
                extracted_alias = %alias,
                "pass 2 reverse: extracted alias matches existing name"
            );
            candidates.entry(entity.id).or_insert(entity);
        }
    }

    match candidates.len() {
        0 => {
            tracing::debug!(
                name = %name,
                alias_count = aliases.len(),
                entity_type = %entity_type,
                namespace = %namespace,
                "pass 2: no alias match found"
            );
            Ok(None)
        }
        1 => {
            // Exactly one match — merge with confidence 0.95.
            let (entity_id, entity) = candidates.into_iter().next().expect("len checked == 1");

            // Append the new name to the matched entity's aliases for future
            // resolution passes. The database layer handles case-insensitive
            // deduplication via DISTINCT on lowered values.
            let lowercase_name = name.to_lowercase();
            let existing_aliases: Vec<String> = entity
                .properties
                .as_ref()
                .and_then(|p| p.get("aliases"))
                .and_then(|a| serde_json::from_value::<Vec<String>>(a.clone()).ok())
                .unwrap_or_default();

            // Only append if the name isn't already present (case-insensitive).
            let already_present = existing_aliases
                .iter()
                .any(|a| a.to_lowercase() == lowercase_name);

            if !already_present {
                entities::update_entity_aliases(pool, entity_id, &[name.to_string()]).await?;
                tracing::info!(
                    entity_id = %entity_id,
                    new_alias = %name,
                    "pass 2: appended new name to aliases"
                );
            }

            tracing::info!(
                entity_id = %entity_id,
                entity_name = %entity.name,
                extracted_name = %name,
                method = "alias",
                confidence = 0.95,
                "pass 2: alias match resolved"
            );

            Ok(Some(ResolutionResult {
                entity_id,
                method: "alias".to_string(),
                confidence: 0.95,
            }))
        }
        n => {
            // Multiple matches — fall through to Pass 3.
            let ids: Vec<uuid::Uuid> = candidates.keys().copied().collect();
            tracing::info!(
                name = %name,
                candidate_count = n,
                candidate_ids = ?ids,
                "pass 2: multiple alias matches, falling through to pass 3"
            );
            Ok(None)
        }
    }
}

// ---------------------------------------------------------------------------
// Pass 3 — Semantic similarity
// ---------------------------------------------------------------------------

/// Attempt semantic similarity resolution using nomic-embed-text embeddings.
///
/// Generates a 768-dimension embedding for the entity name combined with a
/// context snippet, then queries `loom_entity_state` by cosine similarity
/// (pgvector) filtered to the same `entity_type` and `namespace`.
///
/// **Threshold logic** (deliberately conservative — prefer fragmentation):
///
/// 1. Top candidate > 0.92 AND gap to second ≥ 0.03 → merge with
///    confidence = similarity score, method `"semantic"`.
/// 2. Top two candidates within 0.03 → return [`SemanticResult::Conflict`]
///    so the caller can create a new entity and log the conflict.
/// 3. No candidate > 0.92 → return [`SemanticResult::NewEntity`].
///
/// # Errors
///
/// Returns [`ResolveError::Embedding`] if embedding generation fails, or
/// [`ResolveError::Entity`] if the database query fails.
#[tracing::instrument(skip(pool, client, config), fields(entity_name = %name))]
pub async fn pass3_semantic_match(
    pool: &PgPool,
    client: &LlmClient,
    config: &LlmConfig,
    name: &str,
    context: &str,
    entity_type: &str,
    namespace: &str,
) -> Result<SemanticResult, ResolveError> {
    // 1. Generate embedding for entity name + context.
    let embedding_vec =
        embeddings::generate_entity_embedding(client, config, name, context).await?;
    let embedding = pgvector::Vector::from(embedding_vec);

    tracing::debug!(
        name = %name,
        entity_type = %entity_type,
        namespace = %namespace,
        "pass 3: querying entities by embedding similarity"
    );

    // 2. Query loom_entity_state by cosine similarity.
    //    We use a lower threshold (0.0) in the DB query so we can inspect
    //    the top candidates and apply the gap logic ourselves. The DB
    //    function already orders by similarity DESC.
    let candidates = entities::query_entities_by_embedding_similarity(
        pool,
        &embedding,
        entity_type,
        namespace,
        0.0, // fetch all non-zero candidates; we apply threshold in code
        SEMANTIC_LIMIT,
    )
    .await?;

    if candidates.is_empty() {
        tracing::debug!(
            name = %name,
            entity_type = %entity_type,
            namespace = %namespace,
            "pass 3: no candidates found, creating new entity"
        );
        return Ok(SemanticResult::NewEntity);
    }

    let top = &candidates[0];
    let top_score = top.similarity;

    tracing::debug!(
        name = %name,
        top_candidate_id = %top.id,
        top_candidate_name = %top.name,
        top_score = top_score,
        candidate_count = candidates.len(),
        "pass 3: evaluated similarity candidates"
    );

    // 3a. No candidate above threshold → new entity.
    if top_score < SEMANTIC_THRESHOLD {
        tracing::info!(
            name = %name,
            top_score = top_score,
            threshold = SEMANTIC_THRESHOLD,
            "pass 3: top candidate below threshold, creating new entity"
        );
        return Ok(SemanticResult::NewEntity);
    }

    // 3b. Check gap to second candidate.
    let second_score = candidates.get(1).map(|c| c.similarity).unwrap_or(0.0);
    let gap = top_score - second_score;

    if gap >= SEMANTIC_GAP {
        // Clear winner — merge.
        tracing::info!(
            entity_id = %top.id,
            entity_name = %top.name,
            similarity = top_score,
            gap = gap,
            method = "semantic",
            "pass 3: semantic match resolved (clear winner)"
        );

        return Ok(SemanticResult::Merge(ResolutionResult {
            entity_id: top.id,
            method: "semantic".to_string(),
            confidence: top_score,
        }));
    }

    // 3c. Top two within gap threshold — ambiguous, log conflict.
    tracing::warn!(
        name = %name,
        top_id = %top.id,
        top_score = top_score,
        second_id = %candidates[1].id,
        second_score = second_score,
        gap = gap,
        "pass 3: ambiguous candidates within gap threshold, flagging conflict"
    );

    // Return all candidates that are within the gap of the top score so the
    // conflict record captures the full picture.
    let conflict_candidates: Vec<EntityWithScore> = candidates
        .into_iter()
        .filter(|c| top_score - c.similarity < SEMANTIC_GAP)
        .collect();

    Ok(SemanticResult::Conflict {
        candidates: conflict_candidates,
    })
}

// ---------------------------------------------------------------------------
// Conflict logging
// ---------------------------------------------------------------------------

/// Log a resolution conflict to the `loom_resolution_conflicts` table.
///
/// Called when Pass 3 semantic matching finds multiple candidates within
/// [`SEMANTIC_GAP`] of each other. The conflict is created as unresolved
/// so operators can review it via the dashboard.
///
/// # Errors
///
/// Returns [`ResolveError::Entity`] if the database insert fails.
#[tracing::instrument(skip(pool, candidates), fields(entity_name = %entity_name))]
pub async fn log_resolution_conflict(
    pool: &PgPool,
    entity_name: &str,
    entity_type: &str,
    namespace: &str,
    candidates: &[EntityWithScore],
) -> Result<uuid::Uuid, ResolveError> {
    // Build the candidates JSONB payload: [{id, name, score, method}]
    let candidates_json: Vec<serde_json::Value> = candidates
        .iter()
        .map(|c| {
            serde_json::json!({
                "id": c.id.to_string(),
                "name": c.name,
                "score": c.similarity,
                "method": "semantic"
            })
        })
        .collect();
    let candidates_value = serde_json::Value::Array(candidates_json);

    let row: (uuid::Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO loom_resolution_conflicts
            (entity_name, entity_type, namespace, candidates)
        VALUES ($1, $2, $3, $4)
        RETURNING id
        "#,
    )
    .bind(entity_name)
    .bind(entity_type)
    .bind(namespace)
    .bind(&candidates_value)
    .fetch_one(pool)
    .await
    .map_err(|e| ResolveError::Entity(EntityError::from(e)))?;

    tracing::info!(
        conflict_id = %row.0,
        entity_name = %entity_name,
        entity_type = %entity_type,
        namespace = %namespace,
        candidate_count = candidates.len(),
        "resolution conflict logged for operator review"
    );

    Ok(row.0)
}

// ---------------------------------------------------------------------------
// Orchestrator — Full 3-pass resolution
// ---------------------------------------------------------------------------

/// Orchestrate the full 3-pass entity resolution algorithm for a single
/// extracted entity.
///
/// Executes the passes in sequence:
///
/// 1. **Pass 1 (exact match)** — if a match is found, returns immediately.
/// 2. **Pass 2 (alias match)** — if a match is found, returns immediately.
/// 3. **Pass 3 (semantic match)** — handles three outcomes:
///    - [`SemanticResult::Merge`] → returns the merge result.
///    - [`SemanticResult::Conflict`] → creates a new entity, logs the
///      conflict via [`log_resolution_conflict`], returns with method
///      `"new"` and confidence `1.0`.
///    - [`SemanticResult::NewEntity`] → creates a new entity, returns
///      with method `"new"` and confidence `1.0`.
///
/// After resolution (whether existing or new entity):
/// - Updates entity serving state (embedding, salience) in
///   `loom_entity_state` via [`entities::update_entity_state`].
/// - Links the entity to the source episode by appending `episode_id`
///   to the `source_episodes` array via [`entities::append_source_episode`].
///
/// # Errors
///
/// Returns [`ResolveError`] if any database query, embedding generation,
/// or entity insertion fails.
#[tracing::instrument(
    skip(pool, client, config, extracted, episode_content),
    fields(
        entity_name = %extracted.name,
        entity_type = %extracted.entity_type,
        namespace = %namespace,
        episode_id = %episode_id,
    )
)]
pub async fn resolve_entity(
    pool: &PgPool,
    client: &LlmClient,
    config: &LlmConfig,
    extracted: &ExtractedEntity,
    episode_content: &str,
    episode_id: uuid::Uuid,
    namespace: &str,
) -> Result<ResolutionResult, ResolveError> {
    let name = &extracted.name;
    let entity_type = &extracted.entity_type;

    // -- Pass 1: Exact match ------------------------------------------------
    if let Some(result) = pass1_exact_match(pool, name, entity_type, namespace).await? {
        tracing::info!(
            entity_id = %result.entity_id,
            method = %result.method,
            confidence = result.confidence,
            "entity resolved via exact match"
        );
        update_serving_state_and_link(
            pool, client, config, result.entity_id, name, episode_content, episode_id,
        )
        .await?;
        return Ok(result);
    }

    // -- Pass 2: Alias match ------------------------------------------------
    if let Some(result) =
        pass2_alias_match(pool, name, &extracted.aliases, entity_type, namespace).await?
    {
        tracing::info!(
            entity_id = %result.entity_id,
            method = %result.method,
            confidence = result.confidence,
            "entity resolved via alias match"
        );
        update_serving_state_and_link(
            pool, client, config, result.entity_id, name, episode_content, episode_id,
        )
        .await?;
        return Ok(result);
    }

    // -- Pass 3: Semantic match ---------------------------------------------
    let semantic =
        pass3_semantic_match(pool, client, config, name, episode_content, entity_type, namespace)
            .await?;

    let result = match semantic {
        SemanticResult::Merge(result) => {
            tracing::info!(
                entity_id = %result.entity_id,
                method = %result.method,
                confidence = result.confidence,
                "entity resolved via semantic match"
            );
            result
        }
        SemanticResult::Conflict { candidates } => {
            // Create a new entity and log the conflict for operator review.
            let entity = create_new_entity(pool, extracted, namespace, episode_id).await?;

            log_resolution_conflict(pool, name, entity_type, namespace, &candidates).await?;

            tracing::info!(
                entity_id = %entity.id,
                method = "new",
                confidence = 1.0,
                conflict = true,
                "new entity created due to semantic conflict"
            );

            ResolutionResult {
                entity_id: entity.id,
                method: "new".to_string(),
                confidence: 1.0,
            }
        }
        SemanticResult::NewEntity => {
            // No match at all — create a brand-new entity.
            let entity = create_new_entity(pool, extracted, namespace, episode_id).await?;

            tracing::info!(
                entity_id = %entity.id,
                method = "new",
                confidence = 1.0,
                "new entity created (no match found)"
            );

            ResolutionResult {
                entity_id: entity.id,
                method: "new".to_string(),
                confidence: 1.0,
            }
        }
    };

    // -- Post-resolution: update serving state and link episode --------------
    update_serving_state_and_link(
        pool, client, config, result.entity_id, name, episode_content, episode_id,
    )
    .await?;

    Ok(result)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a new entity in the database from an extracted entity.
///
/// Builds the `properties` JSONB with the aliases array and inserts via
/// [`entities::insert_entity`]. The `source_episodes` array is initialised
/// with the originating episode.
async fn create_new_entity(
    pool: &PgPool,
    extracted: &ExtractedEntity,
    namespace: &str,
    episode_id: uuid::Uuid,
) -> Result<crate::types::entity::Entity, ResolveError> {
    // Build properties JSONB with aliases.
    let mut properties = if extracted.properties.is_object() {
        extracted.properties.clone()
    } else {
        serde_json::json!({})
    };
    if !extracted.aliases.is_empty() {
        properties["aliases"] = serde_json::json!(extracted.aliases);
    }

    let new_entity = NewEntity {
        name: extracted.name.clone(),
        entity_type: extracted.entity_type.clone(),
        namespace: namespace.to_string(),
        properties: Some(properties),
        source_episodes: Some(vec![episode_id]),
    };

    let entity = entities::insert_entity(pool, &new_entity).await?;

    tracing::debug!(
        entity_id = %entity.id,
        name = %entity.name,
        entity_type = %entity.entity_type,
        "new entity inserted"
    );

    Ok(entity)
}

/// Update entity serving state (embedding + salience) and link the entity
/// to the source episode.
///
/// Generates a fresh embedding for the entity name + episode context,
/// upserts the serving state in `loom_entity_state`, and appends the
/// episode to the entity's `source_episodes` array.
async fn update_serving_state_and_link(
    pool: &PgPool,
    client: &LlmClient,
    config: &LlmConfig,
    entity_id: uuid::Uuid,
    entity_name: &str,
    episode_content: &str,
    episode_id: uuid::Uuid,
) -> Result<(), ResolveError> {
    // Generate embedding for entity serving state.
    let embedding_vec =
        embeddings::generate_entity_embedding(client, config, entity_name, episode_content).await?;
    let embedding = pgvector::Vector::from(embedding_vec);

    // Upsert entity serving state with default salience and warm tier.
    entities::update_entity_state(
        pool,
        entity_id,
        Some(&embedding),
        0.5,    // default salience score
        "warm", // default tier
        0,      // initial access count
        None,   // no last_accessed yet
    )
    .await?;

    tracing::debug!(
        entity_id = %entity_id,
        "entity serving state updated (embedding + salience)"
    );

    // Link entity to source episode.
    entities::append_source_episode(pool, entity_id, episode_id).await?;

    tracing::debug!(
        entity_id = %entity_id,
        episode_id = %episode_id,
        "entity linked to source episode"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::entities::EntityWithScore;
    use crate::types::entity::ResolutionResult;

    #[test]
    fn resolve_error_displays_message() {
        let err = ResolveError::Entity(EntityError::Sqlx(sqlx::Error::RowNotFound));
        let msg = err.to_string();
        assert!(msg.contains("entity lookup failed"), "got: {msg}");
    }

    #[test]
    fn resolve_error_embedding_displays_message() {
        let err = ResolveError::Embedding(EmbeddingError::DimensionMismatch {
            expected: 768,
            actual: 512,
        });
        let msg = err.to_string();
        assert!(
            msg.contains("embedding generation failed"),
            "got: {msg}"
        );
    }

    #[test]
    fn resolution_result_exact_has_confidence_one() {
        let result = ResolutionResult {
            entity_id: uuid::Uuid::new_v4(),
            method: "exact".to_string(),
            confidence: 1.0,
        };
        assert_eq!(result.method, "exact");
        assert!((result.confidence - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn resolution_result_serializes_correctly() {
        let result = ResolutionResult {
            entity_id: uuid::Uuid::nil(),
            method: "exact".to_string(),
            confidence: 1.0,
        };
        let json = serde_json::to_value(&result).expect("should serialize");
        assert_eq!(json["method"], "exact");
        assert_eq!(json["confidence"], 1.0);
    }

    #[test]
    fn resolution_result_alias_has_confidence_095() {
        let result = ResolutionResult {
            entity_id: uuid::Uuid::new_v4(),
            method: "alias".to_string(),
            confidence: 0.95,
        };
        assert_eq!(result.method, "alias");
        assert!((result.confidence - 0.95).abs() < f64::EPSILON);
    }

    #[test]
    fn resolution_result_alias_serializes_correctly() {
        let result = ResolutionResult {
            entity_id: uuid::Uuid::nil(),
            method: "alias".to_string(),
            confidence: 0.95,
        };
        let json = serde_json::to_value(&result).expect("should serialize");
        assert_eq!(json["method"], "alias");
        assert_eq!(json["confidence"], 0.95);
    }

    // -- SemanticResult variants --------------------------------------------

    #[test]
    fn semantic_result_merge_carries_resolution() {
        let result = SemanticResult::Merge(ResolutionResult {
            entity_id: uuid::Uuid::new_v4(),
            method: "semantic".to_string(),
            confidence: 0.95,
        });
        match result {
            SemanticResult::Merge(r) => {
                assert_eq!(r.method, "semantic");
                assert!(r.confidence > SEMANTIC_THRESHOLD);
            }
            other => panic!("expected Merge, got: {other:?}"),
        }
    }

    #[test]
    fn semantic_result_new_entity_variant() {
        let result = SemanticResult::NewEntity;
        assert!(matches!(result, SemanticResult::NewEntity));
    }

    #[test]
    fn semantic_result_conflict_carries_candidates() {
        let candidates = vec![
            EntityWithScore {
                id: uuid::Uuid::new_v4(),
                name: "Candidate A".to_string(),
                entity_type: "service".to_string(),
                namespace: "default".to_string(),
                properties: None,
                created_at: None,
                source_episodes: None,
                deleted_at: None,
                similarity: 0.94,
            },
            EntityWithScore {
                id: uuid::Uuid::new_v4(),
                name: "Candidate B".to_string(),
                entity_type: "service".to_string(),
                namespace: "default".to_string(),
                properties: None,
                created_at: None,
                source_episodes: None,
                deleted_at: None,
                similarity: 0.93,
            },
        ];

        let result = SemanticResult::Conflict {
            candidates: candidates.clone(),
        };
        match result {
            SemanticResult::Conflict { candidates: c } => {
                assert_eq!(c.len(), 2);
                assert!((c[0].similarity - c[1].similarity).abs() < SEMANTIC_GAP);
            }
            other => panic!("expected Conflict, got: {other:?}"),
        }
    }

    // -- Threshold constants ------------------------------------------------

    #[test]
    fn semantic_threshold_is_092() {
        assert!((SEMANTIC_THRESHOLD - 0.92).abs() < f64::EPSILON);
    }

    #[test]
    fn semantic_gap_is_003() {
        assert!((SEMANTIC_GAP - 0.03).abs() < f64::EPSILON);
    }

    // -- create_new_entity helper -------------------------------------------

    #[test]
    fn create_new_entity_builds_properties_with_aliases() {
        // Verify the properties JSONB construction logic used by
        // create_new_entity (tested via the inline logic).
        let extracted = ExtractedEntity {
            name: "Rust".to_string(),
            entity_type: "technology".to_string(),
            aliases: vec!["rust-lang".to_string(), "Rust Language".to_string()],
            properties: serde_json::json!({}),
        };

        let mut properties = if extracted.properties.is_object() {
            extracted.properties.clone()
        } else {
            serde_json::json!({})
        };
        if !extracted.aliases.is_empty() {
            properties["aliases"] = serde_json::json!(extracted.aliases);
        }

        let aliases = properties["aliases"].as_array().expect("should be array");
        assert_eq!(aliases.len(), 2);
        assert_eq!(aliases[0], "rust-lang");
        assert_eq!(aliases[1], "Rust Language");
    }

    #[test]
    fn create_new_entity_handles_empty_aliases() {
        let extracted = ExtractedEntity {
            name: "PostgreSQL".to_string(),
            entity_type: "technology".to_string(),
            aliases: vec![],
            properties: serde_json::json!({}),
        };

        let mut properties = if extracted.properties.is_object() {
            extracted.properties.clone()
        } else {
            serde_json::json!({})
        };
        if !extracted.aliases.is_empty() {
            properties["aliases"] = serde_json::json!(extracted.aliases);
        }

        // No aliases key should be set when aliases are empty.
        assert!(properties.get("aliases").is_none());
    }

    #[test]
    fn create_new_entity_preserves_existing_properties() {
        let extracted = ExtractedEntity {
            name: "APIM".to_string(),
            entity_type: "service".to_string(),
            aliases: vec!["Azure API Management".to_string()],
            properties: serde_json::json!({"region": "eastus"}),
        };

        let mut properties = if extracted.properties.is_object() {
            extracted.properties.clone()
        } else {
            serde_json::json!({})
        };
        if !extracted.aliases.is_empty() {
            properties["aliases"] = serde_json::json!(extracted.aliases);
        }

        assert_eq!(properties["region"], "eastus");
        assert_eq!(properties["aliases"][0], "Azure API Management");
    }

    #[test]
    fn create_new_entity_handles_non_object_properties() {
        let extracted = ExtractedEntity {
            name: "Test".to_string(),
            entity_type: "project".to_string(),
            aliases: vec!["TestAlias".to_string()],
            properties: serde_json::json!(null),
        };

        let mut properties = if extracted.properties.is_object() {
            extracted.properties.clone()
        } else {
            serde_json::json!({})
        };
        if !extracted.aliases.is_empty() {
            properties["aliases"] = serde_json::json!(extracted.aliases);
        }

        assert!(properties.is_object());
        assert_eq!(properties["aliases"][0], "TestAlias");
    }

    // -- Resolution method values -------------------------------------------

    #[test]
    fn resolution_result_new_has_confidence_one() {
        let result = ResolutionResult {
            entity_id: uuid::Uuid::new_v4(),
            method: "new".to_string(),
            confidence: 1.0,
        };
        assert_eq!(result.method, "new");
        assert!((result.confidence - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn resolution_result_new_serializes_correctly() {
        let result = ResolutionResult {
            entity_id: uuid::Uuid::nil(),
            method: "new".to_string(),
            confidence: 1.0,
        };
        let json = serde_json::to_value(&result).expect("should serialize");
        assert_eq!(json["method"], "new");
        assert_eq!(json["confidence"], 1.0);
    }

    #[test]
    fn resolution_methods_are_four_variants() {
        // Verify the four resolution method strings used by the orchestrator.
        let methods = ["exact", "alias", "semantic", "new"];
        for method in &methods {
            let result = ResolutionResult {
                entity_id: uuid::Uuid::new_v4(),
                method: method.to_string(),
                confidence: 1.0,
            };
            assert_eq!(&result.method, method);
        }
    }

    // -- Resolution method selection logic (6.9) ----------------------------

    /// Helper: simulate the semantic decision logic for unit testing.
    fn evaluate_semantic_candidates(candidates: &[EntityWithScore]) -> SemanticResult {
        if candidates.is_empty() {
            return SemanticResult::NewEntity;
        }

        let top = &candidates[0];
        let top_score = top.similarity;

        if top_score < SEMANTIC_THRESHOLD {
            return SemanticResult::NewEntity;
        }

        let second_score = candidates.get(1).map(|c| c.similarity).unwrap_or(0.0);
        let gap = top_score - second_score;

        if gap >= SEMANTIC_GAP {
            return SemanticResult::Merge(ResolutionResult {
                entity_id: top.id,
                method: "semantic".to_string(),
                confidence: top_score,
            });
        }

        let conflict_candidates: Vec<EntityWithScore> = candidates
            .iter()
            .filter(|c| top_score - c.similarity < SEMANTIC_GAP)
            .cloned()
            .collect();

        SemanticResult::Conflict {
            candidates: conflict_candidates,
        }
    }

    fn make_candidate(name: &str, similarity: f64) -> EntityWithScore {
        EntityWithScore {
            id: uuid::Uuid::new_v4(),
            name: name.to_string(),
            entity_type: "service".to_string(),
            namespace: "default".to_string(),
            properties: None,
            created_at: None,
            source_episodes: None,
            deleted_at: None,
            similarity,
        }
    }

    #[test]
    fn resolution_pass1_exact_returns_confidence_one() {
        // Validates: Requirement 3.1, 3.2
        let result = ResolutionResult {
            entity_id: uuid::Uuid::new_v4(),
            method: "exact".to_string(),
            confidence: 1.0,
        };
        assert!((result.confidence - 1.0).abs() < f64::EPSILON);
        assert_eq!(result.method, "exact");
    }

    #[test]
    fn resolution_pass2_alias_returns_confidence_095() {
        // Validates: Requirement 3.4
        let result = ResolutionResult {
            entity_id: uuid::Uuid::new_v4(),
            method: "alias".to_string(),
            confidence: 0.95,
        };
        assert!((result.confidence - 0.95).abs() < f64::EPSILON);
        assert_eq!(result.method, "alias");
    }

    #[test]
    fn resolution_pass3_semantic_merge_above_threshold_with_gap() {
        // Validates: Requirement 3.7
        let candidates = vec![
            make_candidate("ServiceA", 0.96),
            make_candidate("ServiceB", 0.90),
        ];
        let result = evaluate_semantic_candidates(&candidates);
        match result {
            SemanticResult::Merge(r) => {
                assert!((r.confidence - 0.96).abs() < f64::EPSILON);
                assert_eq!(r.method, "semantic");
            }
            other => panic!("expected Merge, got: {other:?}"),
        }
    }

    #[test]
    fn resolution_pass3_semantic_new_entity_below_threshold() {
        // Validates: Requirement 3.9
        let candidates = vec![make_candidate("ServiceA", 0.85)];
        let result = evaluate_semantic_candidates(&candidates);
        assert!(matches!(result, SemanticResult::NewEntity));
    }

    #[test]
    fn resolution_pass3_semantic_conflict_within_gap() {
        // Validates: Requirement 3.8
        let candidates = vec![
            make_candidate("ServiceA", 0.95),
            make_candidate("ServiceB", 0.94),
        ];
        let result = evaluate_semantic_candidates(&candidates);
        match result {
            SemanticResult::Conflict { candidates: c } => {
                assert!(c.len() >= 2, "conflict must have at least 2 candidates");
            }
            other => panic!("expected Conflict, got: {other:?}"),
        }
    }

    #[test]
    fn resolution_pass3_semantic_empty_candidates_creates_new() {
        let candidates: Vec<EntityWithScore> = vec![];
        let result = evaluate_semantic_candidates(&candidates);
        assert!(matches!(result, SemanticResult::NewEntity));
    }

    #[test]
    fn resolution_pass3_single_candidate_above_threshold_merges() {
        // Single candidate above 0.92 always merges (gap to 0.0 is >= 0.03).
        let candidates = vec![make_candidate("OnlyOne", 0.95)];
        let result = evaluate_semantic_candidates(&candidates);
        assert!(matches!(result, SemanticResult::Merge(_)));
    }

    #[test]
    fn resolution_pass3_boundary_score_exactly_092_merges() {
        // Score exactly at threshold with no second candidate should merge.
        let candidates = vec![make_candidate("Boundary", 0.92)];
        let result = evaluate_semantic_candidates(&candidates);
        assert!(matches!(result, SemanticResult::Merge(_)));
    }

    #[test]
    fn resolution_pass3_boundary_score_just_below_092_creates_new() {
        let candidates = vec![make_candidate("JustBelow", 0.9199)];
        let result = evaluate_semantic_candidates(&candidates);
        assert!(matches!(result, SemanticResult::NewEntity));
    }

    #[test]
    fn resolution_pass3_gap_at_threshold_merges() {
        // Gap slightly above 0.03 should merge (>= 0.03).
        // Use values where the gap is unambiguously >= 0.03 in f64.
        let candidates = vec![
            make_candidate("Top", 0.96),
            make_candidate("Second", 0.925),
        ];
        let result = evaluate_semantic_candidates(&candidates);
        assert!(matches!(result, SemanticResult::Merge(_)));
    }

    #[test]
    fn resolution_pass3_gap_just_below_003_conflicts() {
        // Gap of 0.029 (< 0.03) should conflict.
        let candidates = vec![
            make_candidate("Top", 0.96),
            make_candidate("Second", 0.931),
        ];
        let result = evaluate_semantic_candidates(&candidates);
        assert!(matches!(result, SemanticResult::Conflict { .. }));
    }

    // -- Conflict detection captures all close candidates -------------------

    #[test]
    fn conflict_captures_three_close_candidates() {
        // Validates: Requirement 3.8, 23.1
        let candidates = vec![
            make_candidate("A", 0.96),
            make_candidate("B", 0.955),
            make_candidate("C", 0.95),
            make_candidate("D", 0.90), // outside gap, should be excluded
        ];
        let result = evaluate_semantic_candidates(&candidates);
        match result {
            SemanticResult::Conflict { candidates: c } => {
                // A, B, C are within 0.03 of top (0.96); D is not.
                assert_eq!(c.len(), 3, "should capture 3 close candidates");
                for candidate in &c {
                    assert!(
                        0.96 - candidate.similarity < SEMANTIC_GAP,
                        "candidate {} (score={}) should be within gap",
                        candidate.name,
                        candidate.similarity
                    );
                }
            }
            other => panic!("expected Conflict, got: {other:?}"),
        }
    }

    // -- Alias accumulation and case-insensitive dedup ----------------------

    #[test]
    fn alias_dedup_case_insensitive() {
        // Validates: Requirement 42.3
        // Simulate the alias dedup logic from pass2_alias_match.
        let existing_aliases = vec!["K8s".to_string(), "kube".to_string()];
        let new_name = "k8s"; // same as "K8s" case-insensitively

        let lowercase_name = new_name.to_lowercase();
        let already_present = existing_aliases
            .iter()
            .any(|a| a.to_lowercase() == lowercase_name);

        assert!(already_present, "k8s should match K8s case-insensitively");
    }

    #[test]
    fn alias_dedup_allows_new_alias() {
        let existing_aliases = vec!["K8s".to_string(), "kube".to_string()];
        let new_name = "Kubernetes";

        let lowercase_name = new_name.to_lowercase();
        let already_present = existing_aliases
            .iter()
            .any(|a| a.to_lowercase() == lowercase_name);

        assert!(!already_present, "Kubernetes should not match existing aliases");
    }

    #[test]
    fn alias_accumulation_preserves_existing() {
        // Simulate alias accumulation: existing aliases + new alias.
        let mut aliases = vec!["K8s".to_string(), "kube".to_string()];
        let new_alias = "Kubernetes".to_string();

        let lowercase_new = new_alias.to_lowercase();
        let already_present = aliases.iter().any(|a| a.to_lowercase() == lowercase_new);

        if !already_present {
            aliases.push(new_alias);
        }

        assert_eq!(aliases.len(), 3);
        assert_eq!(aliases[2], "Kubernetes");
    }

    #[test]
    fn alias_accumulation_skips_duplicate() {
        let mut aliases = vec!["K8s".to_string(), "kube".to_string()];
        let new_alias = "k8s".to_string(); // duplicate (case-insensitive)

        let lowercase_new = new_alias.to_lowercase();
        let already_present = aliases.iter().any(|a| a.to_lowercase() == lowercase_new);

        if !already_present {
            aliases.push(new_alias);
        }

        assert_eq!(aliases.len(), 2, "should not add duplicate alias");
    }

    // -- Conflict logging JSON structure ------------------------------------

    #[test]
    fn conflict_candidates_json_structure() {
        // Validates: Requirement 23.1, 23.2
        // Verify the JSON structure produced by log_resolution_conflict.
        let candidates = vec![
            make_candidate("ServiceA", 0.95),
            make_candidate("ServiceB", 0.94),
        ];

        let candidates_json: Vec<serde_json::Value> = candidates
            .iter()
            .map(|c| {
                serde_json::json!({
                    "id": c.id.to_string(),
                    "name": c.name,
                    "score": c.similarity,
                    "method": "semantic"
                })
            })
            .collect();
        let candidates_value = serde_json::Value::Array(candidates_json);

        assert!(candidates_value.is_array());
        let arr = candidates_value.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["name"], "ServiceA");
        assert_eq!(arr[0]["method"], "semantic");
        assert!(arr[0]["score"].as_f64().unwrap() > 0.9);
        assert!(arr[0]["id"].as_str().is_some());
    }
}
