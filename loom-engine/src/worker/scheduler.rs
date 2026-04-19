//! Periodic scheduled tasks via tokio.
//!
//! Runs three background jobs on fixed intervals, all cancellable via
//! [`tokio_util::sync::CancellationToken`]:
//!
//! - **Daily hot tier snapshot** — captures hot entities, facts, and procedures
//!   per namespace and stores a snapshot in `loom_snapshots`.
//! - **Daily tier management** — evaluates promotion and demotion criteria and
//!   updates tier assignments on entity and fact serving state.
//! - **Weekly entity health check** — identifies potential duplicate entity
//!   pairs by embedding similarity and logs them for dashboard surfacing.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::db::snapshots::{self, NewSnapshot};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during scheduled task execution.
#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    /// An underlying sqlx / PostgreSQL error.
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),

    /// A snapshot insertion error.
    #[error("snapshot error: {0}")]
    Snapshot(#[from] snapshots::SnapshotError),
}

// ---------------------------------------------------------------------------
// Schedule intervals
// ---------------------------------------------------------------------------

/// Interval for the daily snapshot job (24 hours).
const DAILY_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Interval for the weekly entity health check (7 days).
const WEEKLY_INTERVAL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

// ---------------------------------------------------------------------------
// Result types for scheduled queries
// ---------------------------------------------------------------------------

/// A namespace row returned from `loom_namespace_config`.
#[derive(Debug, Clone, sqlx::FromRow)]
struct NamespaceRow {
    namespace: String,
    hot_tier_budget: Option<i32>,
}

/// A hot-tier entity summary used when building snapshot JSONB.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
struct HotEntity {
    id: Uuid,
    name: String,
    entity_type: String,
}

/// A hot-tier fact summary used when building snapshot JSONB.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
struct HotFact {
    id: Uuid,
    subject_id: Uuid,
    predicate: String,
    object_id: Uuid,
}

/// A hot-tier procedure summary used when building snapshot JSONB.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
struct HotProcedure {
    id: Uuid,
    pattern: String,
}

/// A potential duplicate entity pair from the health check.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DuplicateEntityPair {
    /// First entity in the pair.
    pub entity_a: Uuid,
    /// Second entity in the pair.
    pub entity_b: Uuid,
    /// Entity name of the first entity.
    pub name_a: String,
    /// Entity name of the second entity.
    pub name_b: String,
    /// Shared entity type.
    pub entity_type: String,
    /// Shared namespace.
    pub namespace: String,
    /// Cosine similarity between the two entity embeddings.
    pub similarity: f64,
}

// ---------------------------------------------------------------------------
// Public API — start_scheduler
// ---------------------------------------------------------------------------

/// Start all scheduled background jobs.
///
/// Spawns three tokio tasks:
/// 1. Daily hot tier snapshot (every 24 hours).
/// 2. Daily tier promotion/demotion check (every 24 hours).
/// 3. Weekly entity health check (every 7 days).
///
/// All tasks are cancellable via the provided [`CancellationToken`].
/// Returns a `Vec` of [`tokio::task::JoinHandle`] for each spawned task.
///
/// # Arguments
///
/// * `pool` — The **offline** database connection pool.
/// * `cancel_token` — Token to signal graceful shutdown of all jobs.
pub fn start_scheduler(
    pool: PgPool,
    cancel_token: CancellationToken,
) -> Vec<tokio::task::JoinHandle<()>> {
    tracing::info!("starting scheduled task runner");

    let snapshot_pool = pool.clone();
    let snapshot_token = cancel_token.clone();
    let snapshot_handle = tokio::spawn(async move {
        run_periodic(
            "daily_snapshot",
            DAILY_INTERVAL,
            snapshot_token,
            move || {
                let p = snapshot_pool.clone();
                async move { run_daily_snapshot(&p).await }
            },
        )
        .await;
    });

    let tier_pool = pool.clone();
    let tier_token = cancel_token.clone();
    let tier_handle = tokio::spawn(async move {
        run_periodic(
            "daily_tier_management",
            DAILY_INTERVAL,
            tier_token,
            move || {
                let p = tier_pool.clone();
                async move { run_tier_management(&p).await }
            },
        )
        .await;
    });

    let health_pool = pool.clone();
    let health_token = cancel_token.clone();
    let health_handle = tokio::spawn(async move {
        run_periodic(
            "weekly_entity_health_check",
            WEEKLY_INTERVAL,
            health_token,
            move || {
                let p = health_pool.clone();
                async move { run_entity_health_check(&p).await }
            },
        )
        .await;
    });

    vec![snapshot_handle, tier_handle, health_handle]
}

