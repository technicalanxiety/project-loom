//! Retrieval profile mapping and execution for the online pipeline.
//!
//! Maps [`TaskClass`] variants to retrieval profiles that determine which
//! memory stores are queried. When a secondary task class is present (i.e.
//! the confidence gap between primary and secondary was < 0.3), profiles
//! from both classes are merged, deduplicated, and capped at 3.
//!
//! Each profile is implemented as an async function that queries the database
//! using the online connection pool and returns [`RetrievalCandidate`] items.
//! The [`execute_profiles`] function runs all active profiles in parallel via
//! `tokio::join!`, merges results, and deduplicates by ID.
//!
//! # Pipeline Position
//!
//! ```text
//! loom_think → classify → namespace → **retrieve** → weight → rank → compile
//! ```
//!
//! # Retrieval Profiles
//!
//! | Profile              | Description                                      |
//! |----------------------|--------------------------------------------------|
//! | `FactLookup`         | Semantic facts matching query                    |
//! | `EpisodeRecall`      | Recent episodes by relevance + recency           |
//! | `GraphNeighborhood`  | Related entities via graph traversal              |
//! | `ProcedureAssist`    | High-confidence behavioral patterns              |

use std::collections::HashSet;

use chrono::Utc;
use pgvector::Vector;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::traverse::TraversalResult;
use crate::types::classification::TaskClass;

// ---------------------------------------------------------------------------
// Retrieval profile enum
// ---------------------------------------------------------------------------

/// Maximum number of retrieval profiles that can be active for a single
/// query. Profiles beyond this cap are dropped (least-priority first).
const MAX_PROFILES: usize = 3;

/// A retrieval profile that determines which memory store is queried.
///
/// Each profile corresponds to a distinct retrieval strategy executed in
/// parallel via `tokio::join!` during the retrieval stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalProfile {
    /// Semantic fact lookup — filters current facts by vector similarity.
    FactLookup,
    /// Episode recall — recent episodes ranked by relevance + recency.
    EpisodeRecall,
    /// Graph neighborhood — entity traversal via `loom_traverse`.
    GraphNeighborhood,
    /// Procedure assist — high-confidence behavioral patterns.
    ProcedureAssist,
}

impl std::fmt::Display for RetrievalProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::FactLookup => "fact_lookup",
            Self::EpisodeRecall => "episode_recall",
            Self::GraphNeighborhood => "graph_neighborhood",
            Self::ProcedureAssist => "procedure_assist",
        };
        write!(f, "{s}")
    }
}

// ---------------------------------------------------------------------------
// Profile mapping
// ---------------------------------------------------------------------------

/// Return the retrieval profiles for a given task class.
///
/// The mapping follows the design document:
///
/// | Task Class     | Profiles                              |
/// |----------------|---------------------------------------|
/// | Debug          | GraphNeighborhood, EpisodeRecall      |
/// | Architecture   | FactLookup, GraphNeighborhood         |
/// | Compliance     | EpisodeRecall, FactLookup             |
/// | Writing        | FactLookup                            |
/// | Chat           | FactLookup                            |
pub fn profiles_for_class(class: &TaskClass) -> Vec<RetrievalProfile> {
    match class {
        TaskClass::Debug => vec![
            RetrievalProfile::GraphNeighborhood,
            RetrievalProfile::EpisodeRecall,
        ],
        TaskClass::Architecture => vec![
            RetrievalProfile::FactLookup,
            RetrievalProfile::GraphNeighborhood,
        ],
        TaskClass::Compliance => vec![
            RetrievalProfile::EpisodeRecall,
            RetrievalProfile::FactLookup,
        ],
        TaskClass::Writing => vec![RetrievalProfile::FactLookup],
        TaskClass::Chat => vec![RetrievalProfile::FactLookup],
    }
}

// ---------------------------------------------------------------------------
// Profile merging
// ---------------------------------------------------------------------------

/// Merge retrieval profiles from primary and optional secondary task classes.
///
/// Profiles from the primary class come first, followed by any additional
/// profiles from the secondary class that are not already present. The
/// result is deduplicated and capped at [`MAX_PROFILES`] (3).
///
/// # Examples
///
/// ```
/// use loom_engine::pipeline::online::retrieve::{merge_profiles, RetrievalProfile};
/// use loom_engine::types::classification::TaskClass;
///
/// // Debug (primary) + Architecture (secondary)
/// let profiles = merge_profiles(&TaskClass::Debug, Some(&TaskClass::Architecture));
/// assert_eq!(profiles, vec![
///     RetrievalProfile::GraphNeighborhood,
///     RetrievalProfile::EpisodeRecall,
///     RetrievalProfile::FactLookup,
/// ]);
/// assert!(profiles.len() <= 3);
/// ```
pub fn merge_profiles(
    primary: &TaskClass,
    secondary: Option<&TaskClass>,
) -> Vec<RetrievalProfile> {
    let mut profiles = profiles_for_class(primary);

    if let Some(sec) = secondary {
        for profile in profiles_for_class(sec) {
            if !profiles.contains(&profile) {
                profiles.push(profile);
            }
        }
    }

    // Cap at MAX_PROFILES.
    profiles.truncate(MAX_PROFILES);

    tracing::info!(
        primary_class = %primary,
        secondary_class = secondary.map(|c| c.to_string()).unwrap_or_default(),
        profiles = ?profiles,
        count = profiles.len(),
        "retrieval profiles resolved"
    );

    profiles
}

/// Return the profile names as strings for audit logging.
pub fn profile_names(profiles: &[RetrievalProfile]) -> Vec<String> {
    profiles.iter().map(|p| p.to_string()).collect()
}

// ---------------------------------------------------------------------------
// Retrieval errors
// ---------------------------------------------------------------------------

/// Errors that can occur during retrieval profile execution.
#[derive(Debug, thiserror::Error)]
pub enum RetrievalError {
    /// An underlying database error.
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),

    /// A graph traversal error.
    #[error("traversal error: {0}")]
    Traverse(#[from] crate::db::traverse::TraverseError),

    /// A profile execution timed out.
    #[error("profile timed out: {0}")]
    Timeout(String),
}

// ---------------------------------------------------------------------------
// Retrieval candidate — unified wrapper for all profile results
// ---------------------------------------------------------------------------

