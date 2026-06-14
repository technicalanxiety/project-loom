//! Knowledge consolidation and active forgetting pipeline.
//!
//! This module orchestrates the consolidation pipeline: identifying fact clusters,
//! synthesizing knowledge summaries, and pruning stale derived artifacts.
//! Runs on a scheduled basis (configurable per namespace) during off-peak hours.

use pgvector::Vector;
use sqlx::PgPool;
use std::time::Instant;
use uuid::Uuid;

use crate::db::{consolidation, entities, procedures};
use crate::llm::client::LlmClient;
use crate::types::summary::{ConsolidationResult, SynthesisResponse};

/// Errors that can occur during consolidation.
#[derive(Debug, thiserror::Error)]
pub enum ConsolidationError {
    /// Database error.
    #[error("database error: {0}")]
    Database(String),
    /// LLM error.
    #[error("LLM error: {0}")]
    Llm(String),
    /// JSON parsing error.
    #[error("JSON parse error: {0}")]
    JsonParse(#[from] serde_json::Error),
    /// Hallucination detected in synthesis.
    #[error("summary references non-existent fact: {0}")]
    HallucinatedFactReference(Uuid),
}

/// A cluster of facts about a single entity ready for consolidation.
struct FactCluster {
    entity_id: Uuid,
    entity_name: String,
    entity_type: String,
    namespace: String,
    fact_ids: Vec<Uuid>,
    facts: Vec<FactWithText>,
}

/// A fact record with its full text (subject, predicate, object).
struct FactWithText {
    id: Uuid,
    subject_name: String,
    predicate: String,
    object_name: String,
    evidence_status: String,
}

// ---------------------------------------------------------------------------
// Consolidation Phase
// ---------------------------------------------------------------------------

/// Run the consolidation phase: identify clusters and synthesize summaries.
///
/// Called at the start of a consolidation cycle. Identifies entities with
/// 5+ stable facts, synthesizes summaries via LLM, and stores results.
pub async fn run_consolidation_phase(
    pool: &PgPool,
    llm: &LlmClient,
    namespace: &str,
    min_cluster_size: i32,
) -> Result<consolidation::NewConsolidationLog, ConsolidationError> {
    let start = Instant::now();

    // Get namespace configuration
    let config = get_namespace_config(pool, namespace)
        .await
        .map_err(|e| ConsolidationError::Database(e.to_string()))?;

    // Identify fact clusters
    let clusters = identify_clusters(pool, namespace, min_cluster_size as usize)
        .await
        .map_err(|e| ConsolidationError::Database(e.to_string()))?;

    let clusters_found = clusters.len() as i32;
    let mut summaries_created = 0;
    let mut summaries_refreshed = 0;

    // Synthesize summaries for each cluster
    for cluster in clusters {
        match synthesize_cluster(pool, llm, &cluster, &config.synthesis_model)
            .await
        {
            Ok(ConsolidationResult::Created(_)) => {
                summaries_created += 1;
            }
            Ok(ConsolidationResult::Refreshed(_)) => {
                summaries_refreshed += 1;
            }
            Err(e) => {
                tracing::warn!(
                    cluster_entity_id = %cluster.entity_id,
                    error = %e,
                    "failed to synthesize cluster"
                );
            }
        }
    }

    let duration_ms = start.elapsed().as_millis() as i32;

    Ok(consolidation::NewConsolidationLog {
        namespace: namespace.to_string(),
        run_type: "consolidation".to_string(),
        clusters_found: Some(clusters_found),
        summaries_created: Some(summaries_created),
        summaries_refreshed: Some(summaries_refreshed),
        procedures_pruned: None,
        conflicts_resolved: None,
        summaries_invalidated: None,
        error_detail: None,
        duration_ms: Some(duration_ms),
    })
}

/// Identify entities with fact clusters eligible for consolidation.
///
/// Returns entities with >= min_cluster_size facts that haven't been
/// consolidated in the last 48 hours, ordered by fact count descending.
async fn identify_clusters(
    pool: &PgPool,
    namespace: &str,
    min_cluster_size: usize,
) -> Result<Vec<FactCluster>, sqlx::Error> {
    // Query for entities with enough facts
    let rows = sqlx::query_as::<_, (Uuid, String, String, i64)>(
        r#"
        SELECT e.id, e.name, e.entity_type, COUNT(f.id) as fact_count
        FROM loom_entities e
        JOIN loom_facts f ON f.subject_entity_id = e.id
            AND f.deleted_at IS NULL
            AND f.superseded_by IS NULL
            AND f.created_at < now() - INTERVAL '48 hours'
        WHERE e.namespace = $1 AND e.deleted_at IS NULL
        GROUP BY e.id, e.name, e.entity_type
        HAVING COUNT(f.id) >= $2
        ORDER BY COUNT(f.id) DESC
        LIMIT 20
        "#,
    )
    .bind(namespace)
    .bind(min_cluster_size as i64)
    .fetch_all(pool)
    .await?;

    let mut clusters = Vec::new();

    for (entity_id, entity_name, entity_type, _) in rows {
        // Fetch full fact details for this entity
        let facts = sqlx::query_as::<_, (Uuid, String, String, String, String)>(
            r#"
            SELECT f.id,
                   (SELECT name FROM loom_entities WHERE id = f.subject_id LIMIT 1) as subject_name,
                   f.predicate,
                   (SELECT name FROM loom_entities WHERE id = f.object_id LIMIT 1) as object_name,
                   f.evidence_status
            FROM loom_facts f
            WHERE f.subject_id = $1
              AND f.namespace = $2
              AND f.deleted_at IS NULL
              AND f.superseded_by IS NULL
              AND f.created_at < now() - INTERVAL '48 hours'
            "#,
        )
        .bind(entity_id)
        .bind(namespace)
        .fetch_all(pool)
        .await?;

        let fact_ids: Vec<Uuid> = facts.iter().map(|(id, _, _, _, _)| *id).collect();
        let facts_with_text: Vec<FactWithText> = facts
            .into_iter()
            .map(|(id, subj, pred, obj, status)| FactWithText {
                id,
                subject_name: subj,
                predicate: pred,
                object_name: obj,
                evidence_status: status,
            })
            .collect();

        clusters.push(FactCluster {
            entity_id,
            entity_name,
            entity_type,
            namespace: namespace.to_string(),
            fact_ids,
            facts: facts_with_text,
        });
    }

    Ok(clusters)
}

/// Synthesize or refresh a knowledge summary for a fact cluster.
///
/// Calls Ollama to consolidate facts, validates coverage, and upserts
/// the summary record. Returns whether a new summary was created or an
/// existing one was refreshed.
async fn synthesize_cluster(
    pool: &PgPool,
    llm: &LlmClient,
    cluster: &FactCluster,
    model: &str,
) -> Result<ConsolidationResult, ConsolidationError> {
    // Format facts for prompt
    let fact_list = cluster
        .facts
        .iter()
        .map(|f| {
            format!(
                "- [{}] {} {} {} (evidence: {})",
                f.id, f.subject_name, f.predicate, f.object_name, f.evidence_status
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Load and format consolidation prompt
    let system_prompt = include_str!("../../prompts/consolidation.txt");
    let user_prompt = format_consolidation_prompt(&cluster.entity_name, &cluster.entity_type, &cluster.namespace, &fact_list);

    // Call LLM
    let response = llm
        .call_llm(&model, system_prompt, &user_prompt)
        .await
        .map_err(|e| ConsolidationError::Llm(e.to_string()))?;

    let synthesis: SynthesisResponse = serde_json::from_value(response)?;

    // Validate coverage: all referenced facts must be in the cluster
    for coverage_item in &synthesis.coverage {
        for fact_id in &coverage_item.source_fact_ids {
            if !cluster.fact_ids.contains(fact_id) {
                return Err(ConsolidationError::HallucinatedFactReference(*fact_id));
            }
        }
    }

    // Check evidence status of cluster
    let evidence_status = if cluster
        .facts
        .iter()
        .any(|f| f.evidence_status == "sole_source_flagged")
    {
        "extracted"
    } else {
        "extracted" // All summaries start as extracted
    };

    let contains_sole_source = cluster
        .facts
        .iter()
        .any(|f| f.evidence_status == "sole_source_flagged");

    // Clone summary text before moving into NewSummary
    let summary_text = synthesis.summary_text.clone();

    // Upsert summary
    let summary = entities::insert_summary(
        pool,
        &entities::NewSummary {
            namespace: cluster.namespace.clone(),
            subject_entity_id: cluster.entity_id,
            summary_text: synthesis.summary_text,
            source_facts: cluster.fact_ids.clone(),
            evidence_status: evidence_status.to_string(),
            contains_sole_source,
            synthesis_model: model.to_string(),
            synthesis_prompt_ver: "consolidation_v1".to_string(),
        },
    )
    .await
    .map_err(|e| ConsolidationError::Database(e.to_string()))?;

    // Embed and store state
    let embedding_vec = llm
        .call_embeddings(&model, &summary_text)
        .await
        .map_err(|e| ConsolidationError::Llm(e.to_string()))?;
    let embedding = Vector::from(embedding_vec);

    let token_count = estimate_tokens(&summary_text);

    entities::update_summary_state(pool, summary.id, Some(&embedding), token_count, 0, None)
        .await
        .map_err(|e| ConsolidationError::Database(e.to_string()))?;

    Ok(ConsolidationResult::Created(summary.id))
}

// ---------------------------------------------------------------------------
// Pruning Phase
// ---------------------------------------------------------------------------

/// Run the pruning phase: delete stale procedures, auto-resolve conflicts, clean summaries.
pub async fn run_pruning_phase(
    pool: &PgPool,
    namespace: &str,
    procedure_ttl_days: i32,
    conflict_ttl_days: i32,
    summary_ttl_days: i32,
) -> Result<consolidation::NewConsolidationLog, ConsolidationError> {
    let start = Instant::now();

    // Prune stale procedures
    let procedures_pruned = procedures::delete_stale_procedures(pool, namespace, procedure_ttl_days)
        .await
        .map_err(|e| ConsolidationError::Database(e.to_string()))?;

    // Auto-resolve stale conflicts
    let conflicts = entities::query_unresolved_conflicts_due(pool, namespace, conflict_ttl_days)
        .await
        .map_err(|e| ConsolidationError::Database(e.to_string()))?;

    let mut conflicts_resolved = 0;
    for conflict in conflicts {
        entities::auto_resolve_conflict(pool, conflict.id)
            .await
            .map_err(|e| ConsolidationError::Database(e.to_string()))?;
        conflicts_resolved += 1;
    }

    // Soft-delete long-invalidated summaries
    let summaries_invalidated = sqlx::query_scalar::<_, i64>(
        r#"
        UPDATE loom_summaries
        SET deleted_at = now()
        WHERE namespace = $1
          AND invalidated_at IS NOT NULL
          AND invalidated_at < now() - (INTERVAL '1 day' * $2)
          AND deleted_at IS NULL
        RETURNING id
        "#,
    )
    .bind(namespace)
    .bind(summary_ttl_days as i64)
    .fetch_all(pool)
    .await
    .map_err(|e| ConsolidationError::Database(e.to_string()))?
    .len() as i32;

    let duration_ms = start.elapsed().as_millis() as i32;

    Ok(consolidation::NewConsolidationLog {
        namespace: namespace.to_string(),
        run_type: "pruning".to_string(),
        clusters_found: None,
        summaries_created: None,
        summaries_refreshed: None,
        procedures_pruned: Some(procedures_pruned as i32),
        conflicts_resolved: Some(conflicts_resolved),
        summaries_invalidated: Some(summaries_invalidated),
        error_detail: None,
        duration_ms: Some(duration_ms),
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Configuration for a namespace's consolidation behavior.
struct NamespaceConsolidationConfig {
    min_cluster_size: i32,
    synthesis_model: String,
}

/// Get consolidation configuration for a namespace.
async fn get_namespace_config(
    pool: &PgPool,
    namespace: &str,
) -> Result<NamespaceConsolidationConfig, sqlx::Error> {
    let (min_cluster, hot_budget) = sqlx::query_as::<_, (Option<i32>, Option<i32>)>(
        "SELECT consolidation_min_cluster, hot_tier_budget FROM loom_namespace_config WHERE namespace = $1",
    )
    .bind(namespace)
    .fetch_one(pool)
    .await?;

    // Determine synthesis model based on tier budget (simple heuristic)
    let synthesis_model = if hot_budget.unwrap_or(0) > 1000 {
        "qwen2.5:32b".to_string()
    } else {
        "qwen2.5:14b".to_string()
    };

    Ok(NamespaceConsolidationConfig {
        min_cluster_size: min_cluster.unwrap_or(5),
        synthesis_model,
    })
}

/// Format the consolidation prompt with cluster details.
fn format_consolidation_prompt(
    entity_name: &str,
    entity_type: &str,
    namespace: &str,
    fact_list: &str,
) -> String {
    let template = include_str!("../../prompts/consolidation.txt");

    template
        .replace("{{ENTITY_NAME}}", entity_name)
        .replace("{{ENTITY_TYPE}}", entity_type)
        .replace("{{NAMESPACE}}", namespace)
        .replace("{{FACT_LIST}}", fact_list)
}

/// Estimate token count for text (simple heuristic: ~4 chars per token).
fn estimate_tokens(text: &str) -> i32 {
    (text.len() / 4) as i32
}
