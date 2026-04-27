//! Benchmark evaluation runner for A/B/C condition comparison.
//!
//! Implements the benchmark evaluation protocol (Requirement 47) by running
//! 10+ benchmark tasks across three conditions, each of which actually calls
//! the LLM so the cards measure something the operator can act on:
//!
//! - **Condition A** — no memory baseline. The query is sent to the LLM with
//!   no retrieved context; the answer reflects whatever the model knows from
//!   training alone. This is the lower bound: how well does the LLM do without
//!   Loom? See ADR docs for context — until this rewrite, A was a hardcoded
//!   zero stub which made the comparison meaningless.
//! - **Condition B** — episode-only retrieval. Embed the query, run
//!   `episode_recall`, weight, rank, compile (no hot tier), pass the compiled
//!   context to the LLM, then measure what the LLM produced.
//! - **Condition C** — full Loom pipeline. Same as B but runs every retrieval
//!   profile for the inferred task class, includes hot tier, and the warmer
//!   weighting/ranking that production uses.
//!
//! Two precision metrics are recorded per condition:
//!
//! - `precision` (the headline number, stored as the `precision` column) =
//!   **answer precision**: fraction of `expected_entities` mentioned in the
//!   LLM's answer. Comparable across A/B/C because all three call the LLM.
//! - `details.retrieval_precision` (B and C only) = **predicate-aware
//!   retrieval precision**: average of (entity recall) and (predicate recall)
//!   across the retrieved candidate set. Names are hydrated from
//!   `loom_entities` so warm-tier facts contribute properly — the previous
//!   substring-on-JSON metric saw UUIDs and always returned zero for warm
//!   facts.
//!
//! `task_success` = answer precision > 0 (at least one expected entity
//! mentioned). `token_count` = compiled-context input tokens (0 for A,
//! whatever the compiler emitted for B/C).
//!
//! Empty `benchmark` namespace? B and C will compile the empty wrapper and
//! the LLM will be asked to answer with no context — the answer will likely
//! score similarly to A. The fix is to seed the namespace; see
//! `seed/benchmark/`.

use std::collections::{HashMap, HashSet};
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
    compile::{
        self, CompilationInput, DEFAULT_WARM_TIER_BUDGET, HotEntity, HotFact, HotTierItem,
        HotTierPayload,
    },
    rank::{self, RankedCandidate},
    retrieve::{self, CandidatePayload},
    weight,
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

/// Fraction of `expected_entities` whose names appear in `text`. Used for the
/// answer-side metric across all three conditions and as the entity-recall
/// component of the retrieval-side metric.
fn entity_match_fraction(text: &str, expected_entities: &[String]) -> f64 {
    if expected_entities.is_empty() {
        return 0.0;
    }
    let lc = text.to_lowercase();
    let matched = expected_entities
        .iter()
        .filter(|e| lc.contains(&e.to_lowercase()))
        .count();
    matched as f64 / expected_entities.len() as f64
}

/// Predicate-aware retrieval score for B and C.
#[derive(Debug, Clone)]
struct RetrievalScore {
    /// Fraction of `expected_entities` found across the hydrated retrieval set
    /// (hot tier identity/facts + warm-tier facts with hydrated names + episode
    /// content + graph entity names).
    entity_recall: f64,
    /// Fraction of `expected_facts` predicates present in the retrieved set
    /// (hot-tier facts + warm-tier fact predicates + graph predicates).
    predicate_recall: f64,
    /// Average of the two — the headline retrieval precision number.
    combined: f64,
    /// Number of retrieved warm-tier candidates considered (after compile-time
    /// budget filtering). Useful for diagnosing empty-namespace runs in
    /// `details`.
    candidates_considered: usize,
}

