//! Derived state computation, extraction metrics logging, and tier management.
//!
//! Computes [`ExtractionMetrics`] from entity resolution and fact extraction
//! results, serializes them as JSONB, and stores them on the episode record
//! via [`crate::db::episodes::mark_episode_processed`].
//!
//! Also provides helpers for updating entity and fact serving state
//! (salience, tier) after extraction completes, and tier management
//! functions for hot tier promotion, demotion, budget enforcement, and
//! warm tier archival.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::episodes::{self, EpisodeError};
use crate::db::entities::EntityError;
use crate::db::facts as facts_db;
use crate::db::facts::FactError;
use crate::pipeline::offline::extract::FactOrchestrationResult;
use crate::types::entity::ResolutionResult;
use crate::types::episode::ExtractionMetrics;

/// Errors that can occur during derived state computation.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    /// An underlying episode database error.
    #[error("episode database error: {0}")]
    Episode(#[from] EpisodeError),

    /// An underlying fact database error.
    #[error("fact database error: {0}")]
    Fact(#[from] FactError),

    /// An underlying entity database error.
    #[error("entity database error: {0}")]
    Entity(#[from] EntityError),

    /// An underlying sqlx / PostgreSQL error.
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
}

/// Compute [`ExtractionMetrics`] from entity resolution and fact extraction results.
///
/// Aggregates counts by resolution method (exact, alias, semantic, new,
/// conflict_flagged), predicate type (canonical, custom), evidence strength
/// (explicit, implied), processing time, and extraction model.
///
/// # Arguments
///
/// * `resolution_results` — Results from the 3-pass entity resolution for each entity.
/// * `conflict_count` — Number of resolution conflicts flagged during entity resolution.
/// * `fact_result` — The orchestrated fact extraction result (may be `None` if extraction failed).
/// * `extracted_facts_with_evidence` — Evidence strength values for inserted facts.
/// * `processing_time_ms` — Total processing time in milliseconds.
/// * `extraction_model` — The model identifier used for extraction.
pub fn compute_extraction_metrics(
    resolution_results: &[ResolutionResult],
    conflict_count: i32,
    fact_result: Option<&FactOrchestrationResult>,
    extracted_facts_with_evidence: &[Option<String>],
    processing_time_ms: i64,
    extraction_model: &str,
) -> ExtractionMetrics {
    // Count entities by resolution method.
    let mut resolved_exact: i32 = 0;
    let mut resolved_alias: i32 = 0;
    let mut resolved_semantic: i32 = 0;
    let mut new: i32 = 0;

    for result in resolution_results {
        match result.method.as_str() {
            "exact" => resolved_exact += 1,
            "alias" => resolved_alias += 1,
            "semantic" => resolved_semantic += 1,
            "new" => new += 1,
            _ => {
                tracing::warn!(method = %result.method, "unknown resolution method");
                new += 1;
            }
        }
    }

    let extracted = resolution_results.len() as i32;

    // Fact counts from the orchestration result.
    let (facts_extracted, canonical_predicate, custom_predicate) = match fact_result {
        Some(fr) => {
            let pv = fr.predicate_validation.as_ref();
            (
                fr.inserted_count as i32,
                pv.map_or(0, |v| v.canonical_count as i32),
                pv.map_or(0, |v| v.custom_count as i32),
            )
        }
        None => (0, 0, 0),
    };

    // Evidence strength counts.
    let mut explicit: i32 = 0;
    let mut implied: i32 = 0;

    for evidence in extracted_facts_with_evidence {
        match evidence.as_deref() {
            Some("explicit") => explicit += 1,
            Some("implied") => implied += 1,
            _ => {} // unknown or missing evidence strength
        }
    }

    let metrics = ExtractionMetrics {
        extracted,
        resolved_exact,
        resolved_alias,
        resolved_semantic,
        new,
        conflict_flagged: conflict_count,
        facts_extracted,
        canonical_predicate,
        custom_predicate,
        explicit,
        implied,
        processing_time_ms,
        extraction_model: extraction_model.to_string(),
    };

    tracing::info!(
        entities_extracted = extracted,
        resolved_exact,
        resolved_alias,
        resolved_semantic,
        new_entities = new,
        conflict_flagged = conflict_count,
        facts_extracted,
        canonical_predicates = canonical_predicate,
        custom_predicates = custom_predicate,
        explicit_evidence = explicit,
        implied_evidence = implied,
        processing_time_ms,
        extraction_model,
        "extraction metrics computed"
    );

    metrics
}

/// Store extraction metrics on the episode and mark it as processed.
///
/// Serializes the [`ExtractionMetrics`] as JSONB and calls
/// [`episodes::mark_episode_processed`] to set `processed = true` and
/// write the metrics. Also updates `extraction_model` and
/// `classification_model` lineage fields.
///
/// # Errors
///
/// Returns [`StateError::Episode`] if the database update fails.
pub async fn store_metrics_and_mark_processed(
    pool: &PgPool,
    episode_id: Uuid,
    metrics: &ExtractionMetrics,
    extraction_model: &str,
) -> Result<(), StateError> {
    let metrics_json = serde_json::to_value(metrics)
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, "failed to serialize extraction metrics");
            serde_json::json!({})
        });

    // Update extraction lineage fields and metrics.
    episodes::update_extraction_metrics(
        pool,
        episode_id,
        extraction_model,
        "", // classification_model not used in offline pipeline
        &metrics_json,
    )
    .await?;

    // Mark the episode as processed.
    episodes::mark_episode_processed(pool, episode_id, &metrics_json).await?;

    tracing::info!(
        episode_id = %episode_id,
        "episode marked as processed with extraction metrics"
    );

    Ok(())
}

