//! Candidate procedure flagging for the offline extraction pipeline.
//!
//! Identifies repeated behavioral patterns across episodes by analyzing
//! extracted facts and entities. When a pattern is observed in a new episode,
//! the system either creates a new procedure candidate or increments the
//! observation count on an existing one.
//!
//! Procedures start with `evidence_status = 'extracted'` and `confidence = 0.3`.
//! Confidence increases with each observation. When a procedure reaches
//! `confidence >= 0.8` and `observation_count >= 3`, it is promoted to
//! `evidence_status = 'promoted'` and becomes eligible for the
//! `procedure_assist` retrieval profile.
//!
//! Hot tier eligibility requires additional criteria:
//! - 3+ distinct source episodes
//! - 7+ days since first observation
//! - confidence >= 0.8

use chrono::{DateTime, Utc};
use pgvector::Vector;
use sqlx::PgPool;
use uuid::Uuid;

use crate::config::LlmConfig;
use crate::db::procedures::{self, NewProcedure, Procedure, ProcedureError};
use crate::llm::client::LlmClient;
use crate::llm::embeddings::{self, EmbeddingError};
use crate::types::fact::ExtractedFact;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Initial confidence score for newly created procedures.
pub const INITIAL_CONFIDENCE: f64 = 0.3;

/// Confidence increment per additional observation.
///
/// Each new observation adds this value to the confidence score, capped at 1.0.
pub const CONFIDENCE_INCREMENT: f64 = 0.15;

/// Minimum confidence required for promotion to `promoted` status.
pub const PROMOTION_CONFIDENCE_THRESHOLD: f64 = 0.8;

/// Minimum observation count required for promotion.
pub const PROMOTION_OBSERVATION_THRESHOLD: i32 = 3;

/// Minimum number of distinct source episodes for hot tier eligibility.
pub const HOT_TIER_MIN_EPISODES: i32 = 3;

/// Minimum age in days for hot tier eligibility.
pub const HOT_TIER_MIN_AGE_DAYS: i64 = 7;