/// Compute predicate-aware retrieval precision over the items that survived
/// the compilation budget. Hydrates entity names by UUID so warm-tier facts
/// can contribute to entity recall — the previous substring-on-JSON metric
/// saw UUIDs and always returned zero for warm facts.
async fn compute_retrieval_score(
    pool: &PgPool,
    namespace: &str,
    selected_ids: &[Uuid],
    ranked: &[RankedCandidate],
    hot_items: &[HotTierItem],
    expected_entities: &[String],
    expected_facts: &[String],
) -> RetrievalScore {
    let selected: HashSet<Uuid> = selected_ids.iter().copied().collect();
    let survived: Vec<&RankedCandidate> = ranked
        .iter()
        .filter(|rc| selected.contains(&rc.candidate.id))
        .collect();

    // Collect entity UUIDs referenced by surviving fact candidates so we can
    // hydrate them in a single batch query.
    let mut entity_ids: HashSet<Uuid> = HashSet::new();
    for rc in &survived {
        if let CandidatePayload::Fact(f) = &rc.candidate.payload {
            entity_ids.insert(f.subject_id);
            entity_ids.insert(f.object_id);
        }
    }

    let names: HashMap<Uuid, String> = if entity_ids.is_empty() {
        HashMap::new()
    } else {
        let ids_vec: Vec<Uuid> = entity_ids.into_iter().collect();
        match sqlx::query_as::<_, (Uuid, String)>(
            "SELECT id, name FROM loom_entities WHERE id = ANY($1) AND namespace = $2 AND deleted_at IS NULL",
        )
        .bind(&ids_vec)
        .bind(namespace)
        .fetch_all(pool)
        .await
        {
            Ok(rows) => rows.into_iter().collect(),
            Err(e) => {
                tracing::warn!(error = %e, "benchmark: entity name hydration failed");
                HashMap::new()
            }
        }
    };

    let mut entity_corpus = String::new();
    let mut predicates: HashSet<String> = HashSet::new();

    // Hot tier: facts already carry name strings; entity payload contributes
    // its name + summary to the identity corpus.
    for item in hot_items {
        match &item.payload {
            HotTierPayload::Fact(f) => {
                entity_corpus.push_str(&f.subject);
                entity_corpus.push(' ');
                entity_corpus.push_str(&f.object);
                entity_corpus.push(' ');
                predicates.insert(f.predicate.clone());
            }
            HotTierPayload::Entity(e) => {
                entity_corpus.push_str(&e.name);
                entity_corpus.push(' ');
                if let Some(s) = &e.summary {
                    entity_corpus.push_str(s);
                    entity_corpus.push(' ');
                }
            }
            HotTierPayload::Procedure(_) => {}
        }
    }

    // Warm tier: hydrate fact subject/object names; episodes contribute their
    // verbatim content; graph candidates carry entity_name and an optional
    // predicate.
    for rc in &survived {
        match &rc.candidate.payload {
            CandidatePayload::Fact(f) => {
                if let Some(n) = names.get(&f.subject_id) {
                    entity_corpus.push_str(n);
                    entity_corpus.push(' ');
                }
                if let Some(n) = names.get(&f.object_id) {
                    entity_corpus.push_str(n);
                    entity_corpus.push(' ');
                }
                predicates.insert(f.predicate.clone());
            }
            CandidatePayload::Episode(ep) => {
                entity_corpus.push_str(&ep.content);
                entity_corpus.push(' ');
            }
            CandidatePayload::Graph(g) => {
                entity_corpus.push_str(&g.entity_name);
                entity_corpus.push(' ');
                if let Some(p) = &g.predicate {
                    predicates.insert(p.clone());
                }
            }
            CandidatePayload::Procedure(_) => {}
        }
    }

    let entity_recall = entity_match_fraction(&entity_corpus, expected_entities);

    let predicates_lc: HashSet<String> = predicates.iter().map(|p| p.to_lowercase()).collect();
    let predicate_recall = if expected_facts.is_empty() {
        0.0
    } else {
        let matched = expected_facts
            .iter()
            .filter(|p| predicates_lc.contains(&p.to_lowercase()))
            .count();
        matched as f64 / expected_facts.len() as f64
    };

    let combined = (entity_recall + predicate_recall) / 2.0;
    RetrievalScore {
        entity_recall,
        predicate_recall,
        combined,
        candidates_considered: survived.len(),
    }
}

/// System prompt for benchmark queries. Identical for all three conditions
/// except that A omits the "Context" line — keeps the model's prior behavior
/// constant so we are measuring the marginal effect of retrieved context.
fn build_system_prompt(context: Option<&str>) -> String {
    let base = "You are a technical expert assistant. Answer the user's question in 1-3 \
                sentences. When relevant systems, services, modules, projects, or teams \
                are involved, mention them by their proper names.";
    match context {
        Some(ctx) if !ctx.is_empty() => format!(
            "{base} Use only the JSON context below; if it does not answer the question, \
             say so plainly without inventing names.\n\nContext:\n{ctx}"
        ),
        _ => format!(
            "{base} If you do not know specific named systems for this question, say so \
             plainly rather than inventing names."
        ),
    }
}