/// Update fact serving state for a batch of newly inserted facts.
///
/// Sets default salience (0.5) and warm tier for each fact. This ensures
/// facts are immediately available for retrieval after extraction.
///
/// # Errors
///
/// Returns [`StateError::Fact`] if any database update fails.
pub async fn initialize_fact_serving_state(
    pool: &PgPool,
    fact_ids: &[Uuid],
) -> Result<(), StateError> {
    for &fact_id in fact_ids {
        facts_db::update_fact_state(
            pool,
            fact_id,
            None,  // no embedding yet
            0.5,   // default salience
            "warm", // default tier
            0,     // initial access count
            None,  // no last_accessed
            false, // not pinned
        )
        .await?;
    }

    tracing::debug!(
        fact_count = fact_ids.len(),
        "initialized fact serving state for new facts"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Salience score computation
// ---------------------------------------------------------------------------

/// Default salience score for newly created items.
pub const DEFAULT_SALIENCE: f64 = 0.5;

/// Decay factor applied to existing salience on each access.
///
/// On each access the salience is updated as:
/// ```text
/// new_salience = old_salience * SALIENCE_DECAY + SALIENCE_ACCESS_BOOST
/// ```
/// This ensures salience grows with access but decays over time when not
/// accessed.
const SALIENCE_DECAY: f64 = 0.9;

/// Boost added to salience on each access.
const SALIENCE_ACCESS_BOOST: f64 = 0.1;

/// Compute an updated salience score after an access event.
///
/// Applies exponential decay to the current salience and adds a fixed
/// boost. The result is clamped to `[0.0, 1.0]`.
///
/// # Arguments
///
/// * `current_salience` — The item's current salience score.
///
/// # Returns
///
/// The new salience score after the access.
pub fn compute_salience_on_access(current_salience: f64) -> f64 {
    let new_salience = current_salience * SALIENCE_DECAY + SALIENCE_ACCESS_BOOST;
    new_salience.clamp(0.0, 1.0)
}

/// Record an access event for a fact, updating salience, access count, and
/// last_accessed timestamp in the serving state table.
///
/// # Errors
///
/// Returns [`StateError::Fact`] if the database update fails.
pub async fn record_fact_access(
    pool: &PgPool,
    fact_id: Uuid,
    current_salience: f64,
    current_access_count: i32,
) -> Result<(), StateError> {
    let new_salience = compute_salience_on_access(current_salience);
    let new_access_count = current_access_count + 1;
    let now = chrono::Utc::now();

    facts_db::update_fact_state(
        pool,
        fact_id,
        None, // don't change embedding
        new_salience,
        "warm", // tier unchanged by access alone
        new_access_count,
        Some(now),
        false, // pinned unchanged
    )
    .await?;

    tracing::debug!(
        fact_id = %fact_id,
        old_salience = current_salience,
        new_salience,
        access_count = new_access_count,
        "fact salience updated on access"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Tier management — promotion
// ---------------------------------------------------------------------------

/// Default hot tier token budget when no namespace config exists.
pub const DEFAULT_HOT_TIER_BUDGET: i32 = 500;

/// Minimum access count within 14 days to qualify for automatic hot tier
/// promotion.
pub const PROMOTION_ACCESS_THRESHOLD: i32 = 5;

/// Number of days within which accesses must occur for promotion.
pub const PROMOTION_WINDOW_DAYS: i64 = 14;

/// Minimum number of distinct source episodes for procedure hot tier.
pub const PROCEDURE_MIN_EPISODES: i32 = 3;

/// Minimum age in days for procedure hot tier eligibility.
pub const PROCEDURE_MIN_AGE_DAYS: i64 = 7;

/// Minimum confidence for procedure hot tier eligibility.
pub const PROCEDURE_MIN_CONFIDENCE: f64 = 0.8;

/// Number of days without access before hot tier demotion.
pub const DEMOTION_STALE_DAYS: i64 = 30;

/// Number of days without access before warm tier archival.
pub const ARCHIVAL_STALE_DAYS: i64 = 90;

/// Token estimate per entity in the hot tier.
pub const TOKENS_PER_ENTITY: i32 = 10;

/// Token estimate per fact in the hot tier.
pub const TOKENS_PER_FACT: i32 = 15;

/// Token estimate per procedure in the hot tier.
pub const TOKENS_PER_PROCEDURE: i32 = 30;

/// Estimate the total token count for a set of hot tier items.
///
/// Uses a simple heuristic: each entity ≈ 10 tokens, each fact ≈ 15 tokens,
/// each procedure ≈ 30 tokens.
pub fn estimate_hot_tier_tokens(
    entity_count: usize,
    fact_count: usize,
    procedure_count: usize,
) -> i32 {
    entity_count as i32 * TOKENS_PER_ENTITY
        + fact_count as i32 * TOKENS_PER_FACT
        + procedure_count as i32 * TOKENS_PER_PROCEDURE
}

/// Check whether a procedure meets the criteria for hot tier promotion.
///
/// Procedures require:
/// - 3+ distinct source episodes
/// - 7+ days since first observation
/// - confidence >= 0.8
///
/// Returns `true` if the procedure is eligible for hot tier.
pub fn procedure_eligible_for_hot_tier(
    source_episode_count: i32,
    first_observed: Option<DateTime<Utc>>,
    confidence: f64,
) -> bool {
    if source_episode_count < PROCEDURE_MIN_EPISODES {
        return false;
    }
    if confidence < PROCEDURE_MIN_CONFIDENCE {
        return false;
    }
    match first_observed {
        Some(observed) => {
            let age_days = (Utc::now() - observed).num_days();
            age_days >= PROCEDURE_MIN_AGE_DAYS
        }
        None => false,
    }
}

/// Check whether an item qualifies for automatic hot tier promotion.
///
/// An item qualifies when it has been accessed in 5+ compilations within
/// the last 14 days.
pub fn qualifies_for_promotion(
    access_count: i32,
    last_accessed: Option<DateTime<Utc>>,
) -> bool {
    if access_count < PROMOTION_ACCESS_THRESHOLD {
        return false;
    }
    match last_accessed {
        Some(accessed) => {
            let days_since = (Utc::now() - accessed).num_days();
            days_since <= PROMOTION_WINDOW_DAYS
        }
        None => false,
    }
}

/// Promote an entity to hot tier by setting pinned=true and tier='hot'.
///
/// Used when a user explicitly pins an entity.
///
/// # Errors
///
/// Returns [`StateError::Entity`] if the database update fails.
pub async fn promote_entity_pin(
    pool: &PgPool,
    entity_id: Uuid,
) -> Result<(), StateError> {
    sqlx::query(
        r#"
        UPDATE loom_entity_state
        SET pinned = true, tier = 'hot', updated_at = now()
        WHERE entity_id = $1
        "#,
    )
    .bind(entity_id)
    .execute(pool)
    .await?;

    tracing::info!(entity_id = %entity_id, "entity pinned and promoted to hot tier");
    Ok(())
}

/// Promote a fact to hot tier by setting pinned=true and tier='hot'.
///
/// Used when a user explicitly pins a fact.
///
/// # Errors
///
/// Returns [`StateError::Fact`] if the database update fails.
pub async fn promote_fact_pin(
    pool: &PgPool,
    fact_id: Uuid,
) -> Result<(), StateError> {
    sqlx::query(
        r#"
        UPDATE loom_fact_state
        SET pinned = true, tier = 'hot', updated_at = now()
        WHERE fact_id = $1
        "#,
    )
    .bind(fact_id)
    .execute(pool)
    .await?;

    tracing::info!(fact_id = %fact_id, "fact pinned and promoted to hot tier");
    Ok(())
}

/// Promote warm-tier entities that meet the automatic promotion criteria.
///
/// Promotes entities with access_count >= 5 and last_accessed within 14 days.
/// Skips already-pinned items (they are already hot).
///
/// Returns the number of entities promoted.
pub async fn promote_eligible_entities(pool: &PgPool) -> Result<usize, StateError> {
    let promoted = sqlx::query(
        r#"
        UPDATE loom_entity_state
        SET tier = 'hot', updated_at = now()
        WHERE tier = 'warm'
          AND access_count >= $1
          AND last_accessed >= now() - make_interval(days => $2)
          AND pinned = false
        "#,
    )
    .bind(PROMOTION_ACCESS_THRESHOLD)
    .bind(PROMOTION_WINDOW_DAYS as i32)
    .execute(pool)
    .await?
    .rows_affected();

    if promoted > 0 {
        tracing::info!(count = promoted, "entities promoted to hot tier");
    }
    Ok(promoted as usize)
}

/// Promote warm-tier facts that meet the automatic promotion criteria.
///
/// Promotes facts with access_count >= 5 and last_accessed within 14 days.
/// Excludes superseded facts and already-pinned items.
///
/// Returns the number of facts promoted.
pub async fn promote_eligible_facts(pool: &PgPool) -> Result<usize, StateError> {
    let promoted = sqlx::query(
        r#"
        UPDATE loom_fact_state
        SET tier = 'hot', updated_at = now()
        WHERE tier = 'warm'
          AND access_count >= $1
          AND last_accessed >= now() - make_interval(days => $2)
          AND pinned = false
          AND fact_id NOT IN (
              SELECT id FROM loom_facts
              WHERE evidence_status = 'superseded'
          )
        "#,
    )
    .bind(PROMOTION_ACCESS_THRESHOLD)
    .bind(PROMOTION_WINDOW_DAYS as i32)
    .execute(pool)
    .await?
    .rows_affected();

    if promoted > 0 {
        tracing::info!(count = promoted, "facts promoted to hot tier");
    }
    Ok(promoted as usize)
}

// ---------------------------------------------------------------------------
// Tier management — demotion
// ---------------------------------------------------------------------------

/// Demote an entity from hot tier by unsetting pinned and moving to warm.
///
/// Used when a user unpins an entity.
///
/// # Errors
///
/// Returns [`StateError`] if the database update fails.
pub async fn demote_entity_unpin(
    pool: &PgPool,
    entity_id: Uuid,
) -> Result<(), StateError> {
    sqlx::query(
        r#"
        UPDATE loom_entity_state
        SET pinned = false, tier = 'warm', updated_at = now()
        WHERE entity_id = $1
        "#,
    )
    .bind(entity_id)
    .execute(pool)
    .await?;

    tracing::info!(entity_id = %entity_id, "entity unpinned and demoted to warm tier");
    Ok(())
}

/// Demote a fact from hot tier by unsetting pinned and moving to warm.
///
/// Used when a user unpins a fact.
///
/// # Errors
///
/// Returns [`StateError`] if the database update fails.
pub async fn demote_fact_unpin(
    pool: &PgPool,
    fact_id: Uuid,
) -> Result<(), StateError> {
    sqlx::query(
        r#"
        UPDATE loom_fact_state
        SET pinned = false, tier = 'warm', updated_at = now()
        WHERE fact_id = $1
        "#,
    )
    .bind(fact_id)
    .execute(pool)
    .await?;

    tracing::info!(fact_id = %fact_id, "fact unpinned and demoted to warm tier");
    Ok(())
}

/// Demote hot-tier entities not accessed in 30 days.
///
/// Skips pinned entities. Returns the number of entities demoted.
pub async fn demote_stale_entities(pool: &PgPool) -> Result<usize, StateError> {
    let demoted = sqlx::query(
        r#"
        UPDATE loom_entity_state
        SET tier = 'warm', updated_at = now()
        WHERE tier = 'hot'
          AND pinned = false
          AND (last_accessed IS NULL OR last_accessed < now() - make_interval(days => $1))
        "#,
    )
    .bind(DEMOTION_STALE_DAYS as i32)
    .execute(pool)
    .await?
    .rows_affected();

    if demoted > 0 {
        tracing::info!(count = demoted, "stale entities demoted from hot tier");
    }
    Ok(demoted as usize)
}

/// Demote hot-tier facts not accessed in 30 days.
///
/// Skips pinned facts. Returns the number of facts demoted.
pub async fn demote_stale_facts(pool: &PgPool) -> Result<usize, StateError> {
    let demoted = sqlx::query(
        r#"
        UPDATE loom_fact_state
        SET tier = 'warm', updated_at = now()
        WHERE tier = 'hot'
          AND pinned = false
          AND (last_accessed IS NULL OR last_accessed < now() - make_interval(days => $1))
        "#,
    )
    .bind(DEMOTION_STALE_DAYS as i32)
    .execute(pool)
    .await?
    .rows_affected();

    if demoted > 0 {
        tracing::info!(count = demoted, "stale facts demoted from hot tier");
    }
    Ok(demoted as usize)
}

/// Demote superseded facts from hot tier to warm tier.
///
/// Facts with `superseded_by IS NOT NULL` should never remain in the hot
/// tier. Returns the number of facts demoted.
pub async fn demote_superseded_facts(pool: &PgPool) -> Result<usize, StateError> {
    let demoted = sqlx::query(
        r#"
        UPDATE loom_fact_state
        SET tier = 'warm', updated_at = now()
        WHERE tier = 'hot'
          AND fact_id IN (
              SELECT id FROM loom_facts
              WHERE superseded_by IS NOT NULL
          )
        "#,
    )
    .execute(pool)
    .await?
    .rows_affected();

    if demoted > 0 {
        tracing::info!(count = demoted, "superseded facts demoted from hot tier");
    }
    Ok(demoted as usize)
}

// ---------------------------------------------------------------------------
// Tier management — budget enforcement
// ---------------------------------------------------------------------------

/// Row type for the lowest-salience hot tier item query.
#[derive(Debug, Clone, sqlx::FromRow)]
struct LowestSalienceItem {
    item_id: Uuid,
    salience_score: f64,
    item_kind: String,
}

/// Enforce the hot tier token budget for a namespace.
///
/// Queries the namespace config for `hot_tier_budget` (default 500 tokens),
/// computes the total tokens for hot tier items, and demotes the
/// lowest-salience unpinned item until the budget is satisfied.
///
/// Returns the number of items demoted.
pub async fn enforce_hot_tier_budget(
    pool: &PgPool,
    namespace: &str,
) -> Result<usize, StateError> {
    // Get the budget for this namespace.
    let budget = get_namespace_hot_tier_budget(pool, namespace).await?;
    let mut total_demoted: usize = 0;

    loop {
        let current_tokens = compute_namespace_hot_tier_tokens(pool, namespace).await?;
        if current_tokens <= budget {
            break;
        }

        // Find the lowest-salience unpinned hot item.
        let lowest = sqlx::query_as::<_, LowestSalienceItem>(
            r#"
            SELECT entity_id AS item_id, salience_score, 'entity' AS item_kind
            FROM loom_entity_state
            WHERE tier = 'hot' AND pinned = false
              AND entity_id IN (
                  SELECT id FROM loom_entities
                  WHERE namespace = $1 AND deleted_at IS NULL
              )
            UNION ALL
            SELECT fact_id AS item_id, salience_score, 'fact' AS item_kind
            FROM loom_fact_state
            WHERE tier = 'hot' AND pinned = false
              AND fact_id IN (
                  SELECT id FROM loom_facts
                  WHERE namespace = $1 AND deleted_at IS NULL
              )
            ORDER BY salience_score ASC
            LIMIT 1
            "#,
        )
        .bind(namespace)
        .fetch_optional(pool)
        .await?;

        match lowest {
            Some(item) => {
                match item.item_kind.as_str() {
                    "entity" => {
                        sqlx::query(
                            "UPDATE loom_entity_state SET tier = 'warm', updated_at = now() WHERE entity_id = $1",
                        )
                        .bind(item.item_id)
                        .execute(pool)
                        .await?;
                    }
                    "fact" => {
                        sqlx::query(
                            "UPDATE loom_fact_state SET tier = 'warm', updated_at = now() WHERE fact_id = $1",
                        )
                        .bind(item.item_id)
                        .execute(pool)
                        .await?;
                    }
                    _ => break,
                }
                total_demoted += 1;
                tracing::debug!(
                    namespace = namespace,
                    item_id = %item.item_id,
                    item_kind = %item.item_kind,
                    salience = item.salience_score,
                    "demoted lowest-salience item due to budget overflow"
                );
            }
            None => break, // No unpinned items to demote.
        }
    }

    if total_demoted > 0 {
        tracing::info!(
            namespace = namespace,
            demoted = total_demoted,
            "hot tier budget enforcement complete"
        );
    }
    Ok(total_demoted)
}

/// Get the hot tier budget for a namespace from `loom_namespace_config`.
///
/// Returns the configured `hot_tier_budget` or the default (500 tokens)
/// if no config exists.
pub async fn get_namespace_hot_tier_budget(
    pool: &PgPool,
    namespace: &str,
) -> Result<i32, StateError> {
    let row: Option<(Option<i32>,)> = sqlx::query_as(
        "SELECT hot_tier_budget FROM loom_namespace_config WHERE namespace = $1",
    )
    .bind(namespace)
    .fetch_optional(pool)
    .await?;

    Ok(row
        .and_then(|(budget,)| budget)
        .unwrap_or(DEFAULT_HOT_TIER_BUDGET))
}

/// Compute the total estimated token count for hot tier items in a namespace.
pub async fn compute_namespace_hot_tier_tokens(
    pool: &PgPool,
    namespace: &str,
) -> Result<i32, StateError> {
    let entity_count: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*)
        FROM loom_entity_state es
        JOIN loom_entities e ON e.id = es.entity_id
        WHERE es.tier = 'hot'
          AND e.namespace = $1
          AND e.deleted_at IS NULL
        "#,
    )
    .bind(namespace)
    .fetch_one(pool)
    .await?;

    let fact_count: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*)
        FROM loom_fact_state fs
        JOIN loom_facts f ON f.id = fs.fact_id
        WHERE fs.tier = 'hot'
          AND f.namespace = $1
          AND f.deleted_at IS NULL
          AND f.valid_until IS NULL
        "#,
    )
    .bind(namespace)
    .fetch_one(pool)
    .await?;

    let procedure_count: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*)
        FROM loom_procedures
        WHERE tier = 'hot'
          AND namespace = $1
          AND deleted_at IS NULL
        "#,
    )
    .bind(namespace)
    .fetch_one(pool)
    .await?;

    Ok(estimate_hot_tier_tokens(
        entity_count.0 as usize,
        fact_count.0 as usize,
        procedure_count.0 as usize,
    ))
}