/// The memory type of a retrieval candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    /// A semantic fact (subject-predicate-object triple).
    Semantic,
    /// An episodic memory (raw episode record).
    Episodic,
    /// A graph traversal result (entity + connecting fact).
    Graph,
    /// A procedural pattern.
    Procedural,
}

impl std::fmt::Display for MemoryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Semantic => "semantic",
            Self::Episodic => "episodic",
            Self::Graph => "graph",
            Self::Procedural => "procedural",
        };
        write!(f, "{s}")
    }
}

/// A unified retrieval candidate wrapping results from any profile.
///
/// Carries a common `id` for deduplication, a `score` for ranking, and
/// the originating profile. The `payload` holds the profile-specific data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalCandidate {
    /// Unique identifier used for deduplication across profiles.
    pub id: Uuid,
    /// Relevance score assigned by the originating profile (0.0–1.0).
    pub score: f64,
    /// Which profile produced this candidate.
    pub source_profile: RetrievalProfile,
    /// The memory type category.
    pub memory_type: MemoryType,
    /// Profile-specific payload.
    pub payload: CandidatePayload,
}

/// Profile-specific data carried by a [`RetrievalCandidate`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CandidatePayload {
    /// A fact from the `fact_lookup` profile.
    Fact(FactCandidate),
    /// An episode from the `episode_recall` profile.
    Episode(EpisodeCandidate),
    /// A graph traversal result from the `graph_neighborhood` profile.
    Graph(GraphCandidate),
    /// A procedure from the `procedure_assist` profile.
    Procedure(ProcedureCandidate),
}

/// Payload for a fact candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactCandidate {
    pub subject_id: Uuid,
    pub predicate: String,
    pub object_id: Uuid,
    pub evidence_status: String,
    pub source_episodes: Vec<Uuid>,
    pub namespace: String,
}

/// Payload for an episode candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodeCandidate {
    pub source: String,
    pub content: String,
    pub occurred_at: chrono::DateTime<Utc>,
    pub namespace: String,
}

/// Payload for a graph traversal candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphCandidate {
    pub entity_id: Uuid,
    pub entity_name: String,
    pub entity_type: String,
    pub fact_id: Option<Uuid>,
    pub predicate: Option<String>,
    pub hop_depth: i32,
}

/// Payload for a procedure candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcedureCandidate {
    pub pattern: String,
    pub confidence: f64,
    pub observation_count: i32,
    pub namespace: String,
}

// ---------------------------------------------------------------------------
// Profile execution metadata
// ---------------------------------------------------------------------------

/// Metadata about a single profile execution for audit logging.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileExecution {
    /// Which profile was executed.
    pub profile: RetrievalProfile,
    /// How many candidates the profile returned.
    pub candidate_count: usize,
    /// Execution duration in milliseconds.
    pub duration_ms: u64,
}

/// Result of executing all active profiles in parallel.
#[derive(Debug, Clone)]
pub struct RetrievalResult {
    /// Merged and deduplicated candidates from all profiles.
    pub candidates: Vec<RetrievalCandidate>,
    /// Per-profile execution metadata for audit logging.
    pub executions: Vec<ProfileExecution>,
    /// Names of executed profiles (convenience for audit log).
    pub executed_profile_names: Vec<String>,
}

// ---------------------------------------------------------------------------
// 10.3 — fact_lookup retrieval profile
// ---------------------------------------------------------------------------

/// Maximum number of fact candidates returned by the `fact_lookup` profile.
const FACT_LOOKUP_LIMIT: i64 = 20;

/// Execute the `fact_lookup` retrieval profile.
///
/// Filters to current facts (`valid_until IS NULL`, `deleted_at IS NULL`)
/// within the given namespace. Ranks by cosine similarity between the query
/// embedding and fact-state embeddings, with a boost for facts whose subject
/// or object entity names match query terms.
///
/// Returns up to 20 fact candidates.
pub async fn execute_fact_lookup(
    pool: &PgPool,
    query_embedding: &Vector,
    namespace: &str,
    query_terms: &[String],
) -> Result<Vec<RetrievalCandidate>, RetrievalError> {
    // Retrieve facts ranked by embedding similarity on fact_state.
    // We join loom_facts with loom_fact_state for the embedding, and with
    // loom_entities (subject + object) to enable entity-name boosting.
    let rows = sqlx::query_as::<_, FactWithScore>(
        r#"
        SELECT f.id, f.subject_id, f.predicate, f.object_id,
               f.evidence_status, f.source_episodes, f.namespace,
               1.0 - (fs.embedding <=> $1::vector) AS similarity,
               subj.name AS subject_name,
               obj.name AS object_name
        FROM loom_facts f
        JOIN loom_fact_state fs ON fs.fact_id = f.id
        JOIN loom_entities subj ON subj.id = f.subject_id
        JOIN loom_entities obj ON obj.id = f.object_id
        WHERE f.namespace = $2
          AND f.valid_until IS NULL
          AND f.deleted_at IS NULL
          AND fs.embedding IS NOT NULL
        ORDER BY fs.embedding <=> $1::vector ASC
        LIMIT $3
        "#,
    )
    .bind(query_embedding)
    .bind(namespace)
    .bind(FACT_LOOKUP_LIMIT)
    .fetch_all(pool)
    .await?;

    let candidates = rows
        .into_iter()
        .map(|row| {
            // Boost score when entity names match query terms.
            let base_score = row.similarity;
            let name_boost = compute_entity_name_boost(
                &row.subject_name,
                &row.object_name,
                query_terms,
            );
            let score = (base_score + name_boost).min(1.0);

            RetrievalCandidate {
                id: row.id,
                score,
                source_profile: RetrievalProfile::FactLookup,
                memory_type: MemoryType::Semantic,
                payload: CandidatePayload::Fact(FactCandidate {
                    subject_id: row.subject_id,
                    predicate: row.predicate,
                    object_id: row.object_id,
                    evidence_status: row.evidence_status,
                    source_episodes: row.source_episodes,
                    namespace: row.namespace,
                }),
            }
        })
        .collect();

    Ok(candidates)
}