/// Call the LLM and return the answer as a plain string. Errors are surfaced
/// to the caller so the condition runner can record them in `details` while
/// keeping the run going.
async fn ask_llm(
    llm: &LlmClient,
    config: &AppConfig,
    context: Option<&str>,
    query: &str,
) -> Result<String, crate::llm::client::LlmError> {
    let system = build_system_prompt(context);
    let resp = llm
        .call_llm(&config.llm.classification_model, &system, query)
        .await?;
    let text = match resp {
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    };
    Ok(text)
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

/// Condition runner result. `precision` is answer-side; `details` carries the
/// full diagnostic blob including retrieval-side metrics for B/C.
type CondResult = (f64, i32, bool, i32, serde_json::Value);

/// Condition A: no memory. Sends the bare query to the LLM and measures
/// whether the model mentions expected entities purely from training-data
/// recall. This is the lower bound that B and C are compared against.
async fn run_condition_a(
    task: &BenchmarkTask,
    llm_client: &LlmClient,
    config: &AppConfig,
) -> CondResult {
    let start = Instant::now();

    let answer = match ask_llm(llm_client, config, None, &task.query).await {
        Ok(a) => a,
        Err(e) => {
            return (
                0.0,
                0,
                false,
                start.elapsed().as_millis() as i32,
                json!({
                    "query": task.query,
                    "condition_description": "No memory baseline — LLM-only",
                    "error": format!("llm: {e}"),
                }),
            );
        }
    };

    let latency = start.elapsed().as_millis() as i32;
    let precision = entity_match_fraction(&answer, &task.expected_entities);
    let success = precision > 0.0;

    (
        precision,
        0, // No compiled context — zero input-context tokens.
        success,
        latency,
        json!({
            "query": task.query,
            "condition_description": "No memory baseline — LLM-only",
            "answer": answer,
            "answer_precision": precision,
        }),
    )
}

/// Condition B: episode-only retrieval + LLM. Embeds the query, runs
/// `episode_recall`, compiles a context with no hot tier, then asks the LLM
/// using that context.
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
        ranked_candidates: ranked.clone(),
    };

    let result = compile::compile_package(input);
    let token_count = result.package.token_count as i32;

    let selected_ids: Vec<Uuid> = result.selected_items.iter().map(|s| s.id).collect();
    let retrieval = compute_retrieval_score(
        pool,
        &task.namespace,
        &selected_ids,
        &ranked,
        &[],
        &task.expected_entities,
        &task.expected_facts,
    )
    .await;

    let answer = match ask_llm(
        llm_client,
        config,
        Some(&result.package.context_package),
        &task.query,
    )
    .await
    {
        Ok(a) => a,
        Err(e) => {
            let latency = start.elapsed().as_millis() as i32;
            return (
                0.0,
                token_count,
                false,
                latency,
                json!({
                    "query": task.query,
                    "condition_description": "Episode-only retrieval",
                    "candidates_found": candidates_found,
                    "candidates_selected": result.selected_items.len(),
                    "retrieval_precision": retrieval.combined,
                    "entity_recall": retrieval.entity_recall,
                    "predicate_recall": retrieval.predicate_recall,
                    "error": format!("llm: {e}"),
                }),
            );
        }
    };

    let latency = start.elapsed().as_millis() as i32;
    let precision = entity_match_fraction(&answer, &task.expected_entities);
    let success = precision > 0.0;

    (
        precision,
        token_count,
        success,
        latency,
        json!({
            "query": task.query,
            "condition_description": "Episode-only retrieval",
            "candidates_found": candidates_found,
            "candidates_selected": result.selected_items.len(),
            "candidates_considered": retrieval.candidates_considered,
            "answer": answer,
            "answer_precision": precision,
            "retrieval_precision": retrieval.combined,
            "entity_recall": retrieval.entity_recall,
            "predicate_recall": retrieval.predicate_recall,
        }),
    )
}