// ---------------------------------------------------------------------------
// Generic periodic runner
// ---------------------------------------------------------------------------

/// Run a task function on a fixed interval until the cancellation token fires.
async fn run_periodic<F, Fut>(
    name: &str,
    interval: Duration,
    cancel_token: CancellationToken,
    task_fn: F,
) where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<(), SchedulerError>>,
{
    tracing::info!(task = name, interval_secs = interval.as_secs(), "scheduled task registered");

    loop {
        tokio::select! {
            _ = cancel_token.cancelled() => {
                tracing::info!(task = name, "scheduled task received cancellation, exiting");
                break;
            }
            _ = tokio::time::sleep(interval) => {
                tracing::info!(task = name, "executing scheduled task");
                match task_fn().await {
                    Ok(()) => {
                        tracing::info!(task = name, "scheduled task completed successfully");
                    }
                    Err(e) => {
                        tracing::error!(task = name, error = %e, "scheduled task failed");
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Daily hot tier snapshot
// ---------------------------------------------------------------------------

/// Capture a snapshot of the hot tier for every namespace.
///
/// For each namespace in `loom_namespace_config`:
/// 1. Query hot-tier entities from `loom_entity_state`.
/// 2. Query hot-tier facts from `loom_fact_state`.
/// 3. Query hot-tier procedures from `loom_procedures`.
/// 4. Compute a rough total token count.
/// 5. Store the snapshot in `loom_snapshots`.
///
/// **Validates: Requirements 29.1, 29.2, 29.3, 29.4, 29.5**
pub async fn run_daily_snapshot(pool: &PgPool) -> Result<(), SchedulerError> {
    let namespaces = list_namespaces(pool).await?;

    if namespaces.is_empty() {
        tracing::debug!("no namespaces configured, skipping snapshot");
        return Ok(());
    }

    for ns in &namespaces {
        let hot_entities = query_hot_entities(pool, &ns.namespace).await?;
        let hot_facts = query_hot_facts(pool, &ns.namespace).await?;
        let hot_procedures = query_hot_procedures(pool, &ns.namespace).await?;

        // Rough token estimate: count of items as a proxy. A real implementation
        // would sum actual token counts from content, but the schema stores an
        // integer total_tokens field for the snapshot.
        let total_tokens = estimate_hot_tier_tokens(&hot_entities, &hot_facts, &hot_procedures);

        let entities_json = serde_json::to_value(&hot_entities).ok();
        let facts_json = serde_json::to_value(&hot_facts).ok();
        let procedures_json = serde_json::to_value(&hot_procedures).ok();

        let new_snapshot = NewSnapshot {
            namespace: ns.namespace.clone(),
            hot_entities: entities_json,
            hot_facts: facts_json,
            hot_procedures: procedures_json,
            total_tokens: Some(total_tokens),
        };

        snapshots::insert_snapshot(pool, &new_snapshot).await?;

        tracing::info!(
            namespace = %ns.namespace,
            hot_entities = hot_entities.len(),
            hot_facts = hot_facts.len(),
            hot_procedures = hot_procedures.len(),
            total_tokens,
            "hot tier snapshot stored"
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Daily tier management
// ---------------------------------------------------------------------------

/// Evaluate promotion and demotion criteria and update tier assignments.
///
/// **Promotion criteria** (warm → hot):
/// - Item used in 5+ compilations within the last 14 days.
///
/// **Demotion criteria** (hot → warm):
/// - Hot item not accessed in 30 days.
/// - Fact is superseded (`evidence_status = 'superseded'`).
/// - Hot tier exceeds namespace budget → demote lowest-salience item.
///
/// **Validates: Requirements 24.1, 24.2, 24.3, 24.4, 24.5**
pub async fn run_tier_management(pool: &PgPool) -> Result<(), SchedulerError> {
    // --- Promotions --------------------------------------------------------
    let promoted = promote_eligible_items(pool).await?;
    tracing::info!(promoted_count = promoted, "tier promotions applied");

    // --- Demotions: stale hot items (not accessed in 30 days) --------------
    let demoted_stale = demote_stale_hot_items(pool).await?;
    tracing::info!(demoted_stale = demoted_stale, "stale hot items demoted");

    // --- Demotions: superseded facts ---------------------------------------
    let demoted_superseded = demote_superseded_facts(pool).await?;
    tracing::info!(demoted_superseded = demoted_superseded, "superseded facts demoted");

    // --- Demotions: budget overflow ----------------------------------------
    let demoted_overflow = demote_budget_overflow(pool).await?;
    tracing::info!(demoted_overflow = demoted_overflow, "budget overflow items demoted");

    Ok(())
}

// ---------------------------------------------------------------------------
// Weekly entity health check
// ---------------------------------------------------------------------------

/// Identify potential duplicate entity pairs by embedding similarity.
///
/// Queries entity pairs in the same namespace and type where cosine
/// similarity exceeds 0.85. Returns the top 50 pairs ranked by
/// similarity descending. Excludes soft-deleted entities.
///
/// Results are logged for dashboard surfacing.
///
/// **Validates: Requirements 24.1, 24.2, 24.3, 24.4, 24.5**
pub async fn run_entity_health_check(pool: &PgPool) -> Result<(), SchedulerError> {
    let pairs = query_duplicate_entity_pairs(pool).await?;

    if pairs.is_empty() {
        tracing::info!("entity health check: no potential duplicates found");
        return Ok(());
    }

    tracing::info!(
        pair_count = pairs.len(),
        "entity health check: potential duplicate pairs found"
    );

    for pair in &pairs {
        tracing::info!(
            entity_a = %pair.entity_a,
            entity_b = %pair.entity_b,
            name_a = %pair.name_a,
            name_b = %pair.name_b,
            entity_type = %pair.entity_type,
            namespace = %pair.namespace,
            similarity = pair.similarity,
            "potential duplicate entity pair"
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helper queries
// ---------------------------------------------------------------------------

/// List all configured namespaces.
async fn list_namespaces(pool: &PgPool) -> Result<Vec<NamespaceRow>, SchedulerError> {
    let rows = sqlx::query_as::<_, NamespaceRow>(
        "SELECT namespace, hot_tier_budget FROM loom_namespace_config ORDER BY namespace",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Query hot-tier entities for a namespace.
async fn query_hot_entities(
    pool: &PgPool,
    namespace: &str,
) -> Result<Vec<HotEntity>, SchedulerError> {
    let rows = sqlx::query_as::<_, HotEntity>(
        r#"
        SELECT e.id, e.name, e.entity_type
        FROM loom_entities e
        JOIN loom_entity_state es ON es.entity_id = e.id
        WHERE e.namespace = $1
          AND e.deleted_at IS NULL
          AND es.tier = 'hot'
        ORDER BY es.salience_score DESC
        "#,
    )
    .bind(namespace)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Query hot-tier facts for a namespace.
async fn query_hot_facts(
    pool: &PgPool,
    namespace: &str,
) -> Result<Vec<HotFact>, SchedulerError> {
    let rows = sqlx::query_as::<_, HotFact>(
        r#"
        SELECT f.id, f.subject_id, f.predicate, f.object_id
        FROM loom_facts f
        JOIN loom_fact_state fs ON fs.fact_id = f.id
        WHERE f.namespace = $1
          AND f.deleted_at IS NULL
          AND f.valid_until IS NULL
          AND fs.tier = 'hot'
        ORDER BY fs.salience_score DESC
        "#,
    )
    .bind(namespace)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Query hot-tier procedures for a namespace.
async fn query_hot_procedures(
    pool: &PgPool,
    namespace: &str,
) -> Result<Vec<HotProcedure>, SchedulerError> {
    let rows = sqlx::query_as::<_, HotProcedure>(
        r#"
        SELECT id, pattern
        FROM loom_procedures
        WHERE namespace = $1
          AND deleted_at IS NULL
          AND tier = 'hot'
        ORDER BY confidence DESC
        "#,
    )
    .bind(namespace)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Rough token estimate for the hot tier contents.
///
/// Uses a simple heuristic: each entity ≈ 10 tokens, each fact ≈ 15 tokens,
/// each procedure ≈ 30 tokens. A production system would compute actual
/// token counts from content.
fn estimate_hot_tier_tokens(
    entities: &[HotEntity],
    facts: &[HotFact],
    procedures: &[HotProcedure],
) -> i32 {
    let entity_tokens = entities.len() as i32 * 10;
    let fact_tokens = facts.len() as i32 * 15;
    let procedure_tokens = procedures.len() as i32 * 30;
    entity_tokens + fact_tokens + procedure_tokens
}

/// Promote warm-tier items that have been used in 5+ compilations in the
/// last 14 days.
///
/// Checks both entity and fact serving state tables. Returns the number
/// of items promoted.
async fn promote_eligible_items(pool: &PgPool) -> Result<usize, SchedulerError> {
    // Promote entities: warm → hot where access_count >= 5 and last_accessed
    // within 14 days.
    let entity_promoted = sqlx::query(
        r#"
        UPDATE loom_entity_state
        SET tier = 'hot', updated_at = now()
        WHERE tier = 'warm'
          AND access_count >= 5
          AND last_accessed >= now() - INTERVAL '14 days'
          AND pinned = false
        "#,
    )
    .execute(pool)
    .await?
    .rows_affected();

    // Promote facts: warm → hot where access_count >= 5 and last_accessed
    // within 14 days, and the fact is not superseded.
    let fact_promoted = sqlx::query(
        r#"
        UPDATE loom_fact_state
        SET tier = 'hot', updated_at = now()
        WHERE tier = 'warm'
          AND access_count >= 5
          AND last_accessed >= now() - INTERVAL '14 days'
          AND pinned = false
          AND fact_id NOT IN (
              SELECT id FROM loom_facts
              WHERE evidence_status = 'superseded'
          )
        "#,
    )
    .execute(pool)
    .await?
    .rows_affected();

    let total = (entity_promoted + fact_promoted) as usize;

    if total > 0 {
        tracing::info!(
            entities = entity_promoted,
            facts = fact_promoted,
            "promoted items to hot tier"
        );
    }

    Ok(total)
}

/// Demote hot-tier items that have not been accessed in 30 days.
///
/// Returns the number of items demoted.
async fn demote_stale_hot_items(pool: &PgPool) -> Result<usize, SchedulerError> {
    let entity_demoted = sqlx::query(
        r#"
        UPDATE loom_entity_state
        SET tier = 'warm', updated_at = now()
        WHERE tier = 'hot'
          AND pinned = false
          AND (last_accessed IS NULL OR last_accessed < now() - INTERVAL '30 days')
        "#,
    )
    .execute(pool)
    .await?
    .rows_affected();

    let fact_demoted = sqlx::query(
        r#"
        UPDATE loom_fact_state
        SET tier = 'warm', updated_at = now()
        WHERE tier = 'hot'
          AND pinned = false
          AND (last_accessed IS NULL OR last_accessed < now() - INTERVAL '30 days')
        "#,
    )
    .execute(pool)
    .await?
    .rows_affected();

    Ok((entity_demoted + fact_demoted) as usize)
}

/// Demote superseded facts from hot tier to warm tier.
///
/// Facts with `evidence_status = 'superseded'` should never remain in the
/// hot tier.
///
/// Returns the number of facts demoted.
async fn demote_superseded_facts(pool: &PgPool) -> Result<usize, SchedulerError> {
    let demoted = sqlx::query(
        r#"
        UPDATE loom_fact_state
        SET tier = 'warm', updated_at = now()
        WHERE tier = 'hot'
          AND fact_id IN (
              SELECT id FROM loom_facts
              WHERE evidence_status = 'superseded'
          )
        "#,
    )
    .execute(pool)
    .await?
    .rows_affected();

    Ok(demoted as usize)
}

/// Demote the lowest-salience hot item when a namespace exceeds its budget.
///
/// For each namespace, checks if the estimated hot tier token count exceeds
/// the configured `hot_tier_budget`. If so, demotes the lowest-salience
/// entity or fact until the budget is satisfied.
///
/// Returns the total number of items demoted across all namespaces.
async fn demote_budget_overflow(pool: &PgPool) -> Result<usize, SchedulerError> {
    let namespaces = list_namespaces(pool).await?;
    let mut total_demoted: usize = 0;

    for ns in &namespaces {
        let budget = ns.hot_tier_budget.unwrap_or(500);

        loop {
            let hot_entities = query_hot_entities(pool, &ns.namespace).await?;
            let hot_facts = query_hot_facts(pool, &ns.namespace).await?;
            let hot_procedures = query_hot_procedures(pool, &ns.namespace).await?;
            let current_tokens =
                estimate_hot_tier_tokens(&hot_entities, &hot_facts, &hot_procedures);

            if current_tokens <= budget {
                break;
            }

            // Find the lowest-salience unpinned hot entity.
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
            .bind(&ns.namespace)
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
                        namespace = %ns.namespace,
                        item_id = %item.item_id,
                        item_kind = %item.item_kind,
                        salience = item.salience_score,
                        "demoted lowest-salience item due to budget overflow"
                    );
                }
                None => break, // No unpinned items to demote.
            }
        }
    }

    Ok(total_demoted)
}

/// Helper row for finding the lowest-salience hot item.
#[derive(Debug, Clone, sqlx::FromRow)]
struct LowestSalienceItem {
    item_id: Uuid,
    salience_score: f64,
    item_kind: String,
}

/// Query entity pairs in the same namespace and type with embedding
/// similarity above 0.85.
///
/// Returns the top 50 pairs ranked by similarity descending. Excludes
/// soft-deleted entities.
async fn query_duplicate_entity_pairs(
    pool: &PgPool,
) -> Result<Vec<DuplicateEntityPair>, SchedulerError> {
    let rows = sqlx::query_as::<_, DuplicateEntityPair>(
        r#"
        SELECT
            e1.id AS entity_a,
            e2.id AS entity_b,
            e1.name AS name_a,
            e2.name AS name_b,
            e1.entity_type,
            e1.namespace,
            1.0 - (es1.embedding <=> es2.embedding) AS similarity
        FROM loom_entities e1
        JOIN loom_entity_state es1 ON es1.entity_id = e1.id
        JOIN loom_entities e2 ON e2.entity_type = e1.entity_type
                              AND e2.namespace = e1.namespace
                              AND e2.id > e1.id
        JOIN loom_entity_state es2 ON es2.entity_id = e2.id
        WHERE e1.deleted_at IS NULL
          AND e2.deleted_at IS NULL
          AND es1.embedding IS NOT NULL
          AND es2.embedding IS NOT NULL
          AND 1.0 - (es1.embedding <=> es2.embedding) > 0.85
        ORDER BY similarity DESC
        LIMIT 50
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Error display ------------------------------------------------------

    #[test]
    fn scheduler_error_sqlx_displays_message() {
        let err = SchedulerError::Sqlx(sqlx::Error::RowNotFound);
        assert!(err.to_string().contains("database error"));
    }

    #[test]
    fn scheduler_error_snapshot_displays_message() {
        let err = SchedulerError::Snapshot(snapshots::SnapshotError::Sqlx(
            sqlx::Error::RowNotFound,
        ));
        assert!(err.to_string().contains("snapshot error"));
    }

    // -- Token estimation ---------------------------------------------------

    #[test]
    fn estimate_tokens_empty() {
        let tokens = estimate_hot_tier_tokens(&[], &[], &[]);
        assert_eq!(tokens, 0);
    }

    #[test]
    fn estimate_tokens_with_items() {
        let entities = vec![
            HotEntity {
                id: Uuid::new_v4(),
                name: "Foo".to_string(),
                entity_type: "service".to_string(),
            },
            HotEntity {
                id: Uuid::new_v4(),
                name: "Bar".to_string(),
                entity_type: "project".to_string(),
            },
        ];
        let facts = vec![HotFact {
            id: Uuid::new_v4(),
            subject_id: Uuid::new_v4(),
            predicate: "uses".to_string(),
            object_id: Uuid::new_v4(),
        }];
        let procedures = vec![HotProcedure {
            id: Uuid::new_v4(),
            pattern: "check logs first".to_string(),
        }];

        // 2*10 + 1*15 + 1*30 = 65
        assert_eq!(estimate_hot_tier_tokens(&entities, &facts, &procedures), 65);
    }

    // -- DuplicateEntityPair serialization ----------------------------------

    #[test]
    fn duplicate_pair_serializes_to_json() {
        let pair = DuplicateEntityPair {
            entity_a: Uuid::new_v4(),
            entity_b: Uuid::new_v4(),
            name_a: "APIM".to_string(),
            name_b: "Azure API Management".to_string(),
            entity_type: "service".to_string(),
            namespace: "default".to_string(),
            similarity: 0.91,
        };
        let json = serde_json::to_value(&pair).expect("should serialize");
        assert_eq!(json["name_a"], "APIM");
        assert!(json["similarity"].as_f64().unwrap() > 0.9);
    }

    // -- CancellationToken stops scheduler ----------------------------------

    #[tokio::test]
    async fn cancellation_stops_scheduler() {
        let cancel_token = CancellationToken::new();
        let pool = dummy_pool();

        let handles = start_scheduler(pool, cancel_token.clone());
        assert_eq!(handles.len(), 3, "should spawn 3 scheduled tasks");

        // Cancel immediately.
        cancel_token.cancel();

        for handle in handles {
            let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
            assert!(result.is_ok(), "scheduled task should exit on cancellation");
        }
    }

    // -- run_periodic exits on cancellation ---------------------------------

    #[tokio::test]
    async fn run_periodic_exits_on_cancel() {
        let token = CancellationToken::new();
        let token_clone = token.clone();

        let handle = tokio::spawn(async move {
            run_periodic(
                "test_task",
                Duration::from_secs(3600),
                token_clone,
                || async { Ok(()) },
            )
            .await;
        });

        token.cancel();

        let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(result.is_ok(), "run_periodic should exit on cancellation");
    }

    /// Create a dummy pool that will panic if used. Only for testing
    /// cancellation paths where the pool is never actually accessed.
    fn dummy_pool() -> PgPool {
        use sqlx::postgres::PgPoolOptions;
        PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://invalid:5432/nonexistent")
            .expect("connect_lazy should not fail")
    }

    // -- Snapshot token estimation ------------------------------------------

    #[test]
    fn estimate_tokens_scales_linearly() {
        let entities: Vec<HotEntity> = (0..10)
            .map(|i| HotEntity {
                id: Uuid::new_v4(),
                name: format!("entity_{i}"),
                entity_type: "service".to_string(),
            })
            .collect();
        let facts: Vec<HotFact> = (0..5)
            .map(|_| HotFact {
                id: Uuid::new_v4(),
                subject_id: Uuid::new_v4(),
                predicate: "uses".to_string(),
                object_id: Uuid::new_v4(),
            })
            .collect();
        let procedures: Vec<HotProcedure> = (0..3)
            .map(|_| HotProcedure {
                id: Uuid::new_v4(),
                pattern: "check logs".to_string(),
            })
            .collect();

        // 10*10 + 5*15 + 3*30 = 100 + 75 + 90 = 265
        let tokens = estimate_hot_tier_tokens(&entities, &facts, &procedures);
        assert_eq!(tokens, 265);
    }

    #[test]
    fn estimate_tokens_entities_only() {
        let entities = vec![HotEntity {
            id: Uuid::new_v4(),
            name: "single".to_string(),
            entity_type: "project".to_string(),
        }];
        assert_eq!(estimate_hot_tier_tokens(&entities, &[], &[]), 10);
    }

    #[test]
    fn estimate_tokens_facts_only() {
        let facts = vec![HotFact {
            id: Uuid::new_v4(),
            subject_id: Uuid::new_v4(),
            predicate: "depends_on".to_string(),
            object_id: Uuid::new_v4(),
        }];
        assert_eq!(estimate_hot_tier_tokens(&[], &facts, &[]), 15);
    }

    #[test]
    fn estimate_tokens_procedures_only() {
        let procedures = vec![HotProcedure {
            id: Uuid::new_v4(),
            pattern: "always run tests".to_string(),
        }];
        assert_eq!(estimate_hot_tier_tokens(&[], &[], &procedures), 30);
    }

    // -- HotEntity / HotFact / HotProcedure serialization -------------------

    #[test]
    fn hot_entity_serializes_all_fields() {
        let entity = HotEntity {
            id: Uuid::new_v4(),
            name: "TestService".to_string(),
            entity_type: "service".to_string(),
        };
        let json = serde_json::to_value(&entity).expect("should serialize");
        assert!(json["id"].is_string());
        assert_eq!(json["name"], "TestService");
        assert_eq!(json["entity_type"], "service");
    }

    #[test]
    fn hot_fact_serializes_all_fields() {
        let fact = HotFact {
            id: Uuid::new_v4(),
            subject_id: Uuid::new_v4(),
            predicate: "uses".to_string(),
            object_id: Uuid::new_v4(),
        };
        let json = serde_json::to_value(&fact).expect("should serialize");
        assert!(json["id"].is_string());
        assert!(json["subject_id"].is_string());
        assert_eq!(json["predicate"], "uses");
        assert!(json["object_id"].is_string());
    }

    #[test]
    fn hot_procedure_serializes_all_fields() {
        let proc = HotProcedure {
            id: Uuid::new_v4(),
            pattern: "check logs first".to_string(),
        };
        let json = serde_json::to_value(&proc).expect("should serialize");
        assert!(json["id"].is_string());
        assert_eq!(json["pattern"], "check logs first");
    }

    // -- Snapshot JSONB structure (Req 29.1) --------------------------------

    #[test]
    fn snapshot_jsonb_structure_is_valid() {
        let entities = vec![HotEntity {
            id: Uuid::new_v4(),
            name: "APIM".to_string(),
            entity_type: "service".to_string(),
        }];
        let facts = vec![HotFact {
            id: Uuid::new_v4(),
            subject_id: Uuid::new_v4(),
            predicate: "deployed_to".to_string(),
            object_id: Uuid::new_v4(),
        }];
        let procedures = vec![HotProcedure {
            id: Uuid::new_v4(),
            pattern: "check APIM logs first".to_string(),
        }];

        let entities_json = serde_json::to_value(&entities).expect("entities serialize");
        let facts_json = serde_json::to_value(&facts).expect("facts serialize");
        let procedures_json = serde_json::to_value(&procedures).expect("procedures serialize");

        // Verify JSON arrays.
        assert!(entities_json.is_array());
        assert!(facts_json.is_array());
        assert!(procedures_json.is_array());

        // Verify array lengths.
        assert_eq!(entities_json.as_array().unwrap().len(), 1);
        assert_eq!(facts_json.as_array().unwrap().len(), 1);
        assert_eq!(procedures_json.as_array().unwrap().len(), 1);

        // Verify total tokens.
        let total_tokens = estimate_hot_tier_tokens(&entities, &facts, &procedures);
        assert_eq!(total_tokens, 10 + 15 + 30); // 55
    }

    // -- Scheduler spawns correct number of tasks ---------------------------

    #[tokio::test]
    async fn scheduler_spawns_three_tasks() {
        let cancel_token = CancellationToken::new();
        let pool = dummy_pool();

        let handles = start_scheduler(pool, cancel_token.clone());
        assert_eq!(handles.len(), 3, "scheduler should spawn exactly 3 tasks");

        cancel_token.cancel();
        for handle in handles {
            let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
            assert!(result.is_ok(), "each task should exit on cancellation");
        }
    }

    // -- run_periodic with immediate cancellation ---------------------------

    #[tokio::test]
    async fn run_periodic_never_executes_on_immediate_cancel() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let token = CancellationToken::new();
        let token_clone = token.clone();

        // Cancel before starting.
        token.cancel();

        let handle = tokio::spawn(async move {
            run_periodic(
                "test_never_runs",
                Duration::from_millis(10),
                token_clone,
                move || {
                    let c = Arc::clone(&counter_clone);
                    async move {
                        c.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }
                },
            )
            .await;
        });

        let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;

        assert_eq!(
            counter.load(Ordering::SeqCst),
            0,
            "task function should never execute when cancelled immediately"
        );
    }

    // -- DuplicateEntityPair round-trip -------------------------------------

    #[test]
    fn duplicate_pair_round_trip_serialization() {
        let pair = DuplicateEntityPair {
            entity_a: Uuid::new_v4(),
            entity_b: Uuid::new_v4(),
            name_a: "APIM".to_string(),
            name_b: "Azure API Management".to_string(),
            entity_type: "service".to_string(),
            namespace: "default".to_string(),
            similarity: 0.91,
        };

        let json_str = serde_json::to_string(&pair).expect("should serialize");
        let deserialized: DuplicateEntityPair =
            serde_json::from_str(&json_str).expect("should deserialize");

        assert_eq!(deserialized.entity_a, pair.entity_a);
        assert_eq!(deserialized.entity_b, pair.entity_b);
        assert_eq!(deserialized.name_a, pair.name_a);
        assert_eq!(deserialized.name_b, pair.name_b);
        assert_eq!(deserialized.entity_type, pair.entity_type);
        assert_eq!(deserialized.namespace, pair.namespace);
        assert!((deserialized.similarity - pair.similarity).abs() < f64::EPSILON);
    }

    // -- SchedulerError variants --------------------------------------------

    #[test]
    fn scheduler_error_from_sqlx() {
        let err: SchedulerError = sqlx::Error::RowNotFound.into();
        assert!(err.to_string().contains("database error"));
    }

    #[test]
    fn scheduler_error_from_snapshot() {
        let snap_err = snapshots::SnapshotError::Sqlx(sqlx::Error::RowNotFound);
        let err: SchedulerError = snap_err.into();
        assert!(err.to_string().contains("snapshot error"));
    }
}