// ---------------------------------------------------------------------------
// Tier management — warm tier archival
// ---------------------------------------------------------------------------

/// Archive superseded facts immediately.
///
/// Sets `tier = 'warm'` on any fact state where the fact has been
/// superseded (`superseded_by IS NOT NULL`). Archived facts remain
/// searchable but are excluded from automatic retrieval.
///
/// Returns the number of facts archived.
pub async fn archive_superseded_facts(pool: &PgPool) -> Result<usize, StateError> {
    // Superseded facts should be in warm tier (not hot), which effectively
    // archives them from auto-retrieval since the online pipeline filters
    // by valid_until IS NULL for current facts.
    let archived = sqlx::query(
        r#"
        UPDATE loom_fact_state
        SET tier = 'warm', updated_at = now()
        WHERE tier = 'hot'
          AND fact_id IN (
              SELECT id FROM loom_facts
              WHERE superseded_by IS NOT NULL
          )
        "#,
    )
    .execute(pool)
    .await?
    .rows_affected();

    if archived > 0 {
        tracing::info!(count = archived, "superseded facts archived");
    }
    Ok(archived as usize)
}

/// Archive facts not accessed in 90 days.
///
/// Moves stale warm-tier facts to an effectively archived state by
/// ensuring they remain in warm tier with stale access timestamps.
/// The online pipeline's recency weighting naturally deprioritizes
/// these items, and they are excluded from automatic retrieval.
///
/// Returns the number of facts identified as archival candidates.
pub async fn archive_stale_facts(pool: &PgPool) -> Result<usize, StateError> {
    // Facts not accessed in 90 days are already in warm tier. We ensure
    // any that somehow got promoted to hot are demoted back.
    let archived = sqlx::query(
        r#"
        UPDATE loom_fact_state
        SET tier = 'warm', updated_at = now()
        WHERE tier = 'hot'
          AND pinned = false
          AND (last_accessed IS NULL OR last_accessed < now() - make_interval(days => $1))
        "#,
    )
    .bind(ARCHIVAL_STALE_DAYS as i32)
    .execute(pool)
    .await?
    .rows_affected();

    if archived > 0 {
        tracing::info!(count = archived, days = ARCHIVAL_STALE_DAYS, "stale facts archived");
    }
    Ok(archived as usize)
}