/// Condition C: full Loom pipeline + LLM. Embeds the query, runs all
/// retrieval profiles for the inferred task class, weights, ranks, compiles
/// with hot-tier items, then asks the LLM. Classification is skipped — the
/// task class is inferred from the task name prefix to keep benchmark
/// latency deterministic.
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

    let retrieval_result = match retrieve::execute_profiles(
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

    let candidates_found = retrieval_result.candidates.len();
    let weighted = weight::apply_weights(retrieval_result.candidates, &task_class);
    let ranked = rank::rank_candidates(weighted);
    let hot_tier_items = fetch_hot_tier(pool, &task.namespace).await;

    let input = CompilationInput {
        namespace: task.namespace.clone(),
        task_class: task_class.clone(),
        target_model: "benchmark".to_string(),
        format: OutputFormat::Compact,
        warm_tier_budget: warm_budget(pool, &task.namespace).await,
        hot_tier_items: hot_tier_items.clone(),
        ranked_candidates: ranked.clone(),
    };

    let result = compile::compile_package(input);
    let token_count = result.package.token_count as i32;

    let selected_ids: Vec<Uuid> = result.selected_items.iter().map(|s| s.id).collect();
    let retrieval = compute_retrieval_score(
        pool,
        &task.namespace,
        &selected_ids,
        &ranked,
        &hot_tier_items,
        &task.expected_entities,
        &task.expected_facts,
    )
    .await;

    let answer = match ask_llm(
        llm_client,
        config,
        Some(&result.package.context_package),
        &task.query,
    )
    .await
    {
        Ok(a) => a,
        Err(e) => {
            let latency = start.elapsed().as_millis() as i32;
            return (
                0.0,
                token_count,
                false,
                latency,
                json!({
                    "query": task.query,
                    "condition_description": "Full Loom pipeline",
                    "task_class": task_class.to_string(),
                    "profiles": profile_names,
                    "candidates_found": candidates_found,
                    "candidates_selected": result.selected_items.len(),
                    "retrieval_precision": retrieval.combined,
                    "entity_recall": retrieval.entity_recall,
                    "predicate_recall": retrieval.predicate_recall,
                    "error": format!("llm: {e}"),
                }),
            );
        }
    };

    let latency = start.elapsed().as_millis() as i32;
    let precision = entity_match_fraction(&answer, &task.expected_entities);
    let success = precision > 0.0;

    (
        precision,
        token_count,
        success,
        latency,
        json!({
            "query": task.query,
            "condition_description": "Full Loom pipeline",
            "task_class": task_class.to_string(),
            "profiles": profile_names,
            "candidates_found": candidates_found,
            "candidates_selected": result.selected_items.len(),
            "candidates_considered": retrieval.candidates_considered,
            "answer": answer,
            "answer_precision": precision,
            "retrieval_precision": retrieval.combined,
            "entity_recall": retrieval.entity_recall,
            "predicate_recall": retrieval.predicate_recall,
        }),
    )
}

// ---------------------------------------------------------------------------
// Seed corpus (embedded at compile time)
// ---------------------------------------------------------------------------

/// One markdown document from `seed/benchmark/` — content embedded at compile
/// time via `include_str!` so the engine ships with the corpus and doesn't
/// need a runtime filesystem path. The `event_id` is stable across calls so
/// the (`source`, `source_event_id`) unique constraint makes seeding
/// idempotent — clicking "Seed benchmark data" twice is harmless.
struct SeedDoc {
    /// Stable identifier used as `source_event_id`. Must not change between
    /// releases without resetting the benchmark namespace.
    event_id: &'static str,
    /// Verbatim markdown content. ADR-005: do not summarise.
    content: &'static str,
}

const SEED_DOCS: &[SeedDoc] = &[
    SeedDoc {
        event_id: "seed:benchmark/01-debug-auth-failure.md",
        content: include_str!("../../../seed/benchmark/01-debug-auth-failure.md"),
    },
    SeedDoc {
        event_id: "seed:benchmark/02-debug-memory-leak.md",
        content: include_str!("../../../seed/benchmark/02-debug-memory-leak.md"),
    },
    SeedDoc {
        event_id: "seed:benchmark/03-arch-service-topology.md",
        content: include_str!("../../../seed/benchmark/03-arch-service-topology.md"),
    },
    SeedDoc {
        event_id: "seed:benchmark/04-arch-data-flow.md",
        content: include_str!("../../../seed/benchmark/04-arch-data-flow.md"),
    },
    SeedDoc {
        event_id: "seed:benchmark/05-compliance-gdpr.md",
        content: include_str!("../../../seed/benchmark/05-compliance-gdpr.md"),
    },
    SeedDoc {
        event_id: "seed:benchmark/06-compliance-access-review.md",
        content: include_str!("../../../seed/benchmark/06-compliance-access-review.md"),
    },
    SeedDoc {
        event_id: "seed:benchmark/07-writing-api-docs.md",
        content: include_str!("../../../seed/benchmark/07-writing-api-docs.md"),
    },
    SeedDoc {
        event_id: "seed:benchmark/08-writing-runbook.md",
        content: include_str!("../../../seed/benchmark/08-writing-runbook.md"),
    },
    SeedDoc {
        event_id: "seed:benchmark/09-chat-project-status.md",
        content: include_str!("../../../seed/benchmark/09-chat-project-status.md"),
    },
    SeedDoc {
        event_id: "seed:benchmark/10-chat-team-ownership.md",
        content: include_str!("../../../seed/benchmark/10-chat-team-ownership.md"),
    },
];

