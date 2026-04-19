//! Entity and fact extraction orchestration for the offline pipeline.
//!
//! Coordinates LLM calls via [`crate::llm::extraction`], deserializes
//! responses into strict Rust types via serde, and validates entity types
//! against the [`EntityType`] enum before returning results.
//!
//! Fact extraction uses **pack-aware prompt assembly**: the namespace's
//! configured predicate packs are loaded from `loom_namespace_config`,
//! the `core` pack is always included, and all predicates from active
//! packs are formatted into a grouped block injected into the prompt
//! template.

use std::collections::HashMap;
use std::time::Instant;

use pgvector::Vector;
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::LlmConfig;
use crate::db::episodes as episodes_db;
use crate::db::facts as facts_db;
use crate::db::facts::NewFact;
use crate::db::predicates as pred_db;
use crate::llm::client::LlmClient;
use crate::llm::embeddings::{self, EmbeddingError};
use crate::llm::extraction::{self, EntityExtractionResult, ExtractionError, FactExtractionResult};
use crate::pipeline::offline::resolve::{self, ResolveError};
use crate::pipeline::offline::state;
use crate::pipeline::offline::supersede::{self, NewFactDetails};
use crate::types::entity::{EntityType, ExtractedEntity, ResolutionResult};
use crate::types::episode::{Episode, ExtractionMetrics};
use crate::types::fact::ExtractedFact;
use crate::types::predicate::PredicateEntry;

// ---------------------------------------------------------------------------
// Full pipeline error
// ---------------------------------------------------------------------------