/// Compute a boost factor when entity names match query terms.
///
/// Returns a value in `[0.0, 0.1]` — up to 0.05 per matching entity name.
fn compute_entity_name_boost(
    subject_name: &str,
    object_name: &str,
    query_terms: &[String],
) -> f64 {
    let mut boost: f64 = 0.0;
    let subject_lower = subject_name.to_lowercase();
    let object_lower = object_name.to_lowercase();

    for term in query_terms {
        let term_lower = term.to_lowercase();
        if subject_lower.contains(&term_lower) {
            boost += 0.05;
        }
        if object_lower.contains(&term_lower) {
            boost += 0.05;
        }
    }

    boost.min(0.1)
}

/// Internal row type for the fact_lookup query.
#[derive(Debug, sqlx::FromRow)]
struct FactWithScore {
    id: Uuid,
    subject_id: Uuid,
    predicate: String,
    object_id: Uuid,
    evidence_status: String,
    source_episodes: Vec<Uuid>,
    namespace: String,
    similarity: f64,
    subject_name: String,
    object_name: String,
}

// ---------------------------------------------------------------------------
// 10.4 — episode_recall retrieval profile
// ---------------------------------------------------------------------------

/// Maximum number of episode candidates returned by the `episode_recall` profile.
const EPISODE_RECALL_LIMIT: i64 = 10;

/// Recency half-life in days for the episode recall weighting.
///
/// Episodes older than this many days receive half the recency bonus.
const RECENCY_HALF_LIFE_DAYS: f64 = 30.0;

/// Execute the `episode_recall` retrieval profile.
///
/// Filters to non-deleted episodes in the given namespace. Ranks by cosine
/// similarity to the query embedding with a recency weighting that favors
/// recent `occurred_at` timestamps.
///
/// Returns up to 10 episode candidates.
pub async fn execute_episode_recall(
    pool: &PgPool,
    query_embedding: &Vector,
    namespace: &str,
) -> Result<Vec<RetrievalCandidate>, RetrievalError> {
    let rows = sqlx::query_as::<_, EpisodeWithScore>(
        r#"
        SELECT id, source, content, occurred_at, namespace,
               1.0 - (embedding <=> $1::vector) AS similarity
        FROM loom_episodes
        WHERE namespace = $2
          AND deleted_at IS NULL
          AND embedding IS NOT NULL
        ORDER BY embedding <=> $1::vector ASC
        LIMIT $3
        "#,
    )
    .bind(query_embedding)
    .bind(namespace)
    .bind(EPISODE_RECALL_LIMIT)
    .fetch_all(pool)
    .await?;

    let now = Utc::now();
    let candidates = rows
        .into_iter()
        .map(|row| {
            let similarity = row.similarity;
            let recency = compute_recency_weight(row.occurred_at, now);
            // Blend: 70% similarity + 30% recency.
            let score = (similarity * 0.7 + recency * 0.3).min(1.0);

            RetrievalCandidate {
                id: row.id,
                score,
                source_profile: RetrievalProfile::EpisodeRecall,
                memory_type: MemoryType::Episodic,
                payload: CandidatePayload::Episode(EpisodeCandidate {
                    source: row.source,
                    content: row.content,
                    occurred_at: row.occurred_at,
                    namespace: row.namespace,
                }),
            }
        })
        .collect();

    Ok(candidates)
}

/// Compute a recency weight in `[0.0, 1.0]` using exponential decay.
///
/// Recent episodes score close to 1.0; episodes older than
/// [`RECENCY_HALF_LIFE_DAYS`] score below 0.5.
fn compute_recency_weight(
    occurred_at: chrono::DateTime<Utc>,
    now: chrono::DateTime<Utc>,
) -> f64 {
    let age_days = (now - occurred_at).num_hours().max(0) as f64 / 24.0;
    let decay = (-age_days * (2.0_f64.ln()) / RECENCY_HALF_LIFE_DAYS).exp();
    decay.clamp(0.0, 1.0)
}

/// Internal row type for the episode_recall query.
#[derive(Debug, sqlx::FromRow)]
struct EpisodeWithScore {
    id: Uuid,
    source: String,
    content: String,
    occurred_at: chrono::DateTime<Utc>,
    namespace: String,
    similarity: f64,
}

// ---------------------------------------------------------------------------
// 10.5 — graph_neighborhood retrieval profile
// ---------------------------------------------------------------------------

/// Minimum number of traversal results before retrying with more hops.
const GRAPH_MIN_RESULTS: usize = 3;

/// Execute the `graph_neighborhood` retrieval profile.
///
/// Identifies entities mentioned in the query by keyword matching against
/// entity names, then traverses the knowledge graph via `loom_traverse`.
/// Starts with `max_hops=1`; if fewer than 3 results are found, retries
/// with `max_hops=2`. Filters to current facts (`valid_until IS NULL`,
/// `deleted_at IS NULL`) within the namespace.
///
/// Returns entities and connecting facts as candidates.
pub async fn execute_graph_neighborhood(
    pool: &PgPool,
    namespace: &str,
    query_terms: &[String],
) -> Result<Vec<RetrievalCandidate>, RetrievalError> {
    // Step 1: Identify entities mentioned in the query via keyword matching.
    let entity_ids = find_entities_by_query_terms(pool, namespace, query_terms).await?;

    if entity_ids.is_empty() {
        tracing::debug!(namespace, "graph_neighborhood: no entities matched query terms");
        return Ok(Vec::new());
    }

    // Step 2: Traverse from each matched entity.
    let mut all_results: Vec<TraversalResult> = Vec::new();

    for entity_id in &entity_ids {
        // Start with 1-hop.
        let results =
            crate::db::traverse::traverse(pool, *entity_id, 1, namespace).await?;

        if results.len() < GRAPH_MIN_RESULTS {
            // Retry with 2-hop if too few results.
            tracing::debug!(
                entity_id = %entity_id,
                results_1hop = results.len(),
                "graph_neighborhood: retrying with max_hops=2"
            );
            let results_2hop =
                crate::db::traverse::traverse(pool, *entity_id, 2, namespace).await?;
            all_results.extend(results_2hop);
        } else {
            all_results.extend(results);
        }
    }

    // Step 3: Deduplicate traversal results by entity_id and convert to candidates.
    let mut seen = HashSet::new();
    let candidates = all_results
        .into_iter()
        .filter(|r| seen.insert(r.entity_id))
        .map(|r| {
            // Score inversely proportional to hop depth.
            let score = match r.hop_depth {
                1 => 0.9,
                2 => 0.7,
                _ => 0.5,
            };

            // Use fact_id as the candidate ID when available, otherwise entity_id.
            let candidate_id = r.fact_id.unwrap_or(r.entity_id);

            RetrievalCandidate {
                id: candidate_id,
                score,
                source_profile: RetrievalProfile::GraphNeighborhood,
                memory_type: MemoryType::Graph,
                payload: CandidatePayload::Graph(GraphCandidate {
                    entity_id: r.entity_id,
                    entity_name: r.entity_name,
                    entity_type: r.entity_type,
                    fact_id: r.fact_id,
                    predicate: r.predicate,
                    hop_depth: r.hop_depth,
                }),
            }
        })
        .collect();

    Ok(candidates)
}

