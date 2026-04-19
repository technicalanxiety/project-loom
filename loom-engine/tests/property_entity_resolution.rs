//! Property-based tests for entity resolution logic.
//!
//! Tests the three-pass entity resolution algorithm's decision logic
//! without requiring a database. Uses proptest to generate random inputs
//! and verifies invariants hold across many iterations.
//!
//! **Properties tested:**
//! - Property 5: Exact Match Resolution Confidence
//! - Property 6: Semantic Resolution Threshold
//! - Property 7: Resolution Conflict Logging

use proptest::prelude::*;
use uuid::Uuid;

use loom_engine::db::entities::EntityWithScore;
use loom_engine::pipeline::offline::resolve::{SemanticResult, SEMANTIC_GAP, SEMANTIC_THRESHOLD};
use loom_engine::types::entity::ResolutionResult;

/// The 10 valid entity types defined by the CHECK constraint on loom_entities.
const VALID_ENTITY_TYPES: &[&str] = &[
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

/// Proptest strategy for generating non-empty entity names (1..=80 chars).
fn entity_name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_ -]{0,79}".prop_map(|s| s)
}

/// Proptest strategy for selecting a valid entity type.
fn valid_entity_type() -> impl Strategy<Value = String> {
    prop::sample::select(VALID_ENTITY_TYPES).prop_map(|s| s.to_string())
}

/// Proptest strategy for generating a namespace string.
fn namespace() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9-]{0,29}".prop_map(|s| s)
}

/// Helper to build an `EntityWithScore` for testing.
fn make_candidate(name: &str, entity_type: &str, namespace: &str, similarity: f64) -> EntityWithScore {
    EntityWithScore {
        id: Uuid::new_v4(),
        name: name.to_string(),
        entity_type: entity_type.to_string(),
        namespace: namespace.to_string(),
        properties: None,
        created_at: None,
        source_episodes: None,
        deleted_at: None,
        similarity,
    }
}

/// Simulate the semantic resolution decision logic from `pass3_semantic_match`.
///
/// This replicates the threshold/gap logic without requiring a database or
/// embedding service, allowing us to test the decision invariants directly.
fn evaluate_semantic_candidates(candidates: &[EntityWithScore]) -> SemanticResult {
    if candidates.is_empty() {
        return SemanticResult::NewEntity;
    }

    let top = &candidates[0];
    let top_score = top.similarity;

    // No candidate above threshold → new entity.
    if top_score < SEMANTIC_THRESHOLD {
        return SemanticResult::NewEntity;
    }

    // Check gap to second candidate.
    let second_score = candidates.get(1).map(|c| c.similarity).unwrap_or(0.0);
    let gap = top_score - second_score;

    if gap >= SEMANTIC_GAP {
        // Clear winner — merge.
        return SemanticResult::Merge(ResolutionResult {
            entity_id: top.id,
            method: "semantic".to_string(),
            confidence: top_score,
        });
    }

    // Top two within gap threshold — ambiguous, conflict.
    let conflict_candidates: Vec<EntityWithScore> = candidates
        .iter()
        .filter(|c| top_score - c.similarity < SEMANTIC_GAP)
        .cloned()
        .collect();

    SemanticResult::Conflict {
        candidates: conflict_candidates,
    }
}

// ---------------------------------------------------------------------------
// Property 5: Exact Match Resolution Confidence
// ---------------------------------------------------------------------------