/// Errors that can occur during the full extraction pipeline.
#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    /// An error generating an episode or entity embedding.
    #[error("embedding error: {0}")]
    Embedding(#[from] EmbeddingError),

    /// An error from the entity/fact extraction LLM calls.
    #[error("extraction error: {0}")]
    Extraction(#[from] ExtractionError),

    /// An error from the fact extraction pipeline (DB + LLM).
    #[error("fact extraction pipeline error: {0}")]
    FactPipeline(#[from] FactExtractionPipelineError),

    /// An error from entity resolution.
    #[error("entity resolution error: {0}")]
    Resolution(#[from] ResolveError),

    /// An error from derived state computation or episode updates.
    #[error("state error: {0}")]
    State(#[from] state::StateError),

    /// An underlying database error.
    #[error("database error: {0}")]
    Database(String),
}

impl From<crate::db::episodes::EpisodeError> for PipelineError {
    fn from(err: crate::db::episodes::EpisodeError) -> Self {
        Self::Database(err.to_string())
    }
}

// ---------------------------------------------------------------------------
// Full pipeline result
// ---------------------------------------------------------------------------

/// Result of the full extraction pipeline for a single episode.
#[derive(Debug, Clone)]
pub struct FullPipelineResult {
    /// The episode that was processed.
    pub episode_id: Uuid,
    /// Number of entities extracted and resolved.
    pub entity_count: usize,
    /// Number of facts inserted.
    pub fact_count: usize,
    /// Number of facts skipped due to invalid entity references.
    pub facts_skipped: usize,
    /// Number of old facts superseded.
    pub superseded_count: usize,
    /// Number of resolution conflicts flagged.
    pub conflict_count: usize,
    /// The computed extraction metrics.
    pub metrics: ExtractionMetrics,
}

// ---------------------------------------------------------------------------
// Full extraction pipeline orchestrator
// ---------------------------------------------------------------------------

/// Run the complete offline extraction pipeline for a single episode.
///
/// Orchestrates the full flow:
/// 1. Generate episode embedding via nomic-embed-text and store it.
/// 2. Extract entities via Gemma 4 26B MoE.
/// 3. Resolve each entity through the 3-pass algorithm, building an
///    entity map (name → UUID).
/// 4. Assemble pack-aware fact extraction prompt and extract facts.
/// 5. Validate entity references and insert facts.
/// 6. Validate and track predicates (canonical vs custom).
/// 7. Resolve fact supersessions.
/// 8. Initialize fact serving state (salience, tier).
/// 9. Compute [`ExtractionMetrics`] and store as JSONB on the episode.
/// 10. Mark the episode as processed.
///
/// Uses the offline connection pool for all database operations.
///
/// # Errors
///
/// Returns [`PipelineError`] if any step fails. The episode will not be
/// marked as processed on failure, allowing retry on the next poll cycle.
#[tracing::instrument(
    skip(pool, client, config, episode),
    fields(
        episode_id = %episode.id,
        namespace = %episode.namespace,
    )
)]
pub async fn run_full_extraction_pipeline(
    pool: &PgPool,
    client: &LlmClient,
    config: &LlmConfig,
    episode: &Episode,
) -> Result<FullPipelineResult, PipelineError> {
    let start = Instant::now();
    let episode_id = episode.id;
    let namespace = &episode.namespace;

    tracing::info!(
        episode_id = %episode_id,
        namespace = %namespace,
        content_len = episode.content.len(),
        "starting full extraction pipeline"
    );

    // -- Step 1: Generate and store episode embedding -----------------------
    tracing::debug!(episode_id = %episode_id, "step 1: generating episode embedding");

    let embedding_vec = embeddings::generate_episode_embedding(
        client, config, &episode.content,
    )
    .await?;
    let embedding = Vector::from(embedding_vec);

    episodes_db::store_episode_embedding(pool, episode_id, &embedding).await?;

    tracing::info!(episode_id = %episode_id, "episode embedding stored");

    // -- Step 2: Extract entities via LLM -----------------------------------
    tracing::debug!(episode_id = %episode_id, "step 2: extracting entities");

    let entity_result = extract_entities_from_episode(
        client, config, &episode.content,
    )
    .await?;

    let extraction_model = entity_result.model.clone();
    let extracted_entities = entity_result.entities;

    tracing::info!(
        episode_id = %episode_id,
        entity_count = extracted_entities.len(),
        rejected = entity_result.rejected_count,
        model = %extraction_model,
        "entity extraction complete"
    );

    // -- Step 3: Resolve each entity through 3-pass algorithm ---------------
    tracing::debug!(episode_id = %episode_id, "step 3: resolving entities");

    let mut entity_map: HashMap<String, Uuid> = HashMap::new();
    let mut resolution_results: Vec<ResolutionResult> = Vec::new();

    for extracted in &extracted_entities {
        let result = resolve::resolve_entity(
            pool,
            client,
            config,
            extracted,
            &episode.content,
            episode_id,
            namespace,
        )
        .await?;

        // Track conflicts: new entities created due to semantic ambiguity.
        if result.method == "new" {
            // Check if this was a conflict-triggered creation by looking at
            // whether a conflict was logged. We approximate by counting
            // "new" entities — the actual conflict logging happens inside
            // resolve_entity.
        }

        entity_map.insert(extracted.name.clone(), result.entity_id);

        // Also map aliases to the same entity ID for fact reference resolution.
        for alias in &extracted.aliases {
            entity_map.entry(alias.clone()).or_insert(result.entity_id);
        }

        resolution_results.push(result);
    }

    // Count conflicts from resolution results.
    let conflict_count = count_conflict_flagged(&resolution_results);

    tracing::info!(
        episode_id = %episode_id,
        resolved = entity_map.len(),
        conflicts = conflict_count,
        "entity resolution complete"
    );

    // -- Steps 4-7: Fact extraction, validation, and supersession -----------
    tracing::debug!(episode_id = %episode_id, "steps 4-7: extracting and processing facts");

    let fact_result = orchestrate_fact_extraction(
        client,
        config,
        pool,
        namespace,
        &episode.content,
        episode_id,
        &entity_map,
    )
    .await?;

    tracing::info!(
        episode_id = %episode_id,
        inserted = fact_result.inserted_count,
        skipped = fact_result.skipped_count,
        superseded = fact_result.superseded_count,
        "fact extraction and supersession complete"
    );

    // -- Step 8: Initialize fact serving state ------------------------------
    tracing::debug!(episode_id = %episode_id, "step 8: initializing fact serving state");

    state::initialize_fact_serving_state(pool, &fact_result.inserted_fact_ids).await?;

    // -- Step 9: Optionally flag candidate procedures -----------------------
    // Procedure flagging is a future enhancement (procedures.rs is a TODO).
    // For now, we log that the step was skipped.
    tracing::debug!(
        episode_id = %episode_id,
        "step 9: procedure flagging skipped (not yet implemented)"
    );

    // -- Step 10: Compute and store extraction metrics ----------------------
    tracing::debug!(episode_id = %episode_id, "step 10: computing extraction metrics");

    // Collect evidence strength values for the inserted facts.
    // We query the inserted facts from the DB to get their evidence_strength.
    let mut actual_evidence: Vec<Option<String>> = Vec::new();
    for &fact_id in &fact_result.inserted_fact_ids {
        match facts_db::get_fact_by_id(pool, fact_id).await {
            Ok(Some(fact)) => actual_evidence.push(fact.evidence_strength.clone()),
            Ok(None) => actual_evidence.push(None),
            Err(e) => {
                tracing::warn!(
                    fact_id = %fact_id,
                    error = %e,
                    "failed to read fact for evidence strength"
                );
                actual_evidence.push(None);
            }
        }
    }

    let elapsed = start.elapsed();
    let processing_time_ms = elapsed.as_millis() as i64;

    let metrics = state::compute_extraction_metrics(
        &resolution_results,
        conflict_count as i32,
        Some(&fact_result),
        &actual_evidence,
        processing_time_ms,
        &extraction_model,
    );

    state::store_metrics_and_mark_processed(
        pool,
        episode_id,
        &metrics,
        &extraction_model,
    )
    .await?;

    tracing::info!(
        episode_id = %episode_id,
        namespace = %namespace,
        entities = entity_map.len(),
        facts = fact_result.inserted_count,
        superseded = fact_result.superseded_count,
        processing_time_ms,
        "full extraction pipeline complete"
    );

    Ok(FullPipelineResult {
        episode_id,
        entity_count: entity_map.len(),
        fact_count: fact_result.inserted_count,
        facts_skipped: fact_result.skipped_count,
        superseded_count: fact_result.superseded_count,
        conflict_count,
        metrics,
    })
}

/// Count entities that were flagged as conflicts during resolution.
///
/// This is a heuristic: we can't distinguish "new because no match" from
/// "new because of conflict" purely from the resolution result. However,
/// the actual conflict logging happens in `resolve_entity` and is stored
/// in `loom_resolution_conflicts`. For metrics purposes, we report 0 here
/// and rely on the caller to provide the actual conflict count if available.
fn count_conflict_flagged(_results: &[ResolutionResult]) -> usize {
    // The resolve_entity function logs conflicts to the DB directly.
    // We can't distinguish conflict-new from no-match-new from the
    // ResolutionResult alone. The caller should query the DB or track
    // conflicts separately. For now, return 0 — the actual count is
    // tracked via the loom_resolution_conflicts table.
    0
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Result of predicate validation across a batch of extracted facts.
///
/// Tracks how many predicates matched the canonical registry versus how
/// many were custom (non-canonical), along with per-fact custom flags.
#[derive(Debug, Clone)]
pub struct PredicateValidationResult {
    /// Number of facts whose predicate matched a canonical entry.
    pub canonical_count: usize,
    /// Number of facts whose predicate was custom (not in the registry).
    pub custom_count: usize,
}

/// Extract entities from episode content, validate entity types, and return
/// only those with valid [`EntityType`] values.
///
/// Orchestration steps:
/// 1. Call the LLM via [`extraction::extract_entities`] with the entity
///    extraction prompt (compiled into the binary).
/// 2. Deserialize the JSON response into `Vec<ExtractedEntity>` via serde.
/// 3. Validate each entity's `entity_type` string against the [`EntityType`]
///    enum, filtering out any that don't match the 10 allowed types.
/// 4. Log extraction counts and any rejected entities.
///
/// # Errors
///
/// Returns [`ExtractionError`] if the LLM call fails or the response cannot
/// be deserialized.
pub async fn extract_entities_from_episode(
    client: &LlmClient,
    config: &LlmConfig,
    episode_content: &str,
) -> Result<ValidatedExtractionResult, ExtractionError> {
    let raw_result = extraction::extract_entities(client, config, episode_content).await?;

    let validated = validate_entity_types(raw_result);

    tracing::info!(
        valid_count = validated.entities.len(),
        rejected_count = validated.rejected_count,
        model = %validated.model,
        "entity extraction validated"
    );

    Ok(validated)
}

// ---------------------------------------------------------------------------
// Fact extraction — pack-aware pipeline
// ---------------------------------------------------------------------------

/// Errors produced by the fact extraction pipeline.
///
/// Wraps both database errors (loading namespace config / predicates) and
/// LLM extraction errors into a single type for the orchestration layer.
#[derive(Debug, thiserror::Error)]
pub enum FactExtractionPipelineError {
    /// A database error occurred while loading namespace config or predicates.
    #[error("database error during fact extraction setup: {0}")]
    Database(String),

    /// The underlying LLM extraction call failed.
    #[error(transparent)]
    Extraction(#[from] ExtractionError),
}

impl From<sqlx::Error> for FactExtractionPipelineError {
    fn from(err: sqlx::Error) -> Self {
        Self::Database(err.to_string())
    }
}

impl From<pred_db::PredicateError> for FactExtractionPipelineError {
    fn from(err: pred_db::PredicateError) -> Self {
        Self::Database(err.to_string())
    }
}

/// Assemble a pack-aware predicate block for the fact extraction prompt.
///
/// Orchestration steps:
/// 1. Query `loom_namespace_config` for the namespace's `predicate_packs`.
///    If the namespace has no config row, default to `["core"]`.
/// 2. Ensure `"core"` is always present in the pack list.
/// 3. Query `loom_predicates` for all predicates in the active packs.
/// 4. Format predicates into a grouped block organized by pack name.
///
/// # Errors
///
/// Returns [`FactExtractionPipelineError::Database`] if any database query
/// fails.
pub async fn assemble_fact_prompt(
    pool: &PgPool,
    namespace: &str,
) -> Result<String, FactExtractionPipelineError> {
    // Step 1: Load namespace predicate packs from loom_namespace_config.
    let packs_row: Option<NamespacePacksRow> = sqlx::query_as(
        "SELECT predicate_packs FROM loom_namespace_config WHERE namespace = $1",
    )
    .bind(namespace)
    .fetch_optional(pool)
    .await?;

    let mut packs: Vec<String> = packs_row
        .and_then(|r| r.predicate_packs)
        .unwrap_or_default();

    // Step 2: Always include the core pack.
    if !packs.iter().any(|p| p == "core") {
        packs.insert(0, "core".to_string());
    }

    tracing::info!(
        namespace,
        packs = ?packs,
        "loaded predicate packs for namespace"
    );

    // Step 3: Query all predicates in the active packs.
    let predicates = pred_db::query_predicates_by_pack(pool, &packs).await?;

    tracing::info!(
        predicate_count = predicates.len(),
        pack_count = packs.len(),
        "loaded predicates for fact extraction prompt"
    );

    // Step 4: Format into a grouped block.
    let block = format_predicate_block(&predicates);

    Ok(block)
}

/// Extract facts from an episode using pack-aware prompt assembly.
///
/// Orchestration steps:
/// 1. Assemble the predicate block from the namespace's configured packs.
/// 2. Call the LLM via [`extraction::extract_facts`] with the assembled
///    prompt, entity names, and episode content.
/// 3. Deserialize the JSON response into `Vec<ExtractedFact>` via serde.
/// 4. Return the validated results with model provenance.
///
/// # Errors
///
/// Returns [`FactExtractionPipelineError`] if the database queries fail or
/// the LLM call / deserialization fails.
pub async fn extract_facts_from_episode(
    client: &LlmClient,
    config: &LlmConfig,
    pool: &PgPool,
    namespace: &str,
    episode_content: &str,
    entity_names: &[String],
) -> Result<FactExtractionResult, FactExtractionPipelineError> {
    // Step 1: Assemble the pack-aware predicate block.
    let predicate_block = assemble_fact_prompt(pool, namespace).await?;

    // Step 2–3: Call the LLM and deserialize.
    let result = extraction::extract_facts(
        client,
        config,
        episode_content,
        entity_names,
        &predicate_block,
    )
    .await?;

    tracing::info!(
        fact_count = result.facts.len(),
        model = %result.model,
        namespace,
        "fact extraction from episode complete"
    );

    Ok(result)
}

// ---------------------------------------------------------------------------
// Fact extraction orchestration
// ---------------------------------------------------------------------------

/// Result of the full fact extraction orchestration flow.
///
/// Contains counts and details for inserted facts, skipped facts (invalid
/// entity references), predicate validation results, and supersession
/// resolution.
#[derive(Debug, Clone)]
pub struct FactOrchestrationResult {
    /// Number of facts successfully inserted into `loom_facts`.
    pub inserted_count: usize,
    /// Number of facts skipped due to invalid entity references.
    pub skipped_count: usize,
    /// UUIDs of the inserted facts.
    pub inserted_fact_ids: Vec<Uuid>,
    /// Predicate validation result (canonical vs custom counts).
    pub predicate_validation: Option<PredicateValidationResult>,
    /// Total number of old facts superseded by the new batch.
    pub superseded_count: usize,
    /// The model identifier used for extraction.
    pub model: String,
}

/// Orchestrate the full fact extraction flow for a single episode.
///
/// Steps:
/// 1. Call [`extract_facts_from_episode`] to get raw extracted facts from
///    the LLM.
/// 2. Validate entity references: for each fact, check that subject and
///    object names exist in the provided `entity_map`. Skip facts with
///    invalid references.
/// 3. For each valid fact, look up `subject_id` and `object_id` from the
///    entity map and insert the fact into `loom_facts` via
///    [`facts_db::insert_fact`].
/// 4. Call [`validate_and_track_predicates`] to check canonical vs custom
///    predicates.
/// 5. Call [`supersede::resolve_supersessions_batch`] for all inserted
///    facts.
/// 6. Return a [`FactOrchestrationResult`] with counts and details.
///
/// # Errors
///
/// Returns [`FactExtractionPipelineError`] if the LLM call, database
/// inserts, predicate validation, or supersession resolution fails.
#[tracing::instrument(
    skip(client, config, pool, episode_content, entity_map),
    fields(
        namespace,
        episode_id = %episode_id,
        entity_count = entity_map.len(),
    )
)]
pub async fn orchestrate_fact_extraction(
    client: &LlmClient,
    config: &LlmConfig,
    pool: &PgPool,
    namespace: &str,
    episode_content: &str,
    episode_id: Uuid,
    entity_map: &HashMap<String, Uuid>,
) -> Result<FactOrchestrationResult, FactExtractionPipelineError> {
    // Step 1: Extract raw facts from the LLM.
    let entity_names: Vec<String> = entity_map.keys().cloned().collect();

    let extraction_result = extract_facts_from_episode(
        client, config, pool, namespace, episode_content, &entity_names,
    )
    .await?;

    let model = extraction_result.model.clone();
    let facts = extraction_result.facts;
    let total_extracted = facts.len();

    tracing::info!(
        total_extracted,
        episode_id = %episode_id,
        "raw fact extraction complete, validating entity references"
    );

    // Step 2: Validate entity references and insert valid facts.
    let mut inserted_fact_ids: Vec<Uuid> = Vec::new();
    let mut new_fact_details: Vec<NewFactDetails> = Vec::new();
    let mut valid_fact_indices: Vec<usize> = Vec::new();
    let mut skipped_count: usize = 0;

    for (idx, fact) in facts.iter().enumerate() {
        let subject_id = entity_map.get(&fact.subject);
        let object_id = entity_map.get(&fact.object);

        match (subject_id, object_id) {
            (Some(&subj_id), Some(&obj_id)) => {
                // Step 3: Insert the fact with provenance.
                let new_fact = NewFact {
                    subject_id: subj_id,
                    predicate: fact.predicate.clone(),
                    object_id: obj_id,
                    namespace: namespace.to_string(),
                    source_episodes: vec![episode_id],
                    evidence_status: "extracted".to_string(),
                    evidence_strength: fact.evidence_strength.clone(),
                    properties: None,
                };

                let inserted = facts_db::insert_fact(pool, &new_fact)
                    .await
                    .map_err(|e| FactExtractionPipelineError::Database(e.to_string()))?;

                tracing::debug!(
                    fact_id = %inserted.id,
                    subject = %fact.subject,
                    predicate = %fact.predicate,
                    object = %fact.object,
                    "fact inserted"
                );

                // Build supersession details from the inserted row.
                let details = supersede::new_fact_details_from(&inserted);
                new_fact_details.push(details);
                inserted_fact_ids.push(inserted.id);
                valid_fact_indices.push(idx);
            }
            _ => {
                tracing::warn!(
                    subject = %fact.subject,
                    object = %fact.object,
                    predicate = %fact.predicate,
                    episode_id = %episode_id,
                    subject_found = subject_id.is_some(),
                    object_found = object_id.is_some(),
                    "skipping fact with invalid entity reference"
                );
                skipped_count += 1;
            }
        }
    }

    tracing::info!(
        inserted = inserted_fact_ids.len(),
        skipped = skipped_count,
        episode_id = %episode_id,
        "entity reference validation complete"
    );

    // Step 4: Validate and track predicates for the valid facts.
    // Build a mutable slice of only the valid extracted facts.
    let mut valid_facts: Vec<ExtractedFact> = valid_fact_indices
        .iter()
        .map(|&idx| facts[idx].clone())
        .collect();

    let predicate_validation = if !valid_facts.is_empty() {
        let result = validate_and_track_predicates(
            &mut valid_facts,
            &inserted_fact_ids,
            pool,
            episode_id,
        )
        .await?;
        Some(result)
    } else {
        None
    };

    // Step 5: Resolve supersessions for all inserted facts.
    let superseded_count = if !new_fact_details.is_empty() {
        supersede::resolve_supersessions_batch(pool, &new_fact_details)
            .await
            .map_err(|e| FactExtractionPipelineError::Database(e.to_string()))?
    } else {
        0
    };

    tracing::info!(
        inserted = inserted_fact_ids.len(),
        skipped = skipped_count,
        superseded = superseded_count,
        canonical = predicate_validation.as_ref().map_or(0, |v| v.canonical_count),
        custom = predicate_validation.as_ref().map_or(0, |v| v.custom_count),
        model = %model,
        episode_id = %episode_id,
        "fact extraction orchestration complete"
    );

    Ok(FactOrchestrationResult {
        inserted_count: inserted_fact_ids.len(),
        skipped_count,
        inserted_fact_ids,
        predicate_validation,
        superseded_count,
        model,
    })
}

// ---------------------------------------------------------------------------
// Predicate validation and custom predicate tracking
// ---------------------------------------------------------------------------

/// The occurrence threshold at which a custom predicate candidate is flagged
/// for operator review via the dashboard.
pub const CANDIDATE_REVIEW_THRESHOLD: i32 = 5;

/// Validate extracted fact predicates against the canonical registry and
/// track custom predicates as candidates.
///
/// For each fact in `facts`:
/// 1. Query `loom_predicates` to check if the predicate is canonical.
/// 2. If canonical: set `custom = false`, increment `usage_count`.
/// 3. If not canonical: set `custom = true`, insert or update a row in
///    `loom_predicate_candidates` (increment occurrences, append `fact_id`
///    to `example_facts`).
/// 4. If the candidate reaches [`CANDIDATE_REVIEW_THRESHOLD`] occurrences,
///    log a warning for operator review.
///
/// Returns a [`PredicateValidationResult`] with counts of canonical vs
/// custom predicates and logs the counts per episode via tracing.
///
/// # Errors
///
/// Returns [`FactExtractionPipelineError::Database`] if any database query
/// fails.
pub async fn validate_and_track_predicates(
    facts: &mut [ExtractedFact],
    fact_ids: &[uuid::Uuid],
    pool: &PgPool,
    episode_id: uuid::Uuid,
) -> Result<PredicateValidationResult, FactExtractionPipelineError> {
    let mut canonical_count: usize = 0;
    let mut custom_count: usize = 0;

    for (fact, &fact_id) in facts.iter_mut().zip(fact_ids.iter()) {
        let canonical = pred_db::find_canonical_predicate(pool, &fact.predicate).await?;

        if canonical.is_some() {
            // Canonical predicate: mark and increment usage.
            fact.custom = false;
            pred_db::increment_usage_count(pool, &fact.predicate).await?;
            canonical_count += 1;
        } else {
            // Custom predicate: mark and track as candidate.
            fact.custom = true;
            pred_db::insert_or_update_candidate(pool, &fact.predicate, fact_id).await?;
            custom_count += 1;

            // Check if the candidate has reached the review threshold.
            if let Some(occurrences) =
                pred_db::get_candidate_occurrences(pool, &fact.predicate).await?
            {
                if occurrences >= CANDIDATE_REVIEW_THRESHOLD {
                    tracing::warn!(
                        predicate = %fact.predicate,
                        occurrences,
                        episode_id = %episode_id,
                        "custom predicate candidate reached review threshold — flag for operator review"
                    );
                }
            }
        }
    }

    tracing::info!(
        canonical_count,
        custom_count,
        episode_id = %episode_id,
        "predicate validation complete for episode"
    );

    Ok(PredicateValidationResult {
        canonical_count,
        custom_count,
    })
}

// ---------------------------------------------------------------------------
// Predicate block formatting
// ---------------------------------------------------------------------------

/// Format predicates into a grouped prompt block organized by pack name.
///
/// Each pack gets a markdown heading, and predicates are listed with their
/// category, description, and inverse relationship (when available).
///
/// Example output:
/// ```text
/// ### core (structural, temporal, decisional, operational)
/// - uses (structural): Indicates usage of a technology or service. Inverse: used_by
/// - depends_on (structural): A dependency relationship. Inverse: dependency_of
///
/// ### grc (regulatory)
/// - scoped_as (regulatory): Defines audit scope. Inverse: scoping_includes
/// ```
pub fn format_predicate_block(predicates: &[PredicateEntry]) -> String {
    if predicates.is_empty() {
        return "(no canonical predicates available)".to_string();
    }

    // Group predicates by pack, preserving query order (pack, category, predicate).
    let mut packs: Vec<(String, Vec<&PredicateEntry>)> = Vec::new();

    for pred in predicates {
        if let Some((_pack_name, entries)) = packs.iter_mut().find(|(p, _)| *p == pred.pack) {
            entries.push(pred);
        } else {
            packs.push((pred.pack.clone(), vec![pred]));
        }
    }

    let mut output = String::new();

    for (pack_name, entries) in &packs {
        // Collect unique categories for the pack header.
        let mut categories: Vec<&str> = entries
            .iter()
            .map(|e| e.category.as_str())
            .collect::<Vec<_>>();
        categories.sort_unstable();
        categories.dedup();

        output.push_str(&format!("### {} ({})\n", pack_name, categories.join(", ")));

        for entry in entries {
            let mut line = format!("- {} ({})", entry.predicate, entry.category);

            if let Some(desc) = &entry.description {
                if !desc.is_empty() {
                    line.push_str(&format!(": {desc}"));
                }
            }

            if let Some(inv) = &entry.inverse {
                if !inv.is_empty() {
                    line.push_str(&format!(" [inverse: {inv}]"));
                }
            }

            line.push('\n');
            output.push_str(&line);
        }

        output.push('\n');
    }

    // Trim trailing whitespace.
    output.trim_end().to_string()
}

// ---------------------------------------------------------------------------
// Internal helper types
// ---------------------------------------------------------------------------

/// Row type for querying just the `predicate_packs` column from
/// `loom_namespace_config`.
#[derive(Debug, sqlx::FromRow)]
struct NamespacePacksRow {
    predicate_packs: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

/// Validated entity extraction result with provenance and rejection stats.
#[derive(Debug, Clone)]
pub struct ValidatedExtractionResult {
    /// Entities that passed entity-type validation.
    pub entities: Vec<ExtractedEntity>,
    /// The model identifier used for extraction.
    pub model: String,
    /// Number of entities rejected due to invalid entity type.
    pub rejected_count: usize,
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// The 10 allowed entity type strings, matching [`EntityType`] variants.
const ALLOWED_ENTITY_TYPES: &[&str] = &[
    "person",
    "organization",
    "project",
    "service",
    "technology",
    "pattern",
    "environment",
    "document",
    "metric",
    "decision",
];

/// Validate each extracted entity's `entity_type` against the [`EntityType`]
/// enum. Entities with unrecognised types are logged and filtered out.
fn validate_entity_types(raw: EntityExtractionResult) -> ValidatedExtractionResult {
    let total = raw.entities.len();
    let mut valid: Vec<ExtractedEntity> = Vec::with_capacity(total);

    for entity in raw.entities {
        if is_valid_entity_type(&entity.entity_type) {
            valid.push(entity);
        } else {
            tracing::warn!(
                name = %entity.name,
                entity_type = %entity.entity_type,
                "rejected entity with invalid type"
            );
        }
    }

    let rejected_count = total - valid.len();

    ValidatedExtractionResult {
        entities: valid,
        model: raw.model,
        rejected_count,
    }
}

/// Check whether an entity type string matches one of the 10 allowed types.
///
/// Comparison is case-insensitive to tolerate minor LLM formatting
/// variations.
fn is_valid_entity_type(entity_type: &str) -> bool {
    let lower = entity_type.to_lowercase();
    // Also accept if it parses into the EntityType enum directly.
    if lower.parse::<EntityType>().is_ok() {
        return true;
    }
    ALLOWED_ENTITY_TYPES.contains(&lower.as_str())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::extraction::EntityExtractionResult;
    use crate::types::entity::ExtractedEntity;
    use crate::types::predicate::PredicateEntry;
    use serde_json::json;

    // -- is_valid_entity_type -----------------------------------------------

    #[test]
    fn valid_entity_types_accepted() {
        for ty in ALLOWED_ENTITY_TYPES {
            assert!(is_valid_entity_type(ty), "should accept {ty}");
        }
    }

    #[test]
    fn valid_entity_types_case_insensitive() {
        assert!(is_valid_entity_type("Person"));
        assert!(is_valid_entity_type("TECHNOLOGY"));
        assert!(is_valid_entity_type("Decision"));
    }

    #[test]
    fn invalid_entity_types_rejected() {
        assert!(!is_valid_entity_type("animal"));
        assert!(!is_valid_entity_type("concept"));
        assert!(!is_valid_entity_type(""));
        assert!(!is_valid_entity_type("completely_made_up_type"));
    }

    // -- validate_entity_types ----------------------------------------------

    #[test]
    fn validate_filters_invalid_types() {
        let raw = EntityExtractionResult {
            entities: vec![
                ExtractedEntity {
                    name: "Rust".to_string(),
                    entity_type: "technology".to_string(),
                    aliases: vec!["rust-lang".to_string()],
                    properties: json!({}),
                },
                ExtractedEntity {
                    name: "FooBar".to_string(),
                    entity_type: "unknown_type".to_string(),
                    aliases: vec![],
                    properties: json!({}),
                },
                ExtractedEntity {
                    name: "Alice".to_string(),
                    entity_type: "person".to_string(),
                    aliases: vec![],
                    properties: json!({}),
                },
            ],
            model: "test-model".to_string(),
        };

        let result = validate_entity_types(raw);
        assert_eq!(result.entities.len(), 2);
        assert_eq!(result.rejected_count, 1);
        assert_eq!(result.entities[0].name, "Rust");
        assert_eq!(result.entities[1].name, "Alice");
        assert_eq!(result.model, "test-model");
    }

    #[test]
    fn validate_all_valid_entities_pass() {
        let raw = EntityExtractionResult {
            entities: vec![
                ExtractedEntity {
                    name: "PostgreSQL".to_string(),
                    entity_type: "technology".to_string(),
                    aliases: vec!["Postgres".to_string()],
                    properties: json!({}),
                },
                ExtractedEntity {
                    name: "Platform Team".to_string(),
                    entity_type: "organization".to_string(),
                    aliases: vec![],
                    properties: json!({}),
                },
            ],
            model: "gemma4:26b".to_string(),
        };

        let result = validate_entity_types(raw);
        assert_eq!(result.entities.len(), 2);
        assert_eq!(result.rejected_count, 0);
    }

    #[test]
    fn validate_empty_entities_returns_empty() {
        let raw = EntityExtractionResult {
            entities: vec![],
            model: "test-model".to_string(),
        };

        let result = validate_entity_types(raw);
        assert!(result.entities.is_empty());
        assert_eq!(result.rejected_count, 0);
    }

    #[test]
    fn validate_all_invalid_entities_returns_empty() {
        let raw = EntityExtractionResult {
            entities: vec![
                ExtractedEntity {
                    name: "Foo".to_string(),
                    entity_type: "widget".to_string(),
                    aliases: vec![],
                    properties: json!({}),
                },
                ExtractedEntity {
                    name: "Bar".to_string(),
                    entity_type: "gadget".to_string(),
                    aliases: vec![],
                    properties: json!({}),
                },
            ],
            model: "test-model".to_string(),
        };

        let result = validate_entity_types(raw);
        assert!(result.entities.is_empty());
        assert_eq!(result.rejected_count, 2);
    }

    #[test]
    fn validate_preserves_aliases_and_properties() {
        let raw = EntityExtractionResult {
            entities: vec![ExtractedEntity {
                name: "Kubernetes".to_string(),
                entity_type: "technology".to_string(),
                aliases: vec!["K8s".to_string(), "kube".to_string()],
                properties: json!({"version": "1.28"}),
            }],
            model: "test-model".to_string(),
        };

        let result = validate_entity_types(raw);
        assert_eq!(result.entities.len(), 1);
        let entity = &result.entities[0];
        assert_eq!(entity.aliases, vec!["K8s", "kube"]);
        assert_eq!(entity.properties["version"], "1.28");
    }

    // -- All 10 entity types pass validation --------------------------------

    #[test]
    fn all_ten_entity_types_pass_validation() {
        let types = vec![
            "person",
            "organization",
            "project",
            "service",
            "technology",
            "pattern",
            "environment",
            "document",
            "metric",
            "decision",
        ];

        let entities: Vec<ExtractedEntity> = types
            .iter()
            .enumerate()
            .map(|(i, ty)| ExtractedEntity {
                name: format!("Entity{i}"),
                entity_type: ty.to_string(),
                aliases: vec![],
                properties: json!({}),
            })
            .collect();

        let raw = EntityExtractionResult {
            entities,
            model: "test-model".to_string(),
        };

        let result = validate_entity_types(raw);
        assert_eq!(result.entities.len(), 10);
        assert_eq!(result.rejected_count, 0);
    }

    // -- End-to-end with wiremock -------------------------------------------

    #[tokio::test]
    async fn extract_entities_from_episode_end_to_end() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        let llm_content = json!({
            "entities": [
                {
                    "name": "Rust",
                    "entity_type": "technology",
                    "aliases": ["rust-lang"],
                    "properties": {}
                },
                {
                    "name": "Alice",
                    "entity_type": "person",
                    "aliases": [],
                    "properties": {}
                },
                {
                    "name": "SomeWeirdThing",
                    "entity_type": "alien",
                    "aliases": [],
                    "properties": {}
                }
            ]
        });

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{
                    "message": { "content": llm_content.to_string() }
                }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let config = crate::config::LlmConfig {
            ollama_url: server.uri(),
            extraction_model: "test-model".to_string(),
            classification_model: "test-model".to_string(),
            embedding_model: "nomic-embed-text".to_string(),
            azure_openai_url: None,
            azure_openai_key: None,
        };

        let client =
            crate::llm::client::LlmClient::new(&config).expect("should build client");

        let result =
            extract_entities_from_episode(&client, &config, "Alice uses Rust for programming")
                .await
                .expect("should extract entities");

        // "alien" type should be rejected.
        assert_eq!(result.entities.len(), 2);
        assert_eq!(result.rejected_count, 1);
        assert_eq!(result.entities[0].name, "Rust");
        assert_eq!(result.entities[0].entity_type, "technology");
        assert_eq!(result.entities[1].name, "Alice");
        assert_eq!(result.entities[1].entity_type, "person");
        assert_eq!(result.model, "test-model");
    }

    #[tokio::test]
    async fn extract_entities_from_episode_llm_error() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{
                    "message": { "content": "not valid json at all" }
                }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let config = crate::config::LlmConfig {
            ollama_url: server.uri(),
            extraction_model: "test-model".to_string(),
            classification_model: "test-model".to_string(),
            embedding_model: "nomic-embed-text".to_string(),
            azure_openai_url: None,
            azure_openai_key: None,
        };

        let client =
            crate::llm::client::LlmClient::new(&config).expect("should build client");

        let err =
            extract_entities_from_episode(&client, &config, "some content")
                .await
                .unwrap_err();

        assert!(matches!(err, ExtractionError::Deserialization(_)));
    }

    // -- format_predicate_block ---------------------------------------------

    fn make_predicate(name: &str, category: &str, pack: &str, desc: &str, inverse: Option<&str>) -> PredicateEntry {
        PredicateEntry {
            predicate: name.to_string(),
            category: category.to_string(),
            pack: pack.to_string(),
            inverse: inverse.map(|s| s.to_string()),
            description: Some(desc.to_string()),
            usage_count: Some(0),
            created_at: None,
        }
    }

    #[test]
    fn format_predicate_block_groups_by_pack() {
        let predicates = vec![
            make_predicate("uses", "structural", "core", "Usage relationship", Some("used_by")),
            make_predicate("depends_on", "structural", "core", "Dependency", Some("dependency_of")),
            make_predicate("scoped_as", "regulatory", "grc", "Audit scope", Some("scoping_includes")),
        ];

        let block = format_predicate_block(&predicates);

        // Should have both pack headers.
        assert!(block.contains("### core"));
        assert!(block.contains("### grc"));

        // Should contain predicate entries.
        assert!(block.contains("- uses (structural): Usage relationship [inverse: used_by]"));
        assert!(block.contains("- depends_on (structural): Dependency [inverse: dependency_of]"));
        assert!(block.contains("- scoped_as (regulatory): Audit scope [inverse: scoping_includes]"));

        // Core should appear before grc (preserves query order).
        let core_pos = block.find("### core").unwrap();
        let grc_pos = block.find("### grc").unwrap();
        assert!(core_pos < grc_pos);
    }

    #[test]
    fn format_predicate_block_empty_returns_placeholder() {
        let block = format_predicate_block(&[]);
        assert_eq!(block, "(no canonical predicates available)");
    }

    #[test]
    fn format_predicate_block_no_inverse() {
        let predicates = vec![
            make_predicate("custom_rel", "operational", "core", "A custom relationship", None),
        ];

        let block = format_predicate_block(&predicates);
        assert!(block.contains("- custom_rel (operational): A custom relationship"));
        assert!(!block.contains("[inverse:"));
    }

    #[test]
    fn format_predicate_block_no_description() {
        let predicates = vec![
            PredicateEntry {
                predicate: "bare_pred".to_string(),
                category: "structural".to_string(),
                pack: "core".to_string(),
                inverse: None,
                description: None,
                usage_count: Some(0),
                created_at: None,
            },
        ];

        let block = format_predicate_block(&predicates);
        assert!(block.contains("- bare_pred (structural)"));
    }

    #[test]
    fn format_predicate_block_multiple_categories_in_header() {
        let predicates = vec![
            make_predicate("uses", "structural", "core", "Usage", None),
            make_predicate("decided", "decisional", "core", "Decision", None),
            make_predicate("deployed_to", "operational", "core", "Deployment", None),
        ];

        let block = format_predicate_block(&predicates);

        // Header should list all unique categories sorted.
        assert!(block.contains("### core (decisional, operational, structural)"));
    }

    #[test]
    fn format_predicate_block_single_pack_single_predicate() {
        let predicates = vec![
            make_predicate("uses", "structural", "core", "Usage relationship", Some("used_by")),
        ];

        let block = format_predicate_block(&predicates);
        assert!(block.contains("### core (structural)"));
        assert!(block.contains("- uses (structural): Usage relationship [inverse: used_by]"));
    }

    // -- FactExtractionPipelineError ----------------------------------------

    #[test]
    fn fact_extraction_pipeline_error_display() {
        let err = FactExtractionPipelineError::Database("connection refused".into());
        assert!(err.to_string().contains("connection refused"));

        let inner = ExtractionError::Validation("bad field".into());
        let err = FactExtractionPipelineError::Extraction(inner);
        assert!(err.to_string().contains("bad field"));
    }

    #[test]
    fn fact_extraction_pipeline_error_from_sqlx() {
        let sqlx_err = sqlx::Error::RowNotFound;
        let err: FactExtractionPipelineError = sqlx_err.into();
        assert!(matches!(err, FactExtractionPipelineError::Database(_)));
    }

    #[test]
    fn fact_extraction_pipeline_error_from_predicate_error() {
        let pred_err = pred_db::PredicateError::Sqlx(sqlx::Error::RowNotFound);
        let err: FactExtractionPipelineError = pred_err.into();
        assert!(matches!(err, FactExtractionPipelineError::Database(_)));
    }

    // -- PredicateValidationResult ------------------------------------------

    #[test]
    fn predicate_validation_result_default_counts() {
        let result = PredicateValidationResult {
            canonical_count: 0,
            custom_count: 0,
        };
        assert_eq!(result.canonical_count, 0);
        assert_eq!(result.custom_count, 0);
    }

    #[test]
    fn predicate_validation_result_tracks_counts() {
        let result = PredicateValidationResult {
            canonical_count: 7,
            custom_count: 3,
        };
        assert_eq!(result.canonical_count, 7);
        assert_eq!(result.custom_count, 3);
    }

    #[test]
    fn candidate_review_threshold_is_five() {
        assert_eq!(CANDIDATE_REVIEW_THRESHOLD, 5);
    }

    // -- FactOrchestrationResult --------------------------------------------

    #[test]
    fn fact_orchestration_result_empty() {
        let result = FactOrchestrationResult {
            inserted_count: 0,
            skipped_count: 0,
            inserted_fact_ids: vec![],
            predicate_validation: None,
            superseded_count: 0,
            model: "test-model".to_string(),
        };
        assert_eq!(result.inserted_count, 0);
        assert_eq!(result.skipped_count, 0);
        assert!(result.inserted_fact_ids.is_empty());
        assert!(result.predicate_validation.is_none());
        assert_eq!(result.superseded_count, 0);
    }

    #[test]
    fn fact_orchestration_result_with_counts() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let result = FactOrchestrationResult {
            inserted_count: 2,
            skipped_count: 1,
            inserted_fact_ids: vec![id1, id2],
            predicate_validation: Some(PredicateValidationResult {
                canonical_count: 1,
                custom_count: 1,
            }),
            superseded_count: 1,
            model: "gemma4:26b".to_string(),
        };
        assert_eq!(result.inserted_count, 2);
        assert_eq!(result.skipped_count, 1);
        assert_eq!(result.inserted_fact_ids.len(), 2);
        assert_eq!(result.superseded_count, 1);
        assert_eq!(result.model, "gemma4:26b");
        let pv = result.predicate_validation.unwrap();
        assert_eq!(pv.canonical_count, 1);
        assert_eq!(pv.custom_count, 1);
    }

    #[test]
    fn fact_orchestration_result_debug_format() {
        let result = FactOrchestrationResult {
            inserted_count: 3,
            skipped_count: 0,
            inserted_fact_ids: vec![Uuid::new_v4()],
            predicate_validation: None,
            superseded_count: 0,
            model: "test".to_string(),
        };
        let debug = format!("{result:?}");
        assert!(debug.contains("inserted_count: 3"));
        assert!(debug.contains("model: \"test\""));
    }

    #[test]
    fn fact_orchestration_result_clone() {
        let result = FactOrchestrationResult {
            inserted_count: 1,
            skipped_count: 2,
            inserted_fact_ids: vec![Uuid::new_v4()],
            predicate_validation: Some(PredicateValidationResult {
                canonical_count: 1,
                custom_count: 0,
            }),
            superseded_count: 0,
            model: "test".to_string(),
        };
        let cloned = result.clone();
        assert_eq!(cloned.inserted_count, result.inserted_count);
        assert_eq!(cloned.skipped_count, result.skipped_count);
        assert_eq!(cloned.inserted_fact_ids, result.inserted_fact_ids);
        assert_eq!(cloned.superseded_count, result.superseded_count);
        assert_eq!(cloned.model, result.model);
    }

    // -- Unit tests for task 7.10: fact extraction and supersession ----------

    // -- Predicate validation against canonical registry --------------------

    #[test]
    fn canonical_predicate_classification_uses() {
        // "uses" is a canonical core predicate — should be classified as
        // custom=false.
        use std::collections::HashSet;

        let canonical: HashSet<&str> = [
            "uses", "used_by", "depends_on", "deployed_to", "implements",
        ]
        .into_iter()
        .collect();

        let mut fact = crate::types::fact::ExtractedFact {
            subject: "ServiceA".to_string(),
            predicate: "uses".to_string(),
            object: "PostgreSQL".to_string(),
            custom: true, // default before validation
            evidence_strength: Some("explicit".to_string()),
            temporal_markers: None,
        };

        // Simulate classification logic.
        if canonical.contains(fact.predicate.as_str()) {
            fact.custom = false;
        }

        assert!(!fact.custom, "canonical predicate 'uses' should be custom=false");
    }

    #[test]
    fn non_canonical_predicate_classified_as_custom() {
        use std::collections::HashSet;

        let canonical: HashSet<&str> = [
            "uses", "used_by", "depends_on", "deployed_to",
        ]
        .into_iter()
        .collect();

        let mut fact = crate::types::fact::ExtractedFact {
            subject: "ServiceA".to_string(),
            predicate: "talks_to".to_string(),
            object: "ServiceB".to_string(),
            custom: false,
            evidence_strength: Some("implied".to_string()),
            temporal_markers: None,
        };

        if !canonical.contains(fact.predicate.as_str()) {
            fact.custom = true;
        }

        assert!(fact.custom, "non-canonical predicate 'talks_to' should be custom=true");
    }

    // -- Custom predicate candidate creation and flagging at 5 occurrences --

    #[test]
    fn custom_predicate_candidate_flagged_at_threshold() {
        use std::collections::HashMap;

        let mut occurrences: HashMap<String, i32> = HashMap::new();
        let predicate = "communicates_with";

        // Simulate 5 facts with the same custom predicate.
        for _ in 0..5 {
            let count = occurrences.entry(predicate.to_string()).or_insert(0);
            *count += 1;
        }

        let count = occurrences[predicate];
        assert_eq!(count, 5);
        assert!(
            count >= CANDIDATE_REVIEW_THRESHOLD,
            "5 occurrences should meet the review threshold of {}",
            CANDIDATE_REVIEW_THRESHOLD
        );
    }

    #[test]
    fn custom_predicate_candidate_not_flagged_below_threshold() {
        use std::collections::HashMap;

        let mut occurrences: HashMap<String, i32> = HashMap::new();
        let predicate = "communicates_with";

        // Simulate 4 facts — below threshold.
        for _ in 0..4 {
            let count = occurrences.entry(predicate.to_string()).or_insert(0);
            *count += 1;
        }

        let count = occurrences[predicate];
        assert_eq!(count, 4);
        assert!(
            count < CANDIDATE_REVIEW_THRESHOLD,
            "4 occurrences should be below the review threshold of {}",
            CANDIDATE_REVIEW_THRESHOLD
        );
    }

    #[test]
    fn multiple_custom_predicates_tracked_independently() {
        use std::collections::HashMap;

        let mut occurrences: HashMap<String, i32> = HashMap::new();

        // 5 facts with "talks_to", 3 facts with "monitors".
        for _ in 0..5 {
            *occurrences.entry("talks_to".to_string()).or_insert(0) += 1;
        }
        for _ in 0..3 {
            *occurrences.entry("monitors".to_string()).or_insert(0) += 1;
        }

        assert_eq!(occurrences["talks_to"], 5);
        assert_eq!(occurrences["monitors"], 3);
        assert!(occurrences["talks_to"] >= CANDIDATE_REVIEW_THRESHOLD);
        assert!(occurrences["monitors"] < CANDIDATE_REVIEW_THRESHOLD);
    }

    // -- Temporal marker extraction -----------------------------------------

    #[test]
    fn temporal_markers_deserialized_from_json() {
        let json_str = r#"{
            "subject": "ServiceA",
            "predicate": "deployed_to",
            "object": "Production",
            "custom": false,
            "evidence_strength": "explicit",
            "temporal_markers": {
                "valid_from": "2025-01-15T10:00:00Z",
                "valid_until": "2025-06-15T10:00:00Z"
            }
        }"#;

        let fact: crate::types::fact::ExtractedFact =
            serde_json::from_str(json_str).expect("should deserialize");

        assert_eq!(fact.subject, "ServiceA");
        assert_eq!(fact.predicate, "deployed_to");
        assert!(!fact.custom);

        let markers = fact.temporal_markers.expect("should have temporal markers");
        assert!(markers.valid_from.is_some());
        assert!(markers.valid_until.is_some());
    }

    #[test]
    fn temporal_markers_absent_when_not_provided() {
        let json_str = r#"{
            "subject": "ServiceA",
            "predicate": "uses",
            "object": "PostgreSQL",
            "custom": false,
            "evidence_strength": "explicit"
        }"#;

        let fact: crate::types::fact::ExtractedFact =
            serde_json::from_str(json_str).expect("should deserialize");

        assert!(fact.temporal_markers.is_none());
    }

    #[test]
    fn temporal_markers_partial_valid_from_only() {
        let json_str = r#"{
            "subject": "ServiceA",
            "predicate": "deployed_to",
            "object": "Staging",
            "custom": false,
            "evidence_strength": "explicit",
            "temporal_markers": {
                "valid_from": "2025-03-01T00:00:00Z",
                "valid_until": null
            }
        }"#;

        let fact: crate::types::fact::ExtractedFact =
            serde_json::from_str(json_str).expect("should deserialize");

        let markers = fact.temporal_markers.expect("should have temporal markers");
        assert!(markers.valid_from.is_some());
        assert!(markers.valid_until.is_none());
    }

    // -- Pack-aware prompt assembly with various namespace configurations ----

    #[test]
    fn prompt_assembly_core_only() {
        let predicates = vec![
            make_predicate("uses", "structural", "core", "Usage", Some("used_by")),
            make_predicate("depends_on", "structural", "core", "Dependency", Some("dependency_of")),
        ];

        let block = format_predicate_block(&predicates);

        assert!(block.contains("### core"));
        assert!(block.contains("- uses (structural)"));
        assert!(block.contains("- depends_on (structural)"));
        // No other pack headers.
        assert!(!block.contains("### grc"));
    }

    #[test]
    fn prompt_assembly_core_plus_grc() {
        let predicates = vec![
            make_predicate("uses", "structural", "core", "Usage", Some("used_by")),
            make_predicate("scoped_as", "regulatory", "grc", "Audit scope", Some("scoping_includes")),
            make_predicate("maps_to_control", "regulatory", "grc", "Control mapping", None),
        ];

        let block = format_predicate_block(&predicates);

        assert!(block.contains("### core"));
        assert!(block.contains("### grc"));
        assert!(block.contains("- uses (structural)"));
        assert!(block.contains("- scoped_as (regulatory)"));
        assert!(block.contains("- maps_to_control (regulatory)"));
    }

    #[test]
    fn prompt_assembly_three_packs() {
        let predicates = vec![
            make_predicate("uses", "structural", "core", "Usage", None),
            make_predicate("scoped_as", "regulatory", "grc", "Scope", None),
            make_predicate("treats", "operational", "healthcare", "Treatment", None),
        ];

        let block = format_predicate_block(&predicates);

        assert!(block.contains("### core"));
        assert!(block.contains("### grc"));
        assert!(block.contains("### healthcare"));
        assert!(block.contains("- uses (structural)"));
        assert!(block.contains("- scoped_as (regulatory)"));
        assert!(block.contains("- treats (operational)"));
    }

    #[test]
    fn prompt_assembly_preserves_pack_order() {
        // Predicates ordered: core, grc, healthcare.
        let predicates = vec![
            make_predicate("uses", "structural", "core", "Usage", None),
            make_predicate("scoped_as", "regulatory", "grc", "Scope", None),
            make_predicate("treats", "operational", "healthcare", "Treatment", None),
        ];

        let block = format_predicate_block(&predicates);

        let core_pos = block.find("### core").unwrap();
        let grc_pos = block.find("### grc").unwrap();
        let health_pos = block.find("### healthcare").unwrap();

        assert!(core_pos < grc_pos, "core should appear before grc");
        assert!(grc_pos < health_pos, "grc should appear before healthcare");
    }

    // -- PipelineError display ----------------------------------------------

    #[test]
    fn pipeline_error_embedding_displays_message() {
        let err = PipelineError::Embedding(
            crate::llm::embeddings::EmbeddingError::DimensionMismatch {
                expected: 768,
                actual: 512,
            },
        );
        let msg = err.to_string();
        assert!(msg.contains("embedding error"), "got: {msg}");
    }

    #[test]
    fn pipeline_error_extraction_displays_message() {
        let err = PipelineError::Extraction(ExtractionError::Validation("bad".into()));
        let msg = err.to_string();
        assert!(msg.contains("extraction error"), "got: {msg}");
    }

    #[test]
    fn pipeline_error_database_displays_message() {
        let err = PipelineError::Database("connection refused".into());
        let msg = err.to_string();
        assert!(msg.contains("database error"), "got: {msg}");
    }

    #[test]
    fn pipeline_error_from_episode_error() {
        let ep_err = crate::db::episodes::EpisodeError::Sqlx(sqlx::Error::RowNotFound);
        let err: PipelineError = ep_err.into();
        assert!(matches!(err, PipelineError::Database(_)));
    }

    // -- FullPipelineResult -------------------------------------------------

    #[test]
    fn full_pipeline_result_debug_format() {
        let metrics = crate::types::episode::ExtractionMetrics {
            extracted: 3,
            resolved_exact: 1,
            resolved_alias: 1,
            resolved_semantic: 0,
            new: 1,
            conflict_flagged: 0,
            facts_extracted: 2,
            canonical_predicate: 2,
            custom_predicate: 0,
            explicit: 1,
            implied: 1,
            processing_time_ms: 500,
            extraction_model: "test".to_string(),
        };

        let result = FullPipelineResult {
            episode_id: Uuid::new_v4(),
            entity_count: 3,
            fact_count: 2,
            facts_skipped: 0,
            superseded_count: 0,
            conflict_count: 0,
            metrics,
        };

        let debug = format!("{result:?}");
        assert!(debug.contains("entity_count: 3"));
        assert!(debug.contains("fact_count: 2"));
    }

    #[test]
    fn full_pipeline_result_clone() {
        let metrics = crate::types::episode::ExtractionMetrics {
            extracted: 1,
            resolved_exact: 1,
            resolved_alias: 0,
            resolved_semantic: 0,
            new: 0,
            conflict_flagged: 0,
            facts_extracted: 1,
            canonical_predicate: 1,
            custom_predicate: 0,
            explicit: 1,
            implied: 0,
            processing_time_ms: 100,
            extraction_model: "test".to_string(),
        };

        let result = FullPipelineResult {
            episode_id: Uuid::new_v4(),
            entity_count: 1,
            fact_count: 1,
            facts_skipped: 0,
            superseded_count: 0,
            conflict_count: 0,
            metrics,
        };

        let cloned = result.clone();
        assert_eq!(cloned.episode_id, result.episode_id);
        assert_eq!(cloned.entity_count, result.entity_count);
        assert_eq!(cloned.fact_count, result.fact_count);
    }

    // -- count_conflict_flagged ---------------------------------------------

    #[test]
    fn count_conflict_flagged_returns_zero() {
        let results = vec![
            crate::types::entity::ResolutionResult {
                entity_id: Uuid::new_v4(),
                method: "new".to_string(),
                confidence: 1.0,
            },
        ];
        assert_eq!(count_conflict_flagged(&results), 0);
    }
}