/// Find entity UUIDs whose names match any of the query terms.
///
/// Uses case-insensitive `ILIKE` matching against `loom_entities.name`.
async fn find_entities_by_query_terms(
    pool: &PgPool,
    namespace: &str,
    query_terms: &[String],
) -> Result<Vec<Uuid>, sqlx::Error> {
    if query_terms.is_empty() {
        return Ok(Vec::new());
    }

    // Build a list of ILIKE patterns from query terms.
    // We search for entities whose name contains any query term.
    let mut ids = Vec::new();
    for term in query_terms {
        let pattern = format!("%{term}%");
        let rows = sqlx::query_as::<_, EntityIdRow>(
            r#"
            SELECT id
            FROM loom_entities
            WHERE namespace = $1
              AND deleted_at IS NULL
              AND name ILIKE $2
            LIMIT 5
            "#,
        )
        .bind(namespace)
        .bind(&pattern)
        .fetch_all(pool)
        .await?;

        for row in rows {
            if !ids.contains(&row.id) {
                ids.push(row.id);
            }
        }
    }

    Ok(ids)
}

/// Minimal row type for entity ID lookups.
#[derive(Debug, sqlx::FromRow)]
struct EntityIdRow {
    id: Uuid,
}

// ---------------------------------------------------------------------------
// 10.6 — procedure_assist retrieval profile
// ---------------------------------------------------------------------------

/// Maximum number of procedure candidates returned by the `procedure_assist` profile.
const PROCEDURE_ASSIST_LIMIT: i64 = 3;

/// Minimum confidence threshold for procedure candidates.
const PROCEDURE_MIN_CONFIDENCE: f64 = 0.8;

/// Minimum observation count for procedure candidates.
const PROCEDURE_MIN_OBSERVATIONS: i32 = 3;

/// Execute the `procedure_assist` retrieval profile.
///
/// Filters to promoted procedures with `confidence >= 0.8` and
/// `observation_count >= 3` in the given namespace. Excludes soft-deleted
/// procedures. Returns up to 3 procedure candidates.
///
/// This profile is excluded for the `compliance` task class (weight 0.0),
/// but that exclusion is enforced by the weight modifier stage, not here.
pub async fn execute_procedure_assist(
    pool: &PgPool,
    namespace: &str,
    task_class: &TaskClass,
) -> Result<Vec<RetrievalCandidate>, RetrievalError> {
    // Hard-exclude for compliance task class per design doc.
    if *task_class == TaskClass::Compliance {
        tracing::debug!(
            namespace,
            "procedure_assist: excluded for compliance task class"
        );
        return Ok(Vec::new());
    }

    let rows = sqlx::query_as::<_, ProcedureRow>(
        r#"
        SELECT id, pattern, confidence, observation_count, namespace
        FROM loom_procedures
        WHERE namespace = $1
          AND evidence_status = 'promoted'
          AND confidence >= $2
          AND observation_count >= $3
          AND deleted_at IS NULL
        ORDER BY confidence DESC
        LIMIT $4
        "#,
    )
    .bind(namespace)
    .bind(PROCEDURE_MIN_CONFIDENCE)
    .bind(PROCEDURE_MIN_OBSERVATIONS)
    .bind(PROCEDURE_ASSIST_LIMIT)
    .fetch_all(pool)
    .await?;

    let candidates = rows
        .into_iter()
        .map(|row| {
            // Score based on confidence.
            let score = row.confidence.unwrap_or(0.0);

            RetrievalCandidate {
                id: row.id,
                score,
                source_profile: RetrievalProfile::ProcedureAssist,
                memory_type: MemoryType::Procedural,
                payload: CandidatePayload::Procedure(ProcedureCandidate {
                    pattern: row.pattern,
                    confidence: row.confidence.unwrap_or(0.0),
                    observation_count: row.observation_count.unwrap_or(0),
                    namespace: row.namespace,
                }),
            }
        })
        .collect();

    Ok(candidates)
}

/// Internal row type for the procedure_assist query.
#[derive(Debug, sqlx::FromRow)]
struct ProcedureRow {
    id: Uuid,
    pattern: String,
    confidence: Option<f64>,
    observation_count: Option<i32>,
    namespace: String,
}

// ---------------------------------------------------------------------------
// 10.7 — Parallel profile executor
// ---------------------------------------------------------------------------

/// Execute all active retrieval profiles in parallel via `tokio::join!`.
///
/// Merges and deduplicates candidates by ID across all profiles. Tracks
/// per-profile execution metadata for audit logging.
///
/// # Arguments
///
/// * `pool` — Online connection pool.
/// * `profiles` — Active retrieval profiles to execute.
/// * `query_embedding` — 768-dimension query embedding.
/// * `namespace` — Namespace isolation boundary.
/// * `query_terms` — Tokenized query terms for entity matching and boosting.
/// * `task_class` — Active task class (used by procedure_assist exclusion).
/// Default timeout for individual profile execution (5 seconds).
const PROFILE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Default ranking score used when a dimension computation fails.
pub const DEFAULT_RANKING_SCORE: f64 = 0.5;