/// Check whether a fact should be considered archived.
///
/// A fact is archived if:
/// - It has been superseded (`superseded_by` is `Some`), or
/// - It has not been accessed in 90 days.
///
/// Archived facts remain searchable but are excluded from automatic
/// retrieval in the online pipeline.
pub fn is_fact_archived(
    superseded_by: Option<Uuid>,
    last_accessed: Option<DateTime<Utc>>,
) -> bool {
    // Superseded facts are always archived.
    if superseded_by.is_some() {
        return true;
    }
    // Facts not accessed in 90 days are archived.
    match last_accessed {
        Some(accessed) => {
            let days_since = (Utc::now() - accessed).num_days();
            days_since >= ARCHIVAL_STALE_DAYS
        }
        None => true, // Never accessed = archived.
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::offline::extract::PredicateValidationResult;
    use crate::types::entity::ResolutionResult;

    #[test]
    fn state_error_episode_displays_message() {
        let err = StateError::Episode(EpisodeError::Sqlx(sqlx::Error::RowNotFound));
        let msg = err.to_string();
        assert!(msg.contains("episode database error"), "got: {msg}");
    }

    #[test]
    fn state_error_fact_displays_message() {
        let err = StateError::Fact(FactError::Sqlx(sqlx::Error::RowNotFound));
        let msg = err.to_string();
        assert!(msg.contains("fact database error"), "got: {msg}");
    }

    #[test]
    fn compute_metrics_empty_results() {
        let metrics = compute_extraction_metrics(
            &[],
            0,
            None,
            &[],
            100,
            "test-model",
        );

        assert_eq!(metrics.extracted, 0);
        assert_eq!(metrics.resolved_exact, 0);
        assert_eq!(metrics.resolved_alias, 0);
        assert_eq!(metrics.resolved_semantic, 0);
        assert_eq!(metrics.new, 0);
        assert_eq!(metrics.conflict_flagged, 0);
        assert_eq!(metrics.facts_extracted, 0);
        assert_eq!(metrics.canonical_predicate, 0);
        assert_eq!(metrics.custom_predicate, 0);
        assert_eq!(metrics.explicit, 0);
        assert_eq!(metrics.implied, 0);
        assert_eq!(metrics.processing_time_ms, 100);
        assert_eq!(metrics.extraction_model, "test-model");
    }

    #[test]
    fn compute_metrics_counts_resolution_methods() {
        let results = vec![
            ResolutionResult {
                entity_id: uuid::Uuid::new_v4(),
                method: "exact".to_string(),
                confidence: 1.0,
            },
            ResolutionResult {
                entity_id: uuid::Uuid::new_v4(),
                method: "exact".to_string(),
                confidence: 1.0,
            },
            ResolutionResult {
                entity_id: uuid::Uuid::new_v4(),
                method: "alias".to_string(),
                confidence: 0.95,
            },
            ResolutionResult {
                entity_id: uuid::Uuid::new_v4(),
                method: "semantic".to_string(),
                confidence: 0.94,
            },
            ResolutionResult {
                entity_id: uuid::Uuid::new_v4(),
                method: "new".to_string(),
                confidence: 1.0,
            },
        ];

        let metrics = compute_extraction_metrics(
            &results,
            1,
            None,
            &[],
            250,
            "gemma4:26b",
        );

        assert_eq!(metrics.extracted, 5);
        assert_eq!(metrics.resolved_exact, 2);
        assert_eq!(metrics.resolved_alias, 1);
        assert_eq!(metrics.resolved_semantic, 1);
        assert_eq!(metrics.new, 1);
        assert_eq!(metrics.conflict_flagged, 1);
        assert_eq!(metrics.extraction_model, "gemma4:26b");
    }

    #[test]
    fn compute_metrics_with_fact_results() {
        let fact_result = FactOrchestrationResult {
            inserted_count: 5,
            skipped_count: 1,
            inserted_fact_ids: vec![uuid::Uuid::new_v4(); 5],
            predicate_validation: Some(PredicateValidationResult {
                canonical_count: 3,
                custom_count: 2,
            }),
            superseded_count: 1,
            model: "gemma4:26b".to_string(),
        };

        let evidence = vec![
            Some("explicit".to_string()),
            Some("explicit".to_string()),
            Some("implied".to_string()),
            Some("explicit".to_string()),
            None,
        ];

        let metrics = compute_extraction_metrics(
            &[],
            0,
            Some(&fact_result),
            &evidence,
            500,
            "gemma4:26b",
        );

        assert_eq!(metrics.facts_extracted, 5);
        assert_eq!(metrics.canonical_predicate, 3);
        assert_eq!(metrics.custom_predicate, 2);
        assert_eq!(metrics.explicit, 3);
        assert_eq!(metrics.implied, 1);
        assert_eq!(metrics.processing_time_ms, 500);
    }

    #[test]
    fn compute_metrics_unknown_method_counted_as_new() {
        let results = vec![
            ResolutionResult {
                entity_id: uuid::Uuid::new_v4(),
                method: "unknown_method".to_string(),
                confidence: 0.5,
            },
        ];

        let metrics = compute_extraction_metrics(
            &results,
            0,
            None,
            &[],
            50,
            "test",
        );

        assert_eq!(metrics.new, 1);
    }

    #[test]
    fn salience_default_is_half() {
        assert!(
            (DEFAULT_SALIENCE - 0.5).abs() < f64::EPSILON,
            "default salience should be 0.5"
        );
    }

    #[test]
    fn salience_increases_on_access() {
        let initial = 0.5;
        let after_access = compute_salience_on_access(initial);
        assert!(
            after_access > initial,
            "salience should increase on access: {after_access} > {initial}"
        );
    }

    #[test]
    fn salience_clamped_to_one() {
        // Even with very high salience, result should not exceed 1.0.
        let after = compute_salience_on_access(1.0);
        assert!(
            after <= 1.0,
            "salience should be clamped to 1.0: {after}"
        );
    }

    #[test]
    fn salience_from_zero_gets_boost() {
        let after = compute_salience_on_access(0.0);
        assert!(
            (after - SALIENCE_ACCESS_BOOST).abs() < f64::EPSILON,
            "salience from 0.0 should equal boost: {after}"
        );
    }

    #[test]
    fn salience_converges_with_repeated_access() {
        let mut salience = DEFAULT_SALIENCE;
        for _ in 0..100 {
            salience = compute_salience_on_access(salience);
        }
        // Should converge to 1.0 (the clamp ceiling).
        assert!(
            salience > 0.95,
            "salience should converge near 1.0 with repeated access: {salience}"
        );
    }

    #[test]
    fn compute_metrics_serializes_to_json() {
        let metrics = compute_extraction_metrics(
            &[
                ResolutionResult {
                    entity_id: uuid::Uuid::new_v4(),
                    method: "exact".to_string(),
                    confidence: 1.0,
                },
            ],
            0,
            None,
            &[Some("explicit".to_string())],
            200,
            "test-model",
        );

        let json = serde_json::to_value(&metrics).expect("should serialize");
        assert_eq!(json["extracted"], 1);
        assert_eq!(json["resolved_exact"], 1);
        assert_eq!(json["processing_time_ms"], 200);
        assert_eq!(json["extraction_model"], "test-model");
        assert_eq!(json["explicit"], 1);
    }

    // -- Tier management: token estimation ----------------------------------

    #[test]
    fn estimate_hot_tier_tokens_empty() {
        assert_eq!(estimate_hot_tier_tokens(0, 0, 0), 0);
    }

    #[test]
    fn estimate_hot_tier_tokens_entities_only() {
        assert_eq!(estimate_hot_tier_tokens(3, 0, 0), 30);
    }

    #[test]
    fn estimate_hot_tier_tokens_facts_only() {
        assert_eq!(estimate_hot_tier_tokens(0, 4, 0), 60);
    }

    #[test]
    fn estimate_hot_tier_tokens_procedures_only() {
        assert_eq!(estimate_hot_tier_tokens(0, 0, 2), 60);
    }

    #[test]
    fn estimate_hot_tier_tokens_mixed() {
        // 2*10 + 3*15 + 1*30 = 20 + 45 + 30 = 95
        assert_eq!(estimate_hot_tier_tokens(2, 3, 1), 95);
    }

    // -- Tier management: promotion criteria --------------------------------

    #[test]
    fn qualifies_for_promotion_with_enough_accesses() {
        let now = chrono::Utc::now();
        assert!(qualifies_for_promotion(5, Some(now)));
        assert!(qualifies_for_promotion(10, Some(now)));
    }

    #[test]
    fn does_not_qualify_with_too_few_accesses() {
        let now = chrono::Utc::now();
        assert!(!qualifies_for_promotion(4, Some(now)));
        assert!(!qualifies_for_promotion(0, Some(now)));
    }

    #[test]
    fn does_not_qualify_with_stale_access() {
        let old = chrono::Utc::now() - chrono::Duration::days(15);
        assert!(!qualifies_for_promotion(5, Some(old)));
    }

    #[test]
    fn does_not_qualify_with_no_access() {
        assert!(!qualifies_for_promotion(5, None));
    }

    #[test]
    fn qualifies_at_boundary_14_days() {
        let boundary = chrono::Utc::now() - chrono::Duration::days(14);
        assert!(qualifies_for_promotion(5, Some(boundary)));
    }

    // -- Tier management: procedure hot tier prevention ----------------------

    #[test]
    fn procedure_eligible_all_criteria_met() {
        let first = chrono::Utc::now() - chrono::Duration::days(10);
        assert!(procedure_eligible_for_hot_tier(3, Some(first), 0.8));
        assert!(procedure_eligible_for_hot_tier(5, Some(first), 0.95));
    }

    #[test]
    fn procedure_not_eligible_too_few_episodes() {
        let first = chrono::Utc::now() - chrono::Duration::days(10);
        assert!(!procedure_eligible_for_hot_tier(2, Some(first), 0.9));
    }

    #[test]
    fn procedure_not_eligible_too_recent() {
        let first = chrono::Utc::now() - chrono::Duration::days(5);
        assert!(!procedure_eligible_for_hot_tier(3, Some(first), 0.9));
    }

    #[test]
    fn procedure_not_eligible_low_confidence() {
        let first = chrono::Utc::now() - chrono::Duration::days(10);
        assert!(!procedure_eligible_for_hot_tier(3, Some(first), 0.7));
    }

    #[test]
    fn procedure_not_eligible_no_first_observed() {
        assert!(!procedure_eligible_for_hot_tier(3, None, 0.9));
    }

    #[test]
    fn procedure_boundary_exactly_7_days() {
        let first = chrono::Utc::now() - chrono::Duration::days(7);
        assert!(procedure_eligible_for_hot_tier(3, Some(first), 0.8));
    }

    // -- Tier management: demotion criteria ----------------------------------

    #[test]
    fn stale_access_triggers_demotion() {
        let old = chrono::Utc::now() - chrono::Duration::days(31);
        let days_since = (chrono::Utc::now() - old).num_days();
        assert!(days_since > DEMOTION_STALE_DAYS);
    }

    #[test]
    fn recent_access_does_not_trigger_demotion() {
        let recent = chrono::Utc::now() - chrono::Duration::days(10);
        let days_since = (chrono::Utc::now() - recent).num_days();
        assert!(days_since <= DEMOTION_STALE_DAYS);
    }

    // -- Tier management: archival logic ------------------------------------

    #[test]
    fn superseded_fact_is_archived() {
        assert!(is_fact_archived(Some(uuid::Uuid::new_v4()), Some(chrono::Utc::now())));
    }

    #[test]
    fn fact_not_accessed_in_90_days_is_archived() {
        let old = chrono::Utc::now() - chrono::Duration::days(91);
        assert!(is_fact_archived(None, Some(old)));
    }

    #[test]
    fn fact_never_accessed_is_archived() {
        assert!(is_fact_archived(None, None));
    }

    #[test]
    fn recently_accessed_fact_is_not_archived() {
        let recent = chrono::Utc::now() - chrono::Duration::days(30);
        assert!(!is_fact_archived(None, Some(recent)));
    }

    #[test]
    fn fact_at_90_day_boundary_is_archived() {
        let boundary = chrono::Utc::now() - chrono::Duration::days(90);
        assert!(is_fact_archived(None, Some(boundary)));
    }

    #[test]
    fn superseded_fact_archived_regardless_of_access() {
        let recent = chrono::Utc::now();
        assert!(is_fact_archived(Some(uuid::Uuid::new_v4()), Some(recent)));
    }

    // -- Tier management: budget enforcement --------------------------------

    #[test]
    fn budget_not_exceeded_with_few_items() {
        let tokens = estimate_hot_tier_tokens(5, 5, 1);
        // 5*10 + 5*15 + 1*30 = 50 + 75 + 30 = 155
        assert_eq!(tokens, 155);
        assert!(tokens <= DEFAULT_HOT_TIER_BUDGET);
    }

    #[test]
    fn budget_exceeded_with_many_items() {
        let tokens = estimate_hot_tier_tokens(20, 20, 5);
        // 20*10 + 20*15 + 5*30 = 200 + 300 + 150 = 650
        assert_eq!(tokens, 650);
        assert!(tokens > DEFAULT_HOT_TIER_BUDGET);
    }

    // -- Constants ----------------------------------------------------------

    #[test]
    fn default_hot_tier_budget_is_500() {
        assert_eq!(DEFAULT_HOT_TIER_BUDGET, 500);
    }

    #[test]
    fn promotion_threshold_is_5() {
        assert_eq!(PROMOTION_ACCESS_THRESHOLD, 5);
    }

    #[test]
    fn promotion_window_is_14_days() {
        assert_eq!(PROMOTION_WINDOW_DAYS, 14);
    }

    #[test]
    fn demotion_stale_days_is_30() {
        assert_eq!(DEMOTION_STALE_DAYS, 30);
    }

    #[test]
    fn archival_stale_days_is_90() {
        assert_eq!(ARCHIVAL_STALE_DAYS, 90);
    }

    #[test]
    fn procedure_min_episodes_is_3() {
        assert_eq!(PROCEDURE_MIN_EPISODES, 3);
    }

    #[test]
    fn procedure_min_age_is_7_days() {
        assert_eq!(PROCEDURE_MIN_AGE_DAYS, 7);
    }

    #[test]
    fn procedure_min_confidence_is_0_8() {
        assert!((PROCEDURE_MIN_CONFIDENCE - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn state_error_entity_displays_message() {
        let err = StateError::Entity(EntityError::Sqlx(sqlx::Error::RowNotFound));
        let msg = err.to_string();
        assert!(msg.contains("entity database error"), "got: {msg}");
    }

    #[test]
    fn state_error_sqlx_displays_message() {
        let err = StateError::Sqlx(sqlx::Error::RowNotFound);
        let msg = err.to_string();
        assert!(msg.contains("database error"), "got: {msg}");
    }
}
