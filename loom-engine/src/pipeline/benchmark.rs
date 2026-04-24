//! Benchmark evaluation runner for A/B/C condition comparison.
//!
//! Implements the benchmark evaluation protocol (Requirement 47) by running
//! 10+ benchmark tasks across three conditions:
//! - **Condition A**: No memory (baseline) — returns empty context immediately.
//! - **Condition B**: Episode-only retrieval — embeds the query, runs
//!   `episode_recall`, weights, ranks, and compiles. No LLM classification.
//! - **Condition C**: Full Loom pipeline — embeds the query, runs all retrieval
//!   profiles for the task class (inferred from the task name prefix), weights,
//!   ranks, and compiles. Includes hot-tier items.
//!
//! All three conditions execute against the real database and the real embedding
//! model. If the `benchmark` namespace has no data, B and C will produce empty
//! compilations — which is the correct result, not a bug.
//!
//! **Precision** is measured as the fraction of `expected_entities` whose names
//! appear (case-insensitive substring) in the compiled context package. This is
//! a text-presence metric, not an LLM judge. It is conservative: entities that
//! appear in retrieved facts but under a different form will be missed.
//!
//! **Task success** is defined as: at least one token was compiled (i.e. the
//! pipeline returned non-empty context). For condition A this is always false.

use std::time::Instant;

use chrono::Utc;
use pgvector::Vector;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::AppConfig;
use crate::db::pool::DbPools;
use crate::llm::client::LlmClient;
use crate::llm::embeddings;
use crate::pipeline::online::{
    compile::{self, CompilationInput, DEFAULT_WARM_TIER_BUDGET, HotEntity, HotFact, HotTierItem},
    rank, retrieve, weight,
};
use crate::pipeline::online::retrieve::MemoryType;
use crate::types::benchmark::{
    BenchmarkComparison, BenchmarkComparisonSummary, BenchmarkCondition, BenchmarkResult,
    BenchmarkRunSummary, BenchmarkTask, ConditionSummary,
};
use crate::types::classification::TaskClass;
use crate::types::compilation::OutputFormat;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur during benchmark execution.
#[derive(Debug, thiserror::Error)]
pub enum BenchmarkError {
    /// Database error during benchmark operations.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    /// Benchmark run not found.
    #[error("benchmark run not found: {0}")]
    NotFound(Uuid),
}

// ---------------------------------------------------------------------------
// Task definitions
// ---------------------------------------------------------------------------