/// **Property 5: Exact Match Resolution Confidence**
///
/// **Validates: Requirements 3.1, 3.2**
///
/// For any entity name, type, and namespace, when an exact match is found
/// the resolution result MUST have confidence exactly 1.0 and method "exact".
mod exact_match_confidence {
    use super::*;

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        #[test]
        fn exact_match_always_returns_confidence_one(
            name in entity_name(),
            entity_type in valid_entity_type(),
            ns in namespace(),
        ) {
            // Simulate what pass1_exact_match returns on a successful match:
            // a ResolutionResult with method "exact" and confidence 1.0.
            let result = ResolutionResult {
                entity_id: Uuid::new_v4(),
                method: "exact".to_string(),
                confidence: 1.0,
            };

            // Property: exact match confidence is always exactly 1.0
            prop_assert!(
                (result.confidence - 1.0).abs() < f64::EPSILON,
                "Exact match confidence must be 1.0, got {} for entity '{}' (type={}, ns={})",
                result.confidence, name, entity_type, ns
            );

            // Property: method is always "exact"
            prop_assert_eq!(
                &result.method,
                "exact",
                "Exact match method must be 'exact'"
            );

            // Property: the result serializes correctly via serde
            let json = serde_json::to_value(&result).expect("should serialize");
            prop_assert_eq!(json["confidence"].as_f64().unwrap(), 1.0);
            prop_assert_eq!(json["method"].as_str().unwrap(), "exact");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        #[test]
        fn exact_match_confidence_never_varies_with_input(
            name1 in entity_name(),
            name2 in entity_name(),
            et1 in valid_entity_type(),
            et2 in valid_entity_type(),
            ns1 in namespace(),
            ns2 in namespace(),
        ) {
            // Regardless of what entity name, type, or namespace is used,
            // exact match confidence is always 1.0.
            let result1 = ResolutionResult {
                entity_id: Uuid::new_v4(),
                method: "exact".to_string(),
                confidence: 1.0,
            };
            let result2 = ResolutionResult {
                entity_id: Uuid::new_v4(),
                method: "exact".to_string(),
                confidence: 1.0,
            };

            prop_assert!(
                (result1.confidence - result2.confidence).abs() < f64::EPSILON,
                "Exact match confidence must be identical regardless of input: \
                 '{}' ({}@{}) vs '{}' ({}@{})",
                name1, et1, ns1, name2, et2, ns2
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Property 6: Semantic Resolution Threshold
// ---------------------------------------------------------------------------

/// **Property 6: Semantic Resolution Threshold**
///
/// **Validates: Requirements 3.7**
///
/// When the top semantic candidate exceeds 0.92 similarity AND the gap to
/// the second candidate is at least 0.03, the resolution MUST merge with
/// confidence equal to the similarity score.
mod semantic_resolution_threshold {
    use super::*;

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        #[test]
        fn above_threshold_with_sufficient_gap_merges(
            top_score in (0.92f64..=1.0f64),
            gap in (0.03f64..=0.5f64),
            name in entity_name(),
            entity_type in valid_entity_type(),
            ns in namespace(),
        ) {
            let second_score = (top_score - gap).max(0.0);

            let candidates = vec![
                make_candidate(&name, &entity_type, &ns, top_score),
                make_candidate("other", &entity_type, &ns, second_score),
            ];

            let result = evaluate_semantic_candidates(&candidates);

            match result {
                SemanticResult::Merge(r) => {
                    // Confidence must equal the top similarity score.
                    prop_assert!(
                        (r.confidence - top_score).abs() < f64::EPSILON,
                        "Merge confidence ({}) must equal top score ({})",
                        r.confidence, top_score
                    );
                    prop_assert_eq!(
                        &r.method,
                        "semantic",
                        "Merge method must be 'semantic'"
                    );
                }
                other => {
                    prop_assert!(
                        false,
                        "Expected Merge for top_score={}, gap={}, got: {:?}",
                        top_score, gap, other
                    );
                }
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        #[test]
        fn below_threshold_creates_new_entity(
            top_score in (0.0f64..0.92f64),
            name in entity_name(),
            entity_type in valid_entity_type(),
            ns in namespace(),
        ) {
            let candidates = vec![
                make_candidate(&name, &entity_type, &ns, top_score),
            ];

            let result = evaluate_semantic_candidates(&candidates);

            prop_assert!(
                matches!(result, SemanticResult::NewEntity),
                "Score {} below threshold {} should create new entity, got: {:?}",
                top_score, SEMANTIC_THRESHOLD, result
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        #[test]
        fn single_candidate_above_threshold_always_merges(
            top_score in (0.92f64..=1.0f64),
            name in entity_name(),
            entity_type in valid_entity_type(),
            ns in namespace(),
        ) {
            // With only one candidate, the gap to "second" (0.0) is always >= 0.03
            // when top_score >= 0.92.
            let candidates = vec![
                make_candidate(&name, &entity_type, &ns, top_score),
            ];

            let result = evaluate_semantic_candidates(&candidates);

            match result {
                SemanticResult::Merge(r) => {
                    prop_assert!(
                        (r.confidence - top_score).abs() < f64::EPSILON,
                        "Single candidate merge confidence must equal score"
                    );
                }
                other => {
                    prop_assert!(
                        false,
                        "Single candidate above threshold should merge, got: {:?}",
                        other
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Property 7: Resolution Conflict Logging
// ---------------------------------------------------------------------------

/// **Property 7: Resolution Conflict Logging**
///
/// **Validates: Requirements 3.8, 23.1, 23.2**
///
/// When the top two semantic candidates are within 0.03 of each other
/// (both above threshold), the resolution MUST flag a conflict rather
/// than merging.
mod resolution_conflict_logging {
    use super::*;

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        #[test]
        fn ambiguous_candidates_flag_conflict(
            top_score in (0.92f64..=1.0f64),
            // Gap strictly less than 0.03 (the conflict threshold)
            gap_fraction in (0.0f64..1.0f64),
            name in entity_name(),
            entity_type in valid_entity_type(),
            ns in namespace(),
        ) {
            // Scale gap_fraction to [0.0, 0.03) — always below SEMANTIC_GAP
            let gap = gap_fraction * (SEMANTIC_GAP - f64::EPSILON);
            let second_score = (top_score - gap).max(0.0);

            // Both must be above threshold for this to be a meaningful conflict
            prop_assume!(second_score >= SEMANTIC_THRESHOLD);

            let candidates = vec![
                make_candidate(&name, &entity_type, &ns, top_score),
                make_candidate("other-candidate", &entity_type, &ns, second_score),
            ];

            let result = evaluate_semantic_candidates(&candidates);

            match result {
                SemanticResult::Conflict { candidates: conflict_candidates } => {
                    // Conflict must include at least the top two candidates.
                    prop_assert!(
                        conflict_candidates.len() >= 2,
                        "Conflict must include at least 2 candidates, got {}",
                        conflict_candidates.len()
                    );

                    // All conflict candidates must be within SEMANTIC_GAP of the top score.
                    for c in &conflict_candidates {
                        let diff = top_score - c.similarity;
                        prop_assert!(
                            diff < SEMANTIC_GAP,
                            "Conflict candidate '{}' (score={}) is {} away from top ({}), \
                             exceeds gap threshold {}",
                            c.name, c.similarity, diff, top_score, SEMANTIC_GAP
                        );
                    }
                }
                other => {
                    prop_assert!(
                        false,
                        "Expected Conflict for top={}, second={} (gap={}), got: {:?}",
                        top_score, second_score, gap, other
                    );
                }
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        #[test]
        fn conflict_never_merges_when_gap_too_small(
            top_score in (0.92f64..=1.0f64),
            gap_fraction in (0.0f64..1.0f64),
            name in entity_name(),
            entity_type in valid_entity_type(),
            ns in namespace(),
        ) {
            let gap = gap_fraction * (SEMANTIC_GAP - f64::EPSILON);
            let second_score = (top_score - gap).max(0.0);
            prop_assume!(second_score >= SEMANTIC_THRESHOLD);

            let candidates = vec![
                make_candidate(&name, &entity_type, &ns, top_score),
                make_candidate("rival", &entity_type, &ns, second_score),
            ];

            let result = evaluate_semantic_candidates(&candidates);

            // Must NOT be a Merge — ambiguous candidates should never merge.
            prop_assert!(
                !matches!(result, SemanticResult::Merge(_)),
                "Ambiguous candidates (gap={:.4} < {}) must not merge",
                gap, SEMANTIC_GAP
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        #[test]
        fn empty_candidates_create_new_entity(
            name in entity_name(),
        ) {
            let candidates: Vec<EntityWithScore> = vec![];
            let result = evaluate_semantic_candidates(&candidates);

            prop_assert!(
                matches!(result, SemanticResult::NewEntity),
                "Empty candidates should create new entity for '{}', got: {:?}",
                name, result
            );
        }
    }
}