#[tracing::instrument(
    skip(pool, query_embedding, query_terms),
    fields(stage = "retrieve", profile_count = profiles.len())
)]
pub async fn execute_profiles(
    pool: &PgPool,
    profiles: &[RetrievalProfile],
    query_embedding: &Vector,
    namespace: &str,
    query_terms: &[String],
    task_class: &TaskClass,
) -> Result<RetrievalResult, RetrievalError> {
    let start = std::time::Instant::now();

    // Prepare futures for each active profile. We use Option to handle
    // profiles that may not be in the active set.
    let run_fact_lookup = profiles.contains(&RetrievalProfile::FactLookup);
    let run_episode_recall = profiles.contains(&RetrievalProfile::EpisodeRecall);
    let run_graph_neighborhood = profiles.contains(&RetrievalProfile::GraphNeighborhood);
    let run_procedure_assist = profiles.contains(&RetrievalProfile::ProcedureAssist);

    // Execute all active profiles in parallel with per-profile timeouts.
    let (fact_result, episode_result, graph_result, procedure_result) = tokio::join!(
        async {
            if run_fact_lookup {
                let t = std::time::Instant::now();
                let res = tokio::time::timeout(
                    PROFILE_TIMEOUT,
                    execute_fact_lookup(pool, query_embedding, namespace, query_terms),
                ).await;
                let res = match res {
                    Ok(inner) => inner,
                    Err(_) => {
                        tracing::warn!(
                            profile = "fact_lookup",
                            timeout_secs = PROFILE_TIMEOUT.as_secs(),
                            "profile execution timed out, continuing with other profiles"
                        );
                        Err(RetrievalError::Timeout("fact_lookup timed out".into()))
                    }
                };
                Some((res, t.elapsed()))
            } else {
                None
            }
        },
        async {
            if run_episode_recall {
                let t = std::time::Instant::now();
                let res = tokio::time::timeout(
                    PROFILE_TIMEOUT,
                    execute_episode_recall(pool, query_embedding, namespace),
                ).await;
                let res = match res {
                    Ok(inner) => inner,
                    Err(_) => {
                        tracing::warn!(
                            profile = "episode_recall",
                            timeout_secs = PROFILE_TIMEOUT.as_secs(),
                            "profile execution timed out, continuing with other profiles"
                        );
                        Err(RetrievalError::Timeout("episode_recall timed out".into()))
                    }
                };
                Some((res, t.elapsed()))
            } else {
                None
            }
        },
        async {
            if run_graph_neighborhood {
                let t = std::time::Instant::now();
                let res = tokio::time::timeout(
                    PROFILE_TIMEOUT,
                    execute_graph_neighborhood(pool, namespace, query_terms),
                ).await;
                let res = match res {
                    Ok(inner) => inner,
                    Err(_) => {
                        tracing::warn!(
                            profile = "graph_neighborhood",
                            timeout_secs = PROFILE_TIMEOUT.as_secs(),
                            "profile execution timed out, continuing with other profiles"
                        );
                        Err(RetrievalError::Timeout("graph_neighborhood timed out".into()))
                    }
                };
                Some((res, t.elapsed()))
            } else {
                None
            }
        },
        async {
            if run_procedure_assist {
                let t = std::time::Instant::now();
                let res = tokio::time::timeout(
                    PROFILE_TIMEOUT,
                    execute_procedure_assist(pool, namespace, task_class),
                ).await;
                let res = match res {
                    Ok(inner) => inner,
                    Err(_) => {
                        tracing::warn!(
                            profile = "procedure_assist",
                            timeout_secs = PROFILE_TIMEOUT.as_secs(),
                            "profile execution timed out, continuing with other profiles"
                        );
                        Err(RetrievalError::Timeout("procedure_assist timed out".into()))
                    }
                };
                Some((res, t.elapsed()))
            } else {
                None
            }
        },
    );

    // Collect results and execution metadata.
    let mut all_candidates: Vec<RetrievalCandidate> = Vec::new();
    let mut executions: Vec<ProfileExecution> = Vec::new();

    // Helper macro to process each profile result.
    macro_rules! collect_profile {
        ($result:expr, $profile:expr) => {
            if let Some((res, duration)) = $result {
                match res {
                    Ok(candidates) => {
                        let count = candidates.len();
                        tracing::info!(
                            profile = %$profile,
                            candidates = count,
                            duration_ms = duration.as_millis() as u64,
                            "retrieval profile completed"
                        );
                        executions.push(ProfileExecution {
                            profile: $profile,
                            candidate_count: count,
                            duration_ms: duration.as_millis() as u64,
                        });
                        all_candidates.extend(candidates);
                    }
                    Err(e) => {
                        tracing::error!(
                            profile = %$profile,
                            error = %e,
                            "retrieval profile failed"
                        );
                        executions.push(ProfileExecution {
                            profile: $profile,
                            candidate_count: 0,
                            duration_ms: duration.as_millis() as u64,
                        });
                    }
                }
            }
        };
    }

    collect_profile!(fact_result, RetrievalProfile::FactLookup);
    collect_profile!(episode_result, RetrievalProfile::EpisodeRecall);
    collect_profile!(graph_result, RetrievalProfile::GraphNeighborhood);
    collect_profile!(procedure_result, RetrievalProfile::ProcedureAssist);

    // Deduplicate candidates by ID, keeping the highest-scoring entry.
    let mut seen_ids: HashSet<Uuid> = HashSet::new();
    let mut deduped: Vec<RetrievalCandidate> = Vec::with_capacity(all_candidates.len());
    // Sort by score descending so the first occurrence of each ID is the best.
    all_candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    for candidate in all_candidates {
        if seen_ids.insert(candidate.id) {
            deduped.push(candidate);
        }
    }

    let executed_names = executions.iter().map(|e| e.profile.to_string()).collect::<Vec<_>>();

    tracing::info!(
        total_candidates = deduped.len(),
        profiles_executed = ?executed_names,
        total_duration_ms = start.elapsed().as_millis() as u64,
        "retrieval profiles execution complete"
    );

    Ok(RetrievalResult {
        candidates: deduped,
        executions,
        executed_profile_names: executed_names,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- profiles_for_class -------------------------------------------------

    #[test]
    fn debug_profiles() {
        let profiles = profiles_for_class(&TaskClass::Debug);
        assert_eq!(
            profiles,
            vec![
                RetrievalProfile::GraphNeighborhood,
                RetrievalProfile::EpisodeRecall,
            ]
        );
    }

    #[test]
    fn architecture_profiles() {
        let profiles = profiles_for_class(&TaskClass::Architecture);
        assert_eq!(
            profiles,
            vec![
                RetrievalProfile::FactLookup,
                RetrievalProfile::GraphNeighborhood,
            ]
        );
    }

    #[test]
    fn compliance_profiles() {
        let profiles = profiles_for_class(&TaskClass::Compliance);
        assert_eq!(
            profiles,
            vec![
                RetrievalProfile::EpisodeRecall,
                RetrievalProfile::FactLookup,
            ]
        );
    }

    #[test]
    fn writing_profiles() {
        let profiles = profiles_for_class(&TaskClass::Writing);
        assert_eq!(profiles, vec![RetrievalProfile::FactLookup]);
    }

    #[test]
    fn chat_profiles() {
        let profiles = profiles_for_class(&TaskClass::Chat);
        assert_eq!(profiles, vec![RetrievalProfile::FactLookup]);
    }

    // -- merge_profiles -----------------------------------------------------

    #[test]
    fn merge_primary_only() {
        let profiles = merge_profiles(&TaskClass::Debug, None);
        assert_eq!(
            profiles,
            vec![
                RetrievalProfile::GraphNeighborhood,
                RetrievalProfile::EpisodeRecall,
            ]
        );
    }

    #[test]
    fn merge_debug_and_architecture() {
        // Debug: [GraphNeighborhood, EpisodeRecall]
        // Architecture: [FactLookup, GraphNeighborhood]
        // Merged: [GraphNeighborhood, EpisodeRecall, FactLookup] (3 — at cap)
        let profiles = merge_profiles(&TaskClass::Debug, Some(&TaskClass::Architecture));
        assert_eq!(
            profiles,
            vec![
                RetrievalProfile::GraphNeighborhood,
                RetrievalProfile::EpisodeRecall,
                RetrievalProfile::FactLookup,
            ]
        );
    }

    #[test]
    fn merge_architecture_and_debug() {
        // Architecture: [FactLookup, GraphNeighborhood]
        // Debug: [GraphNeighborhood, EpisodeRecall]
        // Merged: [FactLookup, GraphNeighborhood, EpisodeRecall] (3 — at cap)
        let profiles = merge_profiles(&TaskClass::Architecture, Some(&TaskClass::Debug));
        assert_eq!(
            profiles,
            vec![
                RetrievalProfile::FactLookup,
                RetrievalProfile::GraphNeighborhood,
                RetrievalProfile::EpisodeRecall,
            ]
        );
    }

    #[test]
    fn merge_compliance_and_debug() {
        // Compliance: [EpisodeRecall, FactLookup]
        // Debug: [GraphNeighborhood, EpisodeRecall]
        // Merged: [EpisodeRecall, FactLookup, GraphNeighborhood] (3 — at cap)
        let profiles = merge_profiles(&TaskClass::Compliance, Some(&TaskClass::Debug));
        assert_eq!(
            profiles,
            vec![
                RetrievalProfile::EpisodeRecall,
                RetrievalProfile::FactLookup,
                RetrievalProfile::GraphNeighborhood,
            ]
        );
    }

    #[test]
    fn merge_writing_and_compliance() {
        // Writing: [FactLookup]
        // Compliance: [EpisodeRecall, FactLookup]
        // Merged: [FactLookup, EpisodeRecall] (2 — under cap)
        let profiles = merge_profiles(&TaskClass::Writing, Some(&TaskClass::Compliance));
        assert_eq!(
            profiles,
            vec![
                RetrievalProfile::FactLookup,
                RetrievalProfile::EpisodeRecall,
            ]
        );
    }

    #[test]
    fn merge_chat_and_writing() {
        // Chat: [FactLookup]
        // Writing: [FactLookup]
        // Merged: [FactLookup] (1 — both have same profile, deduped)
        let profiles = merge_profiles(&TaskClass::Chat, Some(&TaskClass::Writing));
        assert_eq!(profiles, vec![RetrievalProfile::FactLookup]);
    }

    #[test]
    fn merge_same_class_deduplicates() {
        let profiles = merge_profiles(&TaskClass::Debug, Some(&TaskClass::Debug));
        assert_eq!(
            profiles,
            vec![
                RetrievalProfile::GraphNeighborhood,
                RetrievalProfile::EpisodeRecall,
            ]
        );
    }

    // -- cap at 3 -----------------------------------------------------------

    #[test]
    fn merge_never_exceeds_three_profiles() {
        // Exhaustive check: all primary × secondary combinations.
        let classes = [
            TaskClass::Debug,
            TaskClass::Architecture,
            TaskClass::Compliance,
            TaskClass::Writing,
            TaskClass::Chat,
        ];

        for primary in &classes {
            for secondary in &classes {
                let profiles = merge_profiles(primary, Some(secondary));
                assert!(
                    profiles.len() <= 3,
                    "merge({primary}, {secondary}) produced {} profiles: {profiles:?}",
                    profiles.len()
                );
            }
            // Also test with no secondary.
            let profiles = merge_profiles(primary, None);
            assert!(
                profiles.len() <= 3,
                "merge({primary}, None) produced {} profiles: {profiles:?}",
                profiles.len()
            );
        }
    }

    // -- deduplication ------------------------------------------------------

    #[test]
    fn merge_no_duplicate_profiles() {
        let classes = [
            TaskClass::Debug,
            TaskClass::Architecture,
            TaskClass::Compliance,
            TaskClass::Writing,
            TaskClass::Chat,
        ];

        for primary in &classes {
            for secondary in &classes {
                let profiles = merge_profiles(primary, Some(secondary));
                let unique: std::collections::HashSet<_> = profiles.iter().collect();
                assert_eq!(
                    profiles.len(),
                    unique.len(),
                    "merge({primary}, {secondary}) has duplicates: {profiles:?}"
                );
            }
        }
    }

    // -- profile_names ------------------------------------------------------

    #[test]
    fn profile_names_returns_string_names() {
        let profiles = vec![
            RetrievalProfile::FactLookup,
            RetrievalProfile::GraphNeighborhood,
        ];
        let names = profile_names(&profiles);
        assert_eq!(names, vec!["fact_lookup", "graph_neighborhood"]);
    }

    #[test]
    fn profile_names_empty() {
        let names = profile_names(&[]);
        assert!(names.is_empty());
    }

    // -- Display impl -------------------------------------------------------

    #[test]
    fn retrieval_profile_display() {
        assert_eq!(RetrievalProfile::FactLookup.to_string(), "fact_lookup");
        assert_eq!(RetrievalProfile::EpisodeRecall.to_string(), "episode_recall");
        assert_eq!(
            RetrievalProfile::GraphNeighborhood.to_string(),
            "graph_neighborhood"
        );
        assert_eq!(
            RetrievalProfile::ProcedureAssist.to_string(),
            "procedure_assist"
        );
    }

    // -- serde roundtrip ----------------------------------------------------

    #[test]
    fn retrieval_profile_serde_roundtrip() {
        let profile = RetrievalProfile::GraphNeighborhood;
        let json = serde_json::to_string(&profile).expect("serialize");
        assert_eq!(json, "\"graph_neighborhood\"");

        let deserialized: RetrievalProfile =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, profile);
    }

    #[test]
    fn all_profiles_serde_roundtrip() {
        let profiles = [
            RetrievalProfile::FactLookup,
            RetrievalProfile::EpisodeRecall,
            RetrievalProfile::GraphNeighborhood,
            RetrievalProfile::ProcedureAssist,
        ];

        for profile in profiles {
            let json = serde_json::to_string(&profile).expect("serialize");
            let back: RetrievalProfile =
                serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, profile);
        }
    }

    // -- entity name boost --------------------------------------------------

    #[test]
    fn entity_name_boost_no_match() {
        let boost = compute_entity_name_boost("Rust", "PostgreSQL", &[]);
        assert!((boost - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn entity_name_boost_subject_match() {
        let terms = vec!["rust".to_string()];
        let boost = compute_entity_name_boost("Rust", "PostgreSQL", &terms);
        assert!((boost - 0.05).abs() < f64::EPSILON);
    }

    #[test]
    fn entity_name_boost_both_match() {
        let terms = vec!["rust".to_string(), "postgres".to_string()];
        let boost = compute_entity_name_boost("Rust", "PostgreSQL", &terms);
        // "rust" matches subject (0.05), "postgres" matches object (0.05) = 0.10
        assert!((boost - 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn entity_name_boost_capped_at_0_1() {
        // Even with many matching terms, boost should not exceed 0.1.
        let terms = vec![
            "rust".to_string(),
            "postgres".to_string(),
            "sql".to_string(),
        ];
        let boost = compute_entity_name_boost("Rust SQL", "PostgreSQL", &terms);
        assert!(boost <= 0.1 + f64::EPSILON);
    }

    // -- recency weight -----------------------------------------------------

    #[test]
    fn recency_weight_now_is_one() {
        let now = Utc::now();
        let weight = compute_recency_weight(now, now);
        assert!((weight - 1.0).abs() < 0.01);
    }

    #[test]
    fn recency_weight_half_life() {
        let now = Utc::now();
        let half_life_ago = now - chrono::Duration::days(RECENCY_HALF_LIFE_DAYS as i64);
        let weight = compute_recency_weight(half_life_ago, now);
        // Should be approximately 0.5.
        assert!(
            (weight - 0.5).abs() < 0.05,
            "expected ~0.5, got {weight}"
        );
    }

    #[test]
    fn recency_weight_very_old_is_near_zero() {
        let now = Utc::now();
        let old = now - chrono::Duration::days(365);
        let weight = compute_recency_weight(old, now);
        assert!(weight < 0.01, "expected near 0, got {weight}");
    }

    // -- memory type display ------------------------------------------------

    #[test]
    fn memory_type_display() {
        assert_eq!(MemoryType::Semantic.to_string(), "semantic");
        assert_eq!(MemoryType::Episodic.to_string(), "episodic");
        assert_eq!(MemoryType::Graph.to_string(), "graph");
        assert_eq!(MemoryType::Procedural.to_string(), "procedural");
    }

    // -- retrieval candidate serde ------------------------------------------

    #[test]
    fn retrieval_candidate_serde_roundtrip() {
        let candidate = RetrievalCandidate {
            id: Uuid::new_v4(),
            score: 0.85,
            source_profile: RetrievalProfile::FactLookup,
            memory_type: MemoryType::Semantic,
            payload: CandidatePayload::Fact(FactCandidate {
                subject_id: Uuid::new_v4(),
                predicate: "uses".to_string(),
                object_id: Uuid::new_v4(),
                evidence_status: "extracted".to_string(),
                source_episodes: vec![Uuid::new_v4()],
                namespace: "default".to_string(),
            }),
        };

        let json = serde_json::to_string(&candidate).expect("serialize");
        let back: RetrievalCandidate =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, candidate.id);
        assert!((back.score - candidate.score).abs() < f64::EPSILON);
        assert_eq!(back.source_profile, candidate.source_profile);
        assert_eq!(back.memory_type, candidate.memory_type);
    }

    #[test]
    fn episode_candidate_serde_roundtrip() {
        let candidate = RetrievalCandidate {
            id: Uuid::new_v4(),
            score: 0.72,
            source_profile: RetrievalProfile::EpisodeRecall,
            memory_type: MemoryType::Episodic,
            payload: CandidatePayload::Episode(EpisodeCandidate {
                source: "claude-code".to_string(),
                content: "Discussed APIM auth flow".to_string(),
                occurred_at: Utc::now(),
                namespace: "default".to_string(),
            }),
        };

        let json = serde_json::to_string(&candidate).expect("serialize");
        let back: RetrievalCandidate =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.source_profile, RetrievalProfile::EpisodeRecall);
        assert_eq!(back.memory_type, MemoryType::Episodic);
    }

    #[test]
    fn graph_candidate_serde_roundtrip() {
        let candidate = RetrievalCandidate {
            id: Uuid::new_v4(),
            score: 0.9,
            source_profile: RetrievalProfile::GraphNeighborhood,
            memory_type: MemoryType::Graph,
            payload: CandidatePayload::Graph(GraphCandidate {
                entity_id: Uuid::new_v4(),
                entity_name: "APIM".to_string(),
                entity_type: "service".to_string(),
                fact_id: Some(Uuid::new_v4()),
                predicate: Some("uses".to_string()),
                hop_depth: 1,
            }),
        };

        let json = serde_json::to_string(&candidate).expect("serialize");
        let back: RetrievalCandidate =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.source_profile, RetrievalProfile::GraphNeighborhood);
    }

    #[test]
    fn procedure_candidate_serde_roundtrip() {
        let candidate = RetrievalCandidate {
            id: Uuid::new_v4(),
            score: 0.85,
            source_profile: RetrievalProfile::ProcedureAssist,
            memory_type: MemoryType::Procedural,
            payload: CandidatePayload::Procedure(ProcedureCandidate {
                pattern: "Check APIM logs first".to_string(),
                confidence: 0.85,
                observation_count: 5,
                namespace: "default".to_string(),
            }),
        };

        let json = serde_json::to_string(&candidate).expect("serialize");
        let back: RetrievalCandidate =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.source_profile, RetrievalProfile::ProcedureAssist);
        assert_eq!(back.memory_type, MemoryType::Procedural);
    }

    // -- retrieval error display --------------------------------------------

    #[test]
    fn retrieval_error_displays_message() {
        let err = RetrievalError::Sqlx(sqlx::Error::RowNotFound);
        assert!(err.to_string().contains("database error"));
    }

    // -- profile execution metadata -----------------------------------------

    #[test]
    fn profile_execution_metadata() {
        let exec = ProfileExecution {
            profile: RetrievalProfile::FactLookup,
            candidate_count: 15,
            duration_ms: 42,
        };
        assert_eq!(exec.profile, RetrievalProfile::FactLookup);
        assert_eq!(exec.candidate_count, 15);
        assert_eq!(exec.duration_ms, 42);
    }

    // -- 10.11 additional unit tests ----------------------------------------

    /// Test that each task class produces a non-empty profile list
    /// (Requirement 9.1-9.5).
    #[test]
    fn all_task_classes_produce_non_empty_profiles() {
        let classes = [
            TaskClass::Debug,
            TaskClass::Architecture,
            TaskClass::Compliance,
            TaskClass::Writing,
            TaskClass::Chat,
        ];

        for class in &classes {
            let profiles = profiles_for_class(class);
            assert!(
                !profiles.is_empty(),
                "profiles_for_class({}) should not be empty",
                class
            );
        }
    }

    /// Test that Writing and Chat both map to exactly FactLookup
    /// (Requirement 9.4, 9.5).
    #[test]
    fn writing_and_chat_map_to_fact_lookup_only() {
        let writing = profiles_for_class(&TaskClass::Writing);
        let chat = profiles_for_class(&TaskClass::Chat);

        assert_eq!(writing.len(), 1);
        assert_eq!(chat.len(), 1);
        assert_eq!(writing[0], RetrievalProfile::FactLookup);
        assert_eq!(chat[0], RetrievalProfile::FactLookup);
    }

    /// Test that Debug includes GraphNeighborhood (Requirement 12.1).
    #[test]
    fn debug_includes_graph_neighborhood() {
        let profiles = profiles_for_class(&TaskClass::Debug);
        assert!(
            profiles.contains(&RetrievalProfile::GraphNeighborhood),
            "Debug should include GraphNeighborhood for graph traversal"
        );
    }

    /// Test that Compliance includes EpisodeRecall (Requirement 11.1).
    #[test]
    fn compliance_includes_episode_recall() {
        let profiles = profiles_for_class(&TaskClass::Compliance);
        assert!(
            profiles.contains(&RetrievalProfile::EpisodeRecall),
            "Compliance should include EpisodeRecall for audit trail"
        );
    }

    /// Test that merging all 5×5 class combinations never produces
    /// ProcedureAssist in the profile list (it's not in any class mapping)
    /// (Requirement 34.1).
    #[test]
    fn procedure_assist_never_in_class_profiles() {
        let classes = [
            TaskClass::Debug,
            TaskClass::Architecture,
            TaskClass::Compliance,
            TaskClass::Writing,
            TaskClass::Chat,
        ];

        for primary in &classes {
            let profiles = profiles_for_class(primary);
            assert!(
                !profiles.contains(&RetrievalProfile::ProcedureAssist),
                "profiles_for_class({}) should not contain ProcedureAssist",
                primary
            );
        }
    }

    /// Test that merging profiles from two classes with overlapping profiles
    /// deduplicates correctly (Requirement 9.6, 9.7).
    #[test]
    fn merge_overlapping_profiles_deduplicates() {
        // Architecture: [FactLookup, GraphNeighborhood]
        // Compliance: [EpisodeRecall, FactLookup]
        // FactLookup appears in both — should appear only once.
        let profiles = merge_profiles(
            &TaskClass::Architecture,
            Some(&TaskClass::Compliance),
        );

        let fact_lookup_count = profiles
            .iter()
            .filter(|p| **p == RetrievalProfile::FactLookup)
            .count();
        assert_eq!(
            fact_lookup_count, 1,
            "FactLookup should appear exactly once after merge"
        );
        assert!(profiles.len() <= 3);
    }

    /// Test that the entity name boost is case-insensitive
    /// (Requirement 10.1).
    #[test]
    fn entity_name_boost_case_insensitive() {
        let terms = vec!["RUST".to_string()];
        let boost = compute_entity_name_boost("rust", "PostgreSQL", &terms);
        assert!(boost > 0.0, "Case-insensitive match should produce boost");
    }

    /// Test that recency weight is monotonically decreasing with age.
    #[test]
    fn recency_weight_monotonically_decreasing() {
        let now = Utc::now();
        let recent = now - chrono::Duration::days(1);
        let older = now - chrono::Duration::days(30);
        let oldest = now - chrono::Duration::days(90);

        let w_recent = compute_recency_weight(recent, now);
        let w_older = compute_recency_weight(older, now);
        let w_oldest = compute_recency_weight(oldest, now);

        assert!(
            w_recent > w_older,
            "Recent ({}) should score higher than older ({})",
            w_recent,
            w_older
        );
        assert!(
            w_older > w_oldest,
            "Older ({}) should score higher than oldest ({})",
            w_older,
            w_oldest
        );
    }

    /// Test that recency weight is always in [0.0, 1.0].
    #[test]
    fn recency_weight_bounded() {
        let now = Utc::now();
        let test_ages = [0, 1, 7, 30, 90, 365, 1000];

        for days in test_ages {
            let occurred = now - chrono::Duration::days(days);
            let weight = compute_recency_weight(occurred, now);
            assert!(
                (0.0..=1.0).contains(&weight),
                "Recency weight {} for {} days should be in [0.0, 1.0]",
                weight,
                days
            );
        }
    }
}