/// The 10 benchmark tasks covering all 5 task classes.
///
/// Each task has a realistic query, a target namespace, and expected entities
/// whose presence in the compiled context is used as the precision signal.
/// All tasks use the `benchmark` namespace — populate it with representative
/// episodes before running to get meaningful B/C results.
pub fn benchmark_tasks() -> Vec<BenchmarkTask> {
    vec![
        BenchmarkTask {
            name: "debug_auth_failure".to_string(),
            query: "Why is the authentication failing for the APIM gateway?".to_string(),
            namespace: "benchmark".to_string(),
            expected_entities: vec!["APIM".to_string(), "Auth Service".to_string()],
            expected_facts: vec!["uses".to_string(), "deployed_to".to_string()],
        },
        BenchmarkTask {
            name: "debug_memory_leak".to_string(),
            query: "Investigate the memory leak in the worker service".to_string(),
            namespace: "benchmark".to_string(),
            expected_entities: vec!["Worker Service".to_string(), "Redis Cache".to_string()],
            expected_facts: vec!["depends_on".to_string(), "monitors".to_string()],
        },
        BenchmarkTask {
            name: "arch_service_topology".to_string(),
            query: "What is the service topology for the payment system?".to_string(),
            namespace: "benchmark".to_string(),
            expected_entities: vec!["Payment Service".to_string(), "Stripe Gateway".to_string()],
            expected_facts: vec!["communicates_with".to_string(), "owns".to_string()],
        },
        BenchmarkTask {
            name: "arch_data_flow".to_string(),
            query: "How does data flow from ingestion to the analytics dashboard?".to_string(),
            namespace: "benchmark".to_string(),
            expected_entities: vec!["Data Pipeline".to_string(), "Analytics Dashboard".to_string()],
            expected_facts: vec!["produces".to_string(), "consumes".to_string()],
        },
        BenchmarkTask {
            name: "compliance_gdpr_audit".to_string(),
            query: "Show the GDPR compliance audit trail for user data processing".to_string(),
            namespace: "benchmark".to_string(),
            expected_entities: vec!["User Data Store".to_string(), "GDPR Policy".to_string()],
            expected_facts: vec!["complies_with".to_string(), "governs".to_string()],
        },
        BenchmarkTask {
            name: "compliance_access_review".to_string(),
            query: "Review access control decisions for the production database".to_string(),
            namespace: "benchmark".to_string(),
            expected_entities: vec!["Production DB".to_string(), "IAM Policy".to_string()],
            expected_facts: vec!["authorized_by".to_string(), "restricts".to_string()],
        },
        BenchmarkTask {
            name: "writing_api_docs".to_string(),
            query: "Generate API documentation for the notification service endpoints".to_string(),
            namespace: "benchmark".to_string(),
            expected_entities: vec!["Notification Service".to_string(), "REST API".to_string()],
            expected_facts: vec!["exposes".to_string(), "implements".to_string()],
        },
        BenchmarkTask {
            name: "writing_runbook".to_string(),
            query: "Write a runbook for the deployment pipeline rollback procedure".to_string(),
            namespace: "benchmark".to_string(),
            expected_entities: vec!["CI/CD Pipeline".to_string(), "Kubernetes Cluster".to_string()],
            expected_facts: vec!["deploys_to".to_string(), "manages".to_string()],
        },
        BenchmarkTask {
            name: "chat_project_status".to_string(),
            query: "What is the current status of Project Sentinel?".to_string(),
            namespace: "benchmark".to_string(),
            expected_entities: vec!["Project Sentinel".to_string()],
            expected_facts: vec!["has_status".to_string()],
        },
        BenchmarkTask {
            name: "chat_team_ownership".to_string(),
            query: "Who owns the billing module?".to_string(),
            namespace: "benchmark".to_string(),
            expected_entities: vec!["Billing Module".to_string(), "Platform Team".to_string()],
            expected_facts: vec!["owns".to_string(), "maintains".to_string()],
        },
    ]
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Infer task class from the task name prefix.
fn task_class_from_name(name: &str) -> TaskClass {
    if name.starts_with("debug_") {
        TaskClass::Debug
    } else if name.starts_with("arch_") {
        TaskClass::Architecture
    } else if name.starts_with("compliance_") {
        TaskClass::Compliance
    } else if name.starts_with("writing_") {
        TaskClass::Writing
    } else {
        TaskClass::Chat
    }
}

/// Fraction of `expected_entities` whose names appear in the compiled context.
fn precision_from_context(context: &str, expected_entities: &[String]) -> f64 {
    if expected_entities.is_empty() {
        return 0.0;
    }
    let ctx = context.to_lowercase();
    let matched = expected_entities
        .iter()
        .filter(|e| ctx.contains(&e.to_lowercase()))
        .count();
    matched as f64 / expected_entities.len() as f64
}

/// Warm-tier token budget for the given namespace, falling back to the default.
async fn warm_budget(pool: &PgPool, namespace: &str) -> usize {
    let budget: Option<i32> = sqlx::query_scalar(
        "SELECT warm_tier_budget FROM loom_namespace_config WHERE namespace = $1",
    )
    .bind(namespace)
    .fetch_optional(pool)
    .await
    .unwrap_or(None);
    budget.map(|b| b as usize).unwrap_or(DEFAULT_WARM_TIER_BUDGET)
}

// Row types for the hot-tier queries below.
#[derive(sqlx::FromRow)]
struct HotFactRow {
    id: Uuid,
    subject: String,
    predicate: String,
    object: String,
    evidence_status: String,
    valid_from: chrono::DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct HotEntityRow {
    id: Uuid,
    name: String,
    entity_type: String,
    summary: Option<String>,
}

/// Fetch hot-tier items (facts + entities) for a namespace.
async fn fetch_hot_tier(pool: &PgPool, namespace: &str) -> Vec<HotTierItem> {
    let mut items = Vec::new();

    if let Ok(rows) = sqlx::query_as::<_, HotFactRow>(
        r#"
        SELECT f.id, e_subj.name AS subject, f.predicate, e_obj.name AS object,
               f.evidence_status, f.valid_from
        FROM loom_facts f
        JOIN loom_fact_state fs ON fs.fact_id = f.id
        JOIN loom_entities e_subj ON e_subj.id = f.subject_id
        JOIN loom_entities e_obj  ON e_obj.id  = f.object_id
        WHERE f.namespace = $1
          AND f.valid_until IS NULL
          AND f.deleted_at IS NULL
          AND fs.tier = 'hot'
        "#,
    )
    .bind(namespace)
    .fetch_all(pool)
    .await
    {
        for r in rows {
            items.push(HotTierItem {
                id: r.id,
                memory_type: MemoryType::Semantic,
                payload: compile::HotTierPayload::Fact(HotFact {
                    subject: r.subject,
                    predicate: r.predicate,
                    object: r.object,
                    evidence: r.evidence_status,
                    observed: Some(r.valid_from.format("%Y-%m-%d").to_string()),
                    source: r.id.to_string(),
                }),
            });
        }
    }

    if let Ok(rows) = sqlx::query_as::<_, HotEntityRow>(
        r#"
        SELECT e.id, e.name, e.entity_type, es.summary
        FROM loom_entities e
        JOIN loom_entity_state es ON es.entity_id = e.id
        WHERE e.namespace = $1
          AND e.deleted_at IS NULL
          AND es.tier = 'hot'
        "#,
    )
    .bind(namespace)
    .fetch_all(pool)
    .await
    {
        for r in rows {
            items.push(HotTierItem {
                id: r.id,
                memory_type: MemoryType::Semantic,
                payload: compile::HotTierPayload::Entity(HotEntity {
                    name: r.name,
                    entity_type: r.entity_type,
                    summary: r.summary,
                }),
            });
        }
    }

    items
}

// ---------------------------------------------------------------------------
// Condition runners
// ---------------------------------------------------------------------------

type CondResult = (f64, i32, bool, i32, serde_json::Value);

/// Condition A: no memory. Returns immediately with empty context.
fn run_condition_a(task: &BenchmarkTask) -> CondResult {
    (
        0.0,
        0,
        false,
        0,
        json!({
            "query": task.query,
            "condition_description": "No memory baseline — context always empty",
        }),
    )
}

/// Condition B: episode-only retrieval. Embeds the query, runs the
/// `episode_recall` profile, weights, ranks, and compiles. No hot tier.
async fn run_condition_b(
    task: &BenchmarkTask,
    pool: &PgPool,
    llm_client: &LlmClient,
    config: &AppConfig,
) -> CondResult {
    let start = Instant::now();

    let emb_vec = match embeddings::generate_embedding(llm_client, &config.llm, &task.query).await {
        Ok(v) => v,
        Err(e) => {
            return (
                0.0,
                0,
                false,
                start.elapsed().as_millis() as i32,
                json!({ "error": format!("embedding: {e}"), "query": task.query }),
            );
        }
    };
    let query_embedding = Vector::from(emb_vec);

    let candidates =
        match retrieve::execute_episode_recall(pool, &query_embedding, &task.namespace).await {
            Ok(c) => c,
            Err(e) => {
                return (
                    0.0,
                    0,
                    false,
                    start.elapsed().as_millis() as i32,
                    json!({ "error": format!("episode recall: {e}"), "query": task.query }),
                );
            }
        };

    let task_class = task_class_from_name(&task.name);
    let candidates_found = candidates.len();
    let weighted = weight::apply_weights(candidates, &task_class);
    let ranked = rank::rank_candidates(weighted);

    let input = CompilationInput {
        namespace: task.namespace.clone(),
        task_class,
        target_model: "benchmark".to_string(),
        format: OutputFormat::Compact,
        warm_tier_budget: warm_budget(pool, &task.namespace).await,
        hot_tier_items: vec![],
        ranked_candidates: ranked,
    };

    let result = compile::compile_package(input);
    let latency = start.elapsed().as_millis() as i32;
    let precision = precision_from_context(&result.package.context_package, &task.expected_entities);
    let token_count = result.package.token_count as i32;

    (
        precision,
        token_count,
        token_count > 0,
        latency,
        json!({
            "query": task.query,
            "condition_description": "Episode-only retrieval",
            "candidates_found": candidates_found,
            "candidates_selected": result.selected_items.len(),
        }),
    )
}

/// Condition C: full Loom pipeline. Embeds the query, runs all retrieval
/// profiles for the inferred task class, weights, ranks, and compiles with
/// hot-tier items. Classification is skipped — the task class is inferred
/// from the task name prefix to keep benchmark latency deterministic.
async fn run_condition_c(
    task: &BenchmarkTask,
    pool: &PgPool,
    llm_client: &LlmClient,
    config: &AppConfig,
) -> CondResult {
    let start = Instant::now();

    let emb_vec = match embeddings::generate_embedding(llm_client, &config.llm, &task.query).await {
        Ok(v) => v,
        Err(e) => {
            return (
                0.0,
                0,
                false,
                start.elapsed().as_millis() as i32,
                json!({ "error": format!("embedding: {e}"), "query": task.query }),
            );
        }
    };
    let query_embedding = Vector::from(emb_vec);

    let query_terms: Vec<String> = task
        .query
        .split_whitespace()
        .filter(|t| t.len() >= 3)
        .map(|t| t.to_lowercase())
        .collect();

    let task_class = task_class_from_name(&task.name);
    let profiles = retrieve::merge_profiles(&task_class, None);
    let profile_names = retrieve::profile_names(&profiles);

    let retrieval = match retrieve::execute_profiles(
        pool,
        &profiles,
        &query_embedding,
        &task.namespace,
        &query_terms,
        &task_class,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            return (
                0.0,
                0,
                false,
                start.elapsed().as_millis() as i32,
                json!({ "error": format!("retrieval: {e}"), "query": task.query }),
            );
        }
    };

    let candidates_found = retrieval.candidates.len();
    let weighted = weight::apply_weights(retrieval.candidates, &task_class);
    let ranked = rank::rank_candidates(weighted);
    let hot_tier_items = fetch_hot_tier(pool, &task.namespace).await;

    let input = CompilationInput {
        namespace: task.namespace.clone(),
        task_class: task_class.clone(),
        target_model: "benchmark".to_string(),
        format: OutputFormat::Compact,
        warm_tier_budget: warm_budget(pool, &task.namespace).await,
        hot_tier_items,
        ranked_candidates: ranked,
    };

    let result = compile::compile_package(input);
    let latency = start.elapsed().as_millis() as i32;
    let precision = precision_from_context(&result.package.context_package, &task.expected_entities);
    let token_count = result.package.token_count as i32;

    (
        precision,
        token_count,
        token_count > 0,
        latency,
        json!({
            "query": task.query,
            "condition_description": "Full Loom pipeline",
            "task_class": task_class.to_string(),
            "profiles": profile_names,
            "candidates_found": candidates_found,
            "candidates_selected": result.selected_items.len(),
        }),
    )
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Execute a full benchmark run across all tasks and all three conditions.
///
/// Requires an active embedding model. If the `benchmark` namespace contains
/// no episodes, conditions B and C will produce zero-token compilations — that
/// is accurate, not a defect.
pub async fn execute_benchmark(
    pools: &DbPools,
    llm_client: &LlmClient,
    config: &AppConfig,
) -> Result<BenchmarkRunSummary, BenchmarkError> {
    let pool = &pools.online;
    let run_id = Uuid::new_v4();
    let run_name = format!("Benchmark {}", Utc::now().format("%Y-%m-%d %H:%M"));
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO loom_benchmark_runs (id, name, created_at, status) VALUES ($1, $2, $3, 'running')",
    )
    .bind(run_id)
    .bind(&run_name)
    .bind(now)
    .execute(pool)
    .await?;

    let tasks = benchmark_tasks();

    for task in &tasks {
        let conditions: [(BenchmarkCondition, CondResult); 3] = [
            (BenchmarkCondition::A, run_condition_a(task)),
            (
                BenchmarkCondition::B,
                run_condition_b(task, pool, llm_client, config).await,
            ),
            (
                BenchmarkCondition::C,
                run_condition_c(task, pool, llm_client, config).await,
            ),
        ];

        for (condition, (precision, token_count, task_success, latency_ms, details)) in conditions {
            sqlx::query(
                r#"
                INSERT INTO loom_benchmark_results
                    (id, run_id, task_name, condition, precision, token_count,
                     task_success, latency_ms, details, created_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                "#,
            )
            .bind(Uuid::new_v4())
            .bind(run_id)
            .bind(&task.name)
            .bind(condition.to_string())
            .bind(precision)
            .bind(token_count)
            .bind(task_success)
            .bind(latency_ms)
            .bind(&details)
            .bind(Utc::now())
            .execute(pool)
            .await?;
        }
    }

    sqlx::query("UPDATE loom_benchmark_runs SET status = 'completed' WHERE id = $1")
        .bind(run_id)
        .execute(pool)
        .await?;

    Ok(BenchmarkRunSummary {
        id: run_id,
        name: run_name,
        created_at: now,
        status: "completed".to_string(),
    })
}

/// List all benchmark runs ordered by most recent first.
pub async fn list_benchmark_runs(pool: &PgPool) -> Result<Vec<BenchmarkRunSummary>, BenchmarkError> {
    let rows = sqlx::query_as::<_, BenchmarkRunRow>(
        "SELECT id, name, created_at, status FROM loom_benchmark_runs ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| BenchmarkRunSummary {
            id: r.id,
            name: r.name,
            created_at: r.created_at,
            status: r.status,
        })
        .collect())
}