/// Result of a `seed_benchmark_namespace` call. `inserted + duplicates` always
/// equals `SEED_DOCS.len()` on success — the dashboard uses these counts to
/// tell the operator "I just added 10 episodes" vs "they were already there".
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SeedSummary {
    /// Episodes newly inserted into the `benchmark` namespace.
    pub inserted: usize,
    /// Episodes that already existed (matched by content hash or
    /// `source_event_id`) and were skipped.
    pub duplicates: usize,
}

/// SHA-256 content hash, hex-encoded. Mirrors the helper in `api/rest.rs`
/// rather than coupling the two modules — both rely on the same scheme so
/// cross-source dedup keeps working.
fn content_hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Seed the `benchmark` namespace with the embedded corpus. Idempotent —
/// rerunning is a no-op once the corpus has been seeded once. Episodes are
/// posted as `user_authored_seed` (provenance coefficient 0.8) so they rank
/// the same as content posted via `cli/loom-seed.py`.
///
/// The extraction worker will pick the new episodes up on its next pending
/// poll; entities and facts won't be available until extraction settles —
/// minutes on iGPU. Operators see this on the Compilations page.
pub async fn seed_benchmark_namespace(
    pool: &PgPool,
) -> Result<SeedSummary, BenchmarkError> {
    use crate::db::episodes::{insert_episode, NewEpisode};

    let now = Utc::now();
    let mut inserted = 0usize;
    let mut duplicates = 0usize;

    for doc in SEED_DOCS {
        let hash = content_hash(doc.content);

        // Pre-check covers both dedup keys POST /api/learn uses: matching the
        // (source, source_event_id) pair (re-seed after interrupted run) or
        // matching the content_hash within the namespace (already seeded via
        // CLI with the same file).
        let already_present: bool = sqlx::query_scalar(
            "SELECT EXISTS(\
               SELECT 1 FROM loom_episodes \
               WHERE namespace = 'benchmark' \
                 AND (content_hash = $1 \
                      OR (source = 'seed' AND source_event_id = $2))\
             )",
        )
        .bind(&hash)
        .bind(doc.event_id)
        .fetch_one(pool)
        .await?;

        if already_present {
            duplicates += 1;
            continue;
        }

        let new_ep = NewEpisode {
            source: "seed".to_string(),
            source_id: None,
            source_event_id: Some(doc.event_id.to_string()),
            content: doc.content.to_string(),
            content_hash: hash,
            occurred_at: now,
            namespace: "benchmark".to_string(),
            metadata: None,
            participants: None,
            ingestion_mode: "user_authored_seed".to_string(),
            parser_version: None,
            parser_source_schema: None,
        };

        insert_episode(pool, &new_ep).await.map_err(|e| match e {
            crate::db::episodes::EpisodeError::Sqlx(err) => BenchmarkError::Database(err),
        })?;
        inserted += 1;
    }

    tracing::info!(
        inserted, duplicates, total = SEED_DOCS.len(),
        "benchmark seed corpus posted to namespace=benchmark"
    );
    Ok(SeedSummary { inserted, duplicates })
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Execute a full benchmark run across all tasks and all three conditions.
///
/// Every condition calls the chat LLM (`classification_model`) so the
/// `precision` column is comparable across A/B/C. The embedding model is also
/// required for B and C. Expect ~3 LLM calls per task — on iGPU this can be
/// several minutes for the full ten-task suite.
///
/// If the `benchmark` namespace contains no data, B and C will compile an
/// empty wrapper and the LLM will see no useful context — they will likely
/// score similarly to A. Seed the namespace from `seed/benchmark/` before
/// running.
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
            (
                BenchmarkCondition::A,
                run_condition_a(task, llm_client, config).await,
            ),
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
    fn entity_match_zero_when_text_empty() {
        let entities = vec!["Entity A".to_string(), "Entity B".to_string()];
        assert_eq!(entity_match_fraction("", &entities), 0.0);
    }

    #[test]
    fn entity_match_full_when_all_present() {
        let entities = vec!["APIM".to_string(), "Auth Service".to_string()];
        let text = "The APIM gateway routes requests through Auth Service for validation.";
        assert_eq!(entity_match_fraction(text, &entities), 1.0);
    }

    #[test]
    fn entity_match_partial() {
        let entities = vec!["APIM".to_string(), "Auth Service".to_string(), "Redis".to_string()];
        let text = "The APIM gateway delegates to Auth Service.";
        let p = entity_match_fraction(text, &entities);
        assert!((p - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn entity_match_case_insensitive() {
        let entities = vec!["apim".to_string()];
        assert_eq!(entity_match_fraction("The APIM gateway", &entities), 1.0);
    }

    #[test]
    fn entity_match_empty_expected() {
        // Defensive: if the operator forgot to set expected_entities, return
        // zero rather than dividing by zero.
        assert_eq!(entity_match_fraction("anything", &[]), 0.0);
    }

    #[test]
    fn build_system_prompt_includes_context_when_present() {
        let p = build_system_prompt(Some("{\"facts\":[]}"));
        assert!(p.contains("Context:"));
        assert!(p.contains("{\"facts\":[]}"));
    }

    #[test]
    fn build_system_prompt_no_context_when_absent() {
        let p_none = build_system_prompt(None);
        let p_empty = build_system_prompt(Some(""));
        assert!(!p_none.contains("Context:"));
        assert!(!p_empty.contains("Context:"));
    }

    #[test]
    fn all_tasks_use_benchmark_namespace() {
        for task in benchmark_tasks() {
            assert_eq!(task.namespace, "benchmark", "task {} uses wrong namespace", task.name);
        }
    }

    #[test]
    fn seed_corpus_matches_task_count() {
        // Each task in `benchmark_tasks()` has a corresponding seed file.
        // `include_str!` resolves at compile time; this test catches a
        // missing or unreadable file (which would have failed compilation)
        // and a desynchronisation between SEED_DOCS and the task list.
        assert_eq!(
            SEED_DOCS.len(),
            benchmark_tasks().len(),
            "seed corpus size must match benchmark task count"
        );
    }

    #[test]
    fn seed_corpus_files_are_non_empty() {
        for doc in SEED_DOCS {
            assert!(
                !doc.content.trim().is_empty(),
                "seed file {} embedded as empty content",
                doc.event_id
            );
            assert!(
                doc.content.len() > 200,
                "seed file {} too short ({} chars) for meaningful extraction",
                doc.event_id,
                doc.content.len()
            );
        }
    }

    #[test]
    fn seed_corpus_mentions_expected_entities() {
        // The seed corpus is supposed to make B and C return real signal —
        // every expected entity in `benchmark_tasks()` should appear in at
        // least one seed file. Catches drift between seed prose and the
        // ground-truth list when either side is edited.
        let combined: String = SEED_DOCS
            .iter()
            .map(|d| d.content.to_lowercase())
            .collect::<Vec<_>>()
            .join("\n");
        for task in benchmark_tasks() {
            for entity in &task.expected_entities {
                assert!(
                    combined.contains(&entity.to_lowercase()),
                    "expected entity {:?} (task {}) not found in any seed file",
                    entity,
                    task.name
                );
            }
        }
    }

    #[test]
    fn all_tasks_have_expected_entities_and_facts() {
        // A precision metric needs ground truth. Catch the "operator forgot
        // to populate ground truth" mode at build time so a future task
        // addition can't silently bake zero-precision into the suite.
        for task in benchmark_tasks() {
            assert!(
                !task.expected_entities.is_empty(),
                "task {} has empty expected_entities",
                task.name
            );
            assert!(
                !task.expected_facts.is_empty(),
                "task {} has empty expected_facts",
                task.name
            );
        }
    }
}