/// Similarity threshold for matching an existing procedure pattern.
///
/// If a new pattern's embedding has cosine similarity >= this threshold
/// against an existing procedure in the same namespace, it is considered
/// the same pattern and the observation count is incremented.
pub const PATTERN_SIMILARITY_THRESHOLD: f64 = 0.88;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during procedure extraction and promotion.
#[derive(Debug, thiserror::Error)]
pub enum ProcedureExtractionError {
    /// An underlying database error from the procedures module.
    #[error("procedure database error: {0}")]
    Database(#[from] ProcedureError),

    /// An error generating an embedding for the procedure pattern.
    #[error("embedding error: {0}")]
    Embedding(#[from] EmbeddingError),

    /// An underlying sqlx error.
    #[error("sqlx error: {0}")]
    Sqlx(#[from] sqlx::Error),
}

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

/// Result of procedure flagging for a single episode.
#[derive(Debug, Clone)]
pub struct ProcedureFlaggingResult {
    /// Number of new procedures created.
    pub new_count: usize,
    /// Number of existing procedures whose observation was incremented.
    pub updated_count: usize,
    /// Number of procedures promoted to `promoted` status this cycle.
    pub promoted_count: usize,
    /// IDs of all procedures affected (new or updated).
    pub procedure_ids: Vec<Uuid>,
}

// ---------------------------------------------------------------------------
// Pattern detection
// ---------------------------------------------------------------------------

/// A candidate behavioral pattern detected from extracted facts.
///
/// Patterns are identified by looking for repeated predicate usage across
/// entities that suggests a workflow or practice.
#[derive(Debug, Clone)]
pub struct DetectedPattern {
    /// Human-readable description of the pattern.
    pub pattern: String,
    /// Optional category for the pattern.
    pub category: Option<String>,
}

/// Detect candidate behavioral patterns from extracted facts.
///
/// Looks for repeated predicate sequences and entity interaction patterns
/// that suggest recurring workflows. This is a heuristic approach — not
/// every episode will produce patterns.
///
/// Current heuristics:
/// - Facts with the same predicate appearing 2+ times suggest a pattern.
/// - Facts forming chains (A→B→C) suggest workflow patterns.
pub fn detect_patterns(facts: &[ExtractedFact]) -> Vec<DetectedPattern> {
    let mut patterns: Vec<DetectedPattern> = Vec::new();

    if facts.len() < 2 {
        return patterns;
    }

    // Heuristic 1: Repeated predicates suggest a behavioral pattern.
    let mut predicate_counts: std::collections::HashMap<&str, Vec<&ExtractedFact>> =
        std::collections::HashMap::new();

    for fact in facts {
        predicate_counts
            .entry(&fact.predicate)
            .or_default()
            .push(fact);
    }

    for (predicate, related_facts) in &predicate_counts {
        if related_facts.len() >= 2 {
            // Build a pattern description from the repeated predicate usage.
            let subjects: Vec<&str> = related_facts
                .iter()
                .map(|f| f.subject.as_str())
                .collect();
            let objects: Vec<&str> = related_facts
                .iter()
                .map(|f| f.object.as_str())
                .collect();

            let pattern_desc = format!(
                "When working with {}, {} is applied to {}",
                subjects.join(", "),
                predicate,
                objects.join(", ")
            );

            patterns.push(DetectedPattern {
                pattern: pattern_desc,
                category: Some(categorize_predicate(predicate)),
            });
        }
    }

    // Heuristic 2: Chains (A uses B, B uses C) suggest workflow patterns.
    for fact_a in facts {
        for fact_b in facts {
            if fact_a.object == fact_b.subject && fact_a.predicate == fact_b.predicate {
                let pattern_desc = format!(
                    "{} → {} → {} (via {})",
                    fact_a.subject, fact_a.object, fact_b.object, fact_a.predicate
                );
                patterns.push(DetectedPattern {
                    pattern: pattern_desc,
                    category: Some("workflow".to_string()),
                });
            }
        }
    }

    patterns
}

/// Categorize a predicate into a procedure category.
fn categorize_predicate(predicate: &str) -> String {
    match predicate {
        "uses" | "depends_on" | "integrates_with" => "dependency".to_string(),
        "deployed_to" | "hosts" | "configured_with" => "operational".to_string(),
        "decided" | "decided_by" => "decisional".to_string(),
        "implements" | "implemented_by" => "implementation".to_string(),
        _ => "general".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Core procedure flagging
// ---------------------------------------------------------------------------

/// Flag candidate procedures from extracted facts for a single episode.
///
/// Orchestration steps:
/// 1. Detect candidate patterns from the extracted facts.
/// 2. For each detected pattern, check if a similar procedure already exists
///    in the namespace (by embedding similarity).
/// 3. If a match is found: increment observation count and update confidence.
/// 4. If no match: create a new procedure with initial confidence 0.3.
/// 5. Check promotion criteria and promote eligible procedures.
///
/// # Errors
///
/// Returns [`ProcedureExtractionError`] if database or embedding operations fail.
#[tracing::instrument(
    skip(pool, client, config, facts),
    fields(
        namespace,
        episode_id = %episode_id,
        fact_count = facts.len(),
    )
)]
pub async fn flag_candidate_procedures(
    pool: &PgPool,
    client: &LlmClient,
    config: &LlmConfig,
    facts: &[ExtractedFact],
    namespace: &str,
    episode_id: Uuid,
) -> Result<ProcedureFlaggingResult, ProcedureExtractionError> {
    let detected = detect_patterns(facts);

    if detected.is_empty() {
        tracing::debug!(
            episode_id = %episode_id,
            namespace,
            "no candidate patterns detected"
        );
        return Ok(ProcedureFlaggingResult {
            new_count: 0,
            updated_count: 0,
            promoted_count: 0,
            procedure_ids: Vec::new(),
        });
    }

    tracing::info!(
        episode_id = %episode_id,
        namespace,
        detected_count = detected.len(),
        "candidate patterns detected, checking for existing matches"
    );

    let mut new_count: usize = 0;
    let mut updated_count: usize = 0;
    let mut promoted_count: usize = 0;
    let mut procedure_ids: Vec<Uuid> = Vec::new();

    for pattern in &detected {
        // Generate embedding for the pattern text.
        let embedding_vec = embeddings::generate_embedding(
            client, config, &pattern.pattern,
        )
        .await?;
        let embedding = Vector::from(embedding_vec.clone());

        // Check for existing similar procedure in the namespace.
        let existing = find_similar_procedure(pool, namespace, &embedding).await?;

        match existing {
            Some(existing_proc) => {
                // Increment observation on existing procedure.
                procedures::update_observation(pool, existing_proc.id, episode_id).await?;

                // Update confidence.
                let new_confidence = compute_new_confidence(
                    existing_proc.confidence.unwrap_or(INITIAL_CONFIDENCE),
                    existing_proc.observation_count.unwrap_or(1),
                );
                update_procedure_confidence(pool, existing_proc.id, new_confidence).await?;

                // Check promotion criteria.
                let obs_count = existing_proc.observation_count.unwrap_or(1) + 1;
                if should_promote(new_confidence, obs_count) {
                    promote_procedure(pool, existing_proc.id).await?;
                    promoted_count += 1;
                    tracing::info!(
                        procedure_id = %existing_proc.id,
                        confidence = new_confidence,
                        observations = obs_count,
                        "procedure promoted to 'promoted' status"
                    );
                }

                procedure_ids.push(existing_proc.id);
                updated_count += 1;

                tracing::debug!(
                    procedure_id = %existing_proc.id,
                    new_confidence,
                    observations = obs_count,
                    "existing procedure observation incremented"
                );
            }
            None => {
                // Create a new procedure candidate.
                let new_proc = NewProcedure {
                    pattern: pattern.pattern.clone(),
                    category: pattern.category.clone(),
                    namespace: namespace.to_string(),
                    source_episodes: vec![episode_id],
                    evidence_status: "extracted".to_string(),
                    confidence: INITIAL_CONFIDENCE,
                };

                let inserted = procedures::insert_procedure(pool, &new_proc).await?;

                // Store the embedding on the procedure.
                store_procedure_embedding(pool, inserted.id, &embedding).await?;

                procedure_ids.push(inserted.id);
                new_count += 1;

                tracing::debug!(
                    procedure_id = %inserted.id,
                    pattern = %pattern.pattern,
                    "new procedure candidate created"
                );
            }
        }
    }

    tracing::info!(
        episode_id = %episode_id,
        namespace,
        new_count,
        updated_count,
        promoted_count,
        "procedure flagging complete"
    );

    Ok(ProcedureFlaggingResult {
        new_count,
        updated_count,
        promoted_count,
        procedure_ids,
    })
}

// ---------------------------------------------------------------------------
// Confidence computation
// ---------------------------------------------------------------------------

/// Compute the new confidence score after an additional observation.
///
/// Confidence grows with each observation but is capped at 1.0. The formula
/// applies diminishing returns as confidence approaches 1.0.
pub fn compute_new_confidence(current: f64, _observation_count: i32) -> f64 {
    let new = current + CONFIDENCE_INCREMENT * (1.0 - current);
    new.min(1.0)
}

/// Determine whether a procedure should be promoted to `promoted` status.
///
/// Promotion requires:
/// - confidence >= 0.8
/// - observation_count >= 3
pub fn should_promote(confidence: f64, observation_count: i32) -> bool {
    confidence >= PROMOTION_CONFIDENCE_THRESHOLD
        && observation_count >= PROMOTION_OBSERVATION_THRESHOLD
}

/// Check whether a procedure is eligible for hot tier placement.
///
/// Hot tier requires:
/// - 3+ distinct source episodes
/// - 7+ days since first observation
/// - confidence >= 0.8
pub fn eligible_for_hot_tier(
    source_episode_count: i32,
    first_observed: Option<DateTime<Utc>>,
    confidence: f64,
) -> bool {
    if source_episode_count < HOT_TIER_MIN_EPISODES {
        return false;
    }
    if confidence < PROMOTION_CONFIDENCE_THRESHOLD {
        return false;
    }
    match first_observed {
        Some(observed) => {
            let age_days = (Utc::now() - observed).num_days();
            age_days >= HOT_TIER_MIN_AGE_DAYS
        }
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Database helpers
// ---------------------------------------------------------------------------

/// Find an existing procedure in the namespace with similar pattern embedding.
///
/// Uses cosine similarity via pgvector. Returns the best match if similarity
/// exceeds [`PATTERN_SIMILARITY_THRESHOLD`].
async fn find_similar_procedure(
    pool: &PgPool,
    namespace: &str,
    embedding: &Vector,
) -> Result<Option<Procedure>, ProcedureExtractionError> {
    let row = sqlx::query_as::<_, Procedure>(
        r#"
        SELECT *
        FROM loom_procedures
        WHERE namespace = $1
          AND deleted_at IS NULL
          AND embedding IS NOT NULL
          AND 1.0 - (embedding <=> $2::vector) >= $3
        ORDER BY embedding <=> $2::vector ASC
        LIMIT 1
        "#,
    )
    .bind(namespace)
    .bind(embedding)
    .bind(PATTERN_SIMILARITY_THRESHOLD)
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

/// Update the confidence score on a procedure.
async fn update_procedure_confidence(
    pool: &PgPool,
    procedure_id: Uuid,
    confidence: f64,
) -> Result<(), ProcedureExtractionError> {
    procedures::update_confidence(pool, procedure_id, confidence).await?;
    Ok(())
}

/// Promote a procedure to `promoted` evidence status.
async fn promote_procedure(
    pool: &PgPool,
    procedure_id: Uuid,
) -> Result<(), ProcedureExtractionError> {
    procedures::promote_to_promoted(pool, procedure_id).await?;
    Ok(())
}

/// Store a 768-dimension embedding on a procedure record.
async fn store_procedure_embedding(
    pool: &PgPool,
    procedure_id: Uuid,
    embedding: &Vector,
) -> Result<(), ProcedureExtractionError> {
    procedures::store_embedding(pool, procedure_id, embedding).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_confidence_is_0_3() {
        assert!((INITIAL_CONFIDENCE - 0.3).abs() < f64::EPSILON);
    }

    #[test]
    fn compute_new_confidence_increases() {
        let c1 = compute_new_confidence(0.3, 1);
        assert!(c1 > 0.3);
        assert!(c1 < 1.0);

        let c2 = compute_new_confidence(c1, 2);
        assert!(c2 > c1);
        assert!(c2 < 1.0);
    }

    #[test]
    fn compute_new_confidence_caps_at_1() {
        let c = compute_new_confidence(0.99, 10);
        assert!(c <= 1.0);
    }

    #[test]
    fn compute_new_confidence_diminishing_returns() {
        // Higher starting confidence should yield smaller increments.
        let low_start = compute_new_confidence(0.3, 1) - 0.3;
        let high_start = compute_new_confidence(0.8, 5) - 0.8;
        assert!(low_start > high_start);
    }

    #[test]
    fn compute_new_confidence_from_initial_reaches_promotion() {
        // Simulate multiple observations from initial confidence.
        let mut c = INITIAL_CONFIDENCE;
        let mut obs = 1;
        while c < PROMOTION_CONFIDENCE_THRESHOLD && obs < 20 {
            c = compute_new_confidence(c, obs);
            obs += 1;
        }
        // Should reach promotion threshold within a reasonable number of observations.
        assert!(
            c >= PROMOTION_CONFIDENCE_THRESHOLD,
            "confidence {c} should reach {PROMOTION_CONFIDENCE_THRESHOLD} within {obs} observations"
        );
    }

    #[test]
    fn should_promote_requires_both_criteria() {
        // Below confidence threshold.
        assert!(!should_promote(0.7, 5));
        // Below observation threshold.
        assert!(!should_promote(0.9, 2));
        // Both met.
        assert!(should_promote(0.8, 3));
        assert!(should_promote(0.9, 5));
        // Exact boundary values.
        assert!(should_promote(0.8, 3));
        assert!(!should_promote(0.79, 3));
        assert!(!should_promote(0.8, 2));
    }

    #[test]
    fn should_promote_boundary_values() {
        // Exactly at threshold.
        assert!(should_promote(PROMOTION_CONFIDENCE_THRESHOLD, PROMOTION_OBSERVATION_THRESHOLD));
        // Just below confidence.
        assert!(!should_promote(
            PROMOTION_CONFIDENCE_THRESHOLD - 0.01,
            PROMOTION_OBSERVATION_THRESHOLD
        ));
        // Just below observations.
        assert!(!should_promote(
            PROMOTION_CONFIDENCE_THRESHOLD,
            PROMOTION_OBSERVATION_THRESHOLD - 1
        ));
    }

    #[test]
    fn eligible_for_hot_tier_requires_all_criteria() {
        let old_date = Utc::now() - chrono::Duration::days(10);

        // All criteria met.
        assert!(eligible_for_hot_tier(3, Some(old_date), 0.8));

        // Not enough episodes.
        assert!(!eligible_for_hot_tier(2, Some(old_date), 0.8));

        // Too recent.
        let recent = Utc::now() - chrono::Duration::days(3);
        assert!(!eligible_for_hot_tier(3, Some(recent), 0.8));

        // Low confidence.
        assert!(!eligible_for_hot_tier(3, Some(old_date), 0.7));

        // No first_observed.
        assert!(!eligible_for_hot_tier(3, None, 0.8));
    }

    #[test]
    fn eligible_for_hot_tier_boundary_values() {
        // Exactly at 7 days.
        let exactly_7_days = Utc::now() - chrono::Duration::days(HOT_TIER_MIN_AGE_DAYS);
        assert!(eligible_for_hot_tier(
            HOT_TIER_MIN_EPISODES,
            Some(exactly_7_days),
            PROMOTION_CONFIDENCE_THRESHOLD
        ));

        // 6 days — not eligible.
        let six_days = Utc::now() - chrono::Duration::days(HOT_TIER_MIN_AGE_DAYS - 1);
        assert!(!eligible_for_hot_tier(
            HOT_TIER_MIN_EPISODES,
            Some(six_days),
            PROMOTION_CONFIDENCE_THRESHOLD
        ));
    }

    #[test]
    fn eligible_for_hot_tier_prevents_low_episode_count() {
        let old_date = Utc::now() - chrono::Duration::days(30);
        // High confidence, old enough, but only 1 episode.
        assert!(!eligible_for_hot_tier(1, Some(old_date), 0.95));
        // 2 episodes — still not enough.
        assert!(!eligible_for_hot_tier(2, Some(old_date), 0.95));
        // 3 episodes — eligible.
        assert!(eligible_for_hot_tier(3, Some(old_date), 0.95));
    }

    #[test]
    fn detect_patterns_empty_facts() {
        let patterns = detect_patterns(&[]);
        assert!(patterns.is_empty());
    }

    #[test]
    fn detect_patterns_single_fact_no_pattern() {
        let facts = vec![ExtractedFact {
            subject: "ServiceA".to_string(),
            predicate: "uses".to_string(),
            object: "Redis".to_string(),
            evidence_strength: Some("explicit".to_string()),
            temporal_markers: None,
            custom: false,
        }];
        let patterns = detect_patterns(&facts);
        assert!(patterns.is_empty());
    }

    #[test]
    fn detect_patterns_repeated_predicate() {
        let facts = vec![
            ExtractedFact {
                subject: "ServiceA".to_string(),
                predicate: "uses".to_string(),
                object: "Redis".to_string(),
                evidence_strength: Some("explicit".to_string()),
                temporal_markers: None,
                custom: false,
            },
            ExtractedFact {
                subject: "ServiceB".to_string(),
                predicate: "uses".to_string(),
                object: "PostgreSQL".to_string(),
                evidence_strength: Some("explicit".to_string()),
                temporal_markers: None,
                custom: false,
            },
        ];
        let patterns = detect_patterns(&facts);
        assert!(!patterns.is_empty());
        assert!(patterns[0].pattern.contains("uses"));
    }

    #[test]
    fn detect_patterns_chain() {
        let facts = vec![
            ExtractedFact {
                subject: "ServiceA".to_string(),
                predicate: "depends_on".to_string(),
                object: "ServiceB".to_string(),
                evidence_strength: Some("explicit".to_string()),
                temporal_markers: None,
                custom: false,
            },
            ExtractedFact {
                subject: "ServiceB".to_string(),
                predicate: "depends_on".to_string(),
                object: "ServiceC".to_string(),
                evidence_strength: Some("explicit".to_string()),
                temporal_markers: None,
                custom: false,
            },
        ];
        let patterns = detect_patterns(&facts);
        // Should detect both the repeated predicate and the chain.
        assert!(patterns.len() >= 2);
        let chain_pattern = patterns.iter().find(|p| p.pattern.contains("→"));
        assert!(chain_pattern.is_some());
    }

    #[test]
    fn detect_patterns_different_predicates_no_repeat() {
        let facts = vec![
            ExtractedFact {
                subject: "ServiceA".to_string(),
                predicate: "uses".to_string(),
                object: "Redis".to_string(),
                evidence_strength: Some("explicit".to_string()),
                temporal_markers: None,
                custom: false,
            },
            ExtractedFact {
                subject: "ServiceA".to_string(),
                predicate: "deployed_to".to_string(),
                object: "Production".to_string(),
                evidence_strength: Some("explicit".to_string()),
                temporal_markers: None,
                custom: false,
            },
        ];
        let patterns = detect_patterns(&facts);
        // No repeated predicates and no chains — should produce no patterns.
        assert!(patterns.is_empty());
    }

    #[test]
    fn detect_patterns_three_repeated_predicates() {
        let facts = vec![
            ExtractedFact {
                subject: "ServiceA".to_string(),
                predicate: "uses".to_string(),
                object: "Redis".to_string(),
                evidence_strength: Some("explicit".to_string()),
                temporal_markers: None,
                custom: false,
            },
            ExtractedFact {
                subject: "ServiceB".to_string(),
                predicate: "uses".to_string(),
                object: "PostgreSQL".to_string(),
                evidence_strength: Some("explicit".to_string()),
                temporal_markers: None,
                custom: false,
            },
            ExtractedFact {
                subject: "ServiceC".to_string(),
                predicate: "uses".to_string(),
                object: "Kafka".to_string(),
                evidence_strength: Some("explicit".to_string()),
                temporal_markers: None,
                custom: false,
            },
        ];
        let patterns = detect_patterns(&facts);
        assert!(!patterns.is_empty());
        // Pattern should mention all three subjects.
        let pattern_text = &patterns[0].pattern;
        assert!(pattern_text.contains("ServiceA"));
        assert!(pattern_text.contains("ServiceB"));
        assert!(pattern_text.contains("ServiceC"));
    }

    #[test]
    fn categorize_predicate_maps_correctly() {
        assert_eq!(categorize_predicate("uses"), "dependency");
        assert_eq!(categorize_predicate("depends_on"), "dependency");
        assert_eq!(categorize_predicate("integrates_with"), "dependency");
        assert_eq!(categorize_predicate("deployed_to"), "operational");
        assert_eq!(categorize_predicate("hosts"), "operational");
        assert_eq!(categorize_predicate("configured_with"), "operational");
        assert_eq!(categorize_predicate("decided"), "decisional");
        assert_eq!(categorize_predicate("decided_by"), "decisional");
        assert_eq!(categorize_predicate("implements"), "implementation");
        assert_eq!(categorize_predicate("implemented_by"), "implementation");
        assert_eq!(categorize_predicate("custom_pred"), "general");
        assert_eq!(categorize_predicate("unknown"), "general");
    }

    #[test]
    fn procedure_flagging_result_empty() {
        let result = ProcedureFlaggingResult {
            new_count: 0,
            updated_count: 0,
            promoted_count: 0,
            procedure_ids: vec![],
        };
        assert_eq!(result.new_count, 0);
        assert_eq!(result.updated_count, 0);
        assert_eq!(result.promoted_count, 0);
        assert!(result.procedure_ids.is_empty());
    }

    #[test]
    fn procedure_flagging_result_with_counts() {
        let id = Uuid::new_v4();
        let result = ProcedureFlaggingResult {
            new_count: 2,
            updated_count: 1,
            promoted_count: 1,
            procedure_ids: vec![id],
        };
        assert_eq!(result.new_count, 2);
        assert_eq!(result.updated_count, 1);
        assert_eq!(result.promoted_count, 1);
        assert_eq!(result.procedure_ids.len(), 1);
    }

    #[test]
    fn confidence_progression_simulation() {
        // Simulate the full confidence progression from initial to promotion.
        let mut confidence = INITIAL_CONFIDENCE;
        let mut observations_to_promote = 0;

        for obs in 1..=20 {
            confidence = compute_new_confidence(confidence, obs);
            observations_to_promote = obs;
            if should_promote(confidence, obs) {
                break;
            }
        }

        // Verify promotion is reachable.
        assert!(
            confidence >= PROMOTION_CONFIDENCE_THRESHOLD,
            "confidence {confidence} should reach threshold"
        );
        assert!(
            observations_to_promote >= PROMOTION_OBSERVATION_THRESHOLD,
            "need at least {PROMOTION_OBSERVATION_THRESHOLD} observations"
        );
    }

    #[test]
    fn pattern_similarity_threshold_is_reasonable() {
        // Threshold should be high enough to avoid false matches but not
        // so high that legitimate duplicates are missed.
        assert!(PATTERN_SIMILARITY_THRESHOLD > 0.8);
        assert!(PATTERN_SIMILARITY_THRESHOLD < 0.95);
    }

    #[test]
    fn detected_pattern_has_category() {
        let facts = vec![
            ExtractedFact {
                subject: "ServiceA".to_string(),
                predicate: "deployed_to".to_string(),
                object: "Production".to_string(),
                evidence_strength: Some("explicit".to_string()),
                temporal_markers: None,
                custom: false,
            },
            ExtractedFact {
                subject: "ServiceB".to_string(),
                predicate: "deployed_to".to_string(),
                object: "Staging".to_string(),
                evidence_strength: Some("explicit".to_string()),
                temporal_markers: None,
                custom: false,
            },
        ];
        let patterns = detect_patterns(&facts);
        assert!(!patterns.is_empty());
        assert_eq!(patterns[0].category.as_deref(), Some("operational"));
    }
}