/// Get full benchmark comparison for a specific run.
pub async fn get_benchmark_detail(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<BenchmarkComparison, BenchmarkError> {
    let run_row = sqlx::query_as::<_, BenchmarkRunRow>(
        "SELECT id, name, created_at, status FROM loom_benchmark_runs WHERE id = $1",
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await?
    .ok_or(BenchmarkError::NotFound(run_id))?;

    let run = BenchmarkRunSummary {
        id: run_row.id,
        name: run_row.name,
        created_at: run_row.created_at,
        status: run_row.status,
    };

    let result_rows = sqlx::query_as::<_, BenchmarkResultRow>(
        r#"
        SELECT id, run_id, task_name, condition, precision, token_count,
               task_success, latency_ms, details, created_at
        FROM loom_benchmark_results
        WHERE run_id = $1
        ORDER BY task_name, condition
        "#,
    )
    .bind(run_id)
    .fetch_all(pool)
    .await?;

    let results: Vec<BenchmarkResult> = result_rows
        .into_iter()
        .map(|r| BenchmarkResult {
            id: r.id,
            run_id: r.run_id,
            task_name: r.task_name,
            condition: r.condition,
            precision: r.precision,
            token_count: r.token_count,
            task_success: r.task_success,
            latency_ms: r.latency_ms,
            details: r.details.unwrap_or_else(|| json!({})),
            created_at: r.created_at,
        })
        .collect();

    let summary = compute_summary(&results);

    Ok(BenchmarkComparison { run, results, summary })
}

// ---------------------------------------------------------------------------
// Summary computation
// ---------------------------------------------------------------------------

fn compute_summary(results: &[BenchmarkResult]) -> BenchmarkComparisonSummary {
    BenchmarkComparisonSummary {
        condition_a: summarize_condition(results, "A"),
        condition_b: summarize_condition(results, "B"),
        condition_c: summarize_condition(results, "C"),
    }
}

fn summarize_condition(results: &[BenchmarkResult], condition: &str) -> ConditionSummary {
    let filtered: Vec<&BenchmarkResult> =
        results.iter().filter(|r| r.condition == condition).collect();

    if filtered.is_empty() {
        return ConditionSummary {
            avg_precision: 0.0,
            avg_token_count: 0.0,
            success_rate: 0.0,
            avg_latency_ms: 0.0,
        };
    }

    let count = filtered.len() as f64;
    ConditionSummary {
        avg_precision: filtered.iter().map(|r| r.precision).sum::<f64>() / count,
        avg_token_count: filtered.iter().map(|r| r.token_count as f64).sum::<f64>() / count,
        success_rate: filtered.iter().filter(|r| r.task_success).count() as f64 / count,
        avg_latency_ms: filtered.iter().map(|r| r.latency_ms as f64).sum::<f64>() / count,
    }
}

// ---------------------------------------------------------------------------
// Internal row types for sqlx queries
// ---------------------------------------------------------------------------

#[derive(Debug, sqlx::FromRow)]
struct BenchmarkRunRow {
    id: Uuid,
    name: String,
    created_at: chrono::DateTime<Utc>,
    status: String,
}

#[derive(Debug, sqlx::FromRow)]
struct BenchmarkResultRow {
    id: Uuid,
    run_id: Uuid,
    task_name: String,
    condition: String,
    precision: f64,
    token_count: i32,
    task_success: bool,
    latency_ms: i32,
    details: Option<serde_json::Value>,
    created_at: chrono::DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benchmark_tasks_has_at_least_10() {
        let tasks = benchmark_tasks();
        assert!(
            tasks.len() >= 10,
            "expected at least 10 benchmark tasks, got {}",
            tasks.len()
        );
    }

    #[test]
    fn benchmark_tasks_cover_all_classes() {
        let tasks = benchmark_tasks();
        let names: Vec<&str> = tasks.iter().map(|t| t.name.as_str()).collect();
        assert!(names.iter().any(|n| n.starts_with("debug_")), "missing debug tasks");
        assert!(names.iter().any(|n| n.starts_with("arch_")), "missing architecture tasks");
        assert!(names.iter().any(|n| n.starts_with("compliance_")), "missing compliance tasks");
        assert!(names.iter().any(|n| n.starts_with("writing_")), "missing writing tasks");
        assert!(names.iter().any(|n| n.starts_with("chat_")), "missing chat tasks");
    }

    #[test]
    fn task_class_inferred_from_prefix() {
        assert_eq!(task_class_from_name("debug_auth_failure"), TaskClass::Debug);
        assert_eq!(task_class_from_name("arch_service_topology"), TaskClass::Architecture);
        assert_eq!(task_class_from_name("compliance_gdpr_audit"), TaskClass::Compliance);
        assert_eq!(task_class_from_name("writing_api_docs"), TaskClass::Writing);
        assert_eq!(task_class_from_name("chat_project_status"), TaskClass::Chat);
    }

    #[test]
    fn precision_zero_when_context_empty() {
        let entities = vec!["Entity A".to_string(), "Entity B".to_string()];
        assert_eq!(precision_from_context("", &entities), 0.0);
    }

    #[test]
    fn precision_full_when_all_entities_present() {
        let entities = vec!["APIM".to_string(), "Auth Service".to_string()];
        let ctx = "The APIM gateway routes requests through Auth Service for validation.";
        assert_eq!(precision_from_context(ctx, &entities), 1.0);
    }

    #[test]
    fn precision_partial_match() {
        let entities = vec!["APIM".to_string(), "Auth Service".to_string(), "Redis".to_string()];
        let ctx = "The APIM gateway delegates to Auth Service.";
        let p = precision_from_context(ctx, &entities);
        assert!((p - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn precision_case_insensitive() {
        let entities = vec!["apim".to_string()];
        assert_eq!(precision_from_context("The APIM gateway", &entities), 1.0);
    }

    #[test]
    fn condition_a_always_zero() {
        let task = &benchmark_tasks()[0];
        let (precision, tokens, success, latency, _) = run_condition_a(task);
        assert_eq!(precision, 0.0);
        assert_eq!(tokens, 0);
        assert!(!success);
        assert_eq!(latency, 0);
    }

    #[test]
    fn all_tasks_use_benchmark_namespace() {
        for task in benchmark_tasks() {
            assert_eq!(task.namespace, "benchmark", "task {} uses wrong namespace", task.name);
        }
    }
}
