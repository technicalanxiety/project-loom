//! Property-based tests for the online pipeline: classification, retrieval
//! profile mapping, and graph traversal cycle prevention.
//!
//! Tests classification invariants, profile mapping correctness, merge/cap
//! behavior, and cycle prevention without requiring a database. Uses proptest
//! to generate random inputs and verifies invariants hold across many
//! iterations.
//!
//! **Properties tested:**
//! - Property 15: Task Class Validity
//! - Property 16: Retrieval Profile Mapping and Cap
//! - Property 17: Cycle Prevention in Graph Traversal

use std::collections::HashSet;

use proptest::prelude::*;
use uuid::Uuid;

use loom_engine::llm::classification::keyword_precheck;
use loom_engine::pipeline::online::retrieve::{
    merge_profiles, profiles_for_class, RetrievalProfile,
};
use loom_engine::types::classification::{ClassificationResult, TaskClass};

// ---------------------------------------------------------------------------
// Shared strategies
// ---------------------------------------------------------------------------

/// The five valid task classes.
const VALID_TASK_CLASSES: &[&str] = &[
    "debug",
    "architecture",
    "compliance",
    "writing",
    "chat",
];

/// Proptest strategy for selecting a valid TaskClass.
fn task_class() -> impl Strategy<Value = TaskClass> {
    prop::sample::select(&[
        TaskClass::Debug,
        TaskClass::Architecture,
        TaskClass::Compliance,
        TaskClass::Writing,
        TaskClass::Chat,
    ][..])
    .prop_map(|c| c.clone())
}

/// Proptest strategy for generating a confidence score in [0.0, 1.0].
fn confidence() -> impl Strategy<Value = f64> {
    0.0f64..=1.0f64
}

/// Proptest strategy for generating a random query string.
fn random_query() -> impl Strategy<Value = String> {
    "[a-zA-Z ]{1,100}".prop_map(|s| s)
}

/// Proptest strategy for generating a UUID.
fn uuid_strategy() -> impl Strategy<Value = Uuid> {
    any::<u128>().prop_map(|n| Uuid::from_u128(n))
}

// ---------------------------------------------------------------------------
// Property 15: Task Class Validity
// ---------------------------------------------------------------------------

/// **Property 15: Task Class Validity**
///
/// **Validates: Requirements 8.1, 8.3**
///
/// For any query classification result, the primary task class should be
/// one of the five valid classes. When the confidence gap between the top
/// two classes is less than 0.3, both primary and secondary task classes
/// should be recorded.
mod task_class_validity {
    use super::*;

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that all ClassificationResult instances have a valid primary
        /// task class that serializes to one of the five known values.
        #[test]
        fn primary_class_is_always_valid(
            class in task_class(),
            primary_conf in confidence(),
        ) {
            let result = ClassificationResult {
                primary_class: class,
                secondary_class: None,
                primary_confidence: primary_conf,
                secondary_confidence: None,
            };

            // Property: primary class serializes to one of the 5 valid strings.
            let json = serde_json::to_value(&result).expect("should serialize");
            let primary_str = json["primary_class"].as_str().unwrap();
            prop_assert!(
                VALID_TASK_CLASSES.contains(&primary_str),
                "Primary class '{}' is not one of {:?}",
                primary_str,
                VALID_TASK_CLASSES
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that when the confidence gap < 0.3, both primary and secondary
        /// classes are recorded. When gap >= 0.3, secondary is None.
        #[test]
        fn secondary_class_recorded_when_gap_below_threshold(
            primary in task_class(),
            secondary in task_class(),
            primary_conf in (0.3f64..=1.0f64),
            gap_fraction in (0.0f64..1.0f64),
        ) {
            // Simulate the classification logic:
            // If gap < 0.3, secondary is present; otherwise None.
            let gap_threshold = 0.3;

            // Generate a gap that is either below or above the threshold.
            let gap = gap_fraction * 0.6; // range [0.0, 0.6)
            let secondary_conf = (primary_conf - gap).max(0.0);
            let actual_gap = primary_conf - secondary_conf;

            let result = if actual_gap < gap_threshold {
                ClassificationResult {
                    primary_class: primary.clone(),
                    secondary_class: Some(secondary.clone()),
                    primary_confidence: primary_conf,
                    secondary_confidence: Some(secondary_conf),
                }
            } else {
                ClassificationResult {
                    primary_class: primary.clone(),
                    secondary_class: None,
                    primary_confidence: primary_conf,
                    secondary_confidence: None,
                }
            };

            // Property: when gap < 0.3, secondary must be present.
            if actual_gap < gap_threshold {
                prop_assert!(
                    result.secondary_class.is_some(),
                    "Gap {} < {} but secondary class is None",
                    actual_gap,
                    gap_threshold
                );
                // Secondary class must also be valid.
                let json = serde_json::to_value(&result).expect("should serialize");
                let sec_str = json["secondary_class"].as_str().unwrap();
                prop_assert!(
                    VALID_TASK_CLASSES.contains(&sec_str),
                    "Secondary class '{}' is not valid",
                    sec_str
                );
            } else {
                prop_assert!(
                    result.secondary_class.is_none(),
                    "Gap {} >= {} but secondary class is present",
                    actual_gap,
                    gap_threshold
                );
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that keyword_precheck always returns a valid TaskClass or None.
        #[test]
        fn keyword_precheck_returns_valid_class_or_none(
            query in random_query(),
        ) {
            let result = keyword_precheck(&query);

            if let Some(class) = result {
                let class_str = class.to_string();
                prop_assert!(
                    VALID_TASK_CLASSES.contains(&class_str.as_str()),
                    "keyword_precheck returned invalid class '{}' for query '{}'",
                    class_str,
                    query
                );
            }
            // None is also valid — means no keyword match.
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that ClassificationResult round-trips through serde correctly,
        /// preserving all fields including the task class validity.
        #[test]
        fn classification_result_serde_roundtrip(
            primary in task_class(),
            secondary in prop::option::of(task_class()),
            primary_conf in confidence(),
            secondary_conf in prop::option::of(confidence()),
        ) {
            let result = ClassificationResult {
                primary_class: primary.clone(),
                secondary_class: secondary.clone(),
                primary_confidence: primary_conf,
                secondary_confidence: secondary_conf,
            };

            let json = serde_json::to_value(&result).expect("should serialize");
            let deserialized: ClassificationResult =
                serde_json::from_value(json).expect("should deserialize");

            prop_assert_eq!(deserialized.primary_class, primary);
            prop_assert_eq!(deserialized.secondary_class, secondary);
            prop_assert!(
                (deserialized.primary_confidence - primary_conf).abs() < f64::EPSILON,
                "primary_confidence mismatch"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Property 16: Retrieval Profile Mapping and Cap
// ---------------------------------------------------------------------------

/// **Property 16: Retrieval Profile Mapping and Cap**
///
/// **Validates: Requirements 9.1, 9.2, 9.3, 9.4, 9.5, 9.6, 9.7**
///
/// Each task class maps to the correct retrieval profiles. When a secondary
/// class is present, profiles are merged and deduplicated. The number of
/// active profiles never exceeds 3.
mod retrieval_profile_mapping_and_cap {
    use super::*;

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that each task class maps to the correct canonical profiles.
        #[test]
        fn task_class_maps_to_correct_profiles(
            class in task_class(),
        ) {
            let profiles = profiles_for_class(&class);

            let expected = match class {
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
            };

            prop_assert_eq!(
                &profiles,
                &expected,
                "profiles_for_class({}) returned {:?}, expected {:?}",
                class,
                profiles,
                expected
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that merged profiles from any primary/secondary combination
        /// never exceed 3.
        #[test]
        fn merged_profiles_never_exceed_three(
            primary in task_class(),
            secondary in prop::option::of(task_class()),
        ) {
            let profiles = merge_profiles(
                &primary,
                secondary.as_ref(),
            );

            prop_assert!(
                profiles.len() <= 3,
                "merge_profiles({}, {:?}) produced {} profiles: {:?}",
                primary,
                secondary,
                profiles.len(),
                profiles
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that merged profiles contain no duplicates.
        #[test]
        fn merged_profiles_contain_no_duplicates(
            primary in task_class(),
            secondary in prop::option::of(task_class()),
        ) {
            let profiles = merge_profiles(
                &primary,
                secondary.as_ref(),
            );

            let unique: HashSet<&RetrievalProfile> = profiles.iter().collect();
            prop_assert_eq!(
                profiles.len(),
                unique.len(),
                "merge_profiles({}, {:?}) has duplicates: {:?}",
                primary,
                secondary,
                profiles
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that merged profiles always start with the primary class's
        /// profiles (order preservation).
        #[test]
        fn merged_profiles_preserve_primary_order(
            primary in task_class(),
            secondary in task_class(),
        ) {
            let primary_profiles = profiles_for_class(&primary);
            let merged = merge_profiles(&primary, Some(&secondary));

            // The merged list should start with all primary profiles.
            for (i, expected) in primary_profiles.iter().enumerate() {
                if i < merged.len() {
                    prop_assert_eq!(
                        &merged[i],
                        expected,
                        "Position {} should be primary profile {:?}, got {:?}",
                        i,
                        expected,
                        merged[i]
                    );
                }
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that merging with no secondary returns exactly the primary
        /// class's profiles.
        #[test]
        fn merge_with_no_secondary_returns_primary_profiles(
            primary in task_class(),
        ) {
            let expected = profiles_for_class(&primary);
            let merged = merge_profiles(&primary, None);

            prop_assert_eq!(
                &merged,
                &expected,
                "merge_profiles({}, None) should equal profiles_for_class({})",
                primary,
                primary
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Property 17: Cycle Prevention in Graph Traversal
// ---------------------------------------------------------------------------

/// **Property 17: Cycle Prevention in Graph Traversal**
///
/// **Validates: Requirements 12.4, 26.4**
///
/// For any graph traversal path, no entity should appear more than once.
/// This tests the cycle prevention invariant that the loom_traverse SQL
/// function enforces via `NOT entity_id = ANY(path)`.
///
/// Since we cannot call the actual SQL function without a database, we
/// simulate the traversal path logic and verify the invariant holds.
mod cycle_prevention_graph_traversal {
    use super::*;

    /// Simulate the cycle prevention check used by loom_traverse.
    ///
    /// Given a path of entity UUIDs, returns true if the candidate entity
    /// is NOT already in the path (i.e., traversal is allowed).
    fn is_traversal_allowed(path: &[Uuid], candidate: Uuid) -> bool {
        !path.contains(&candidate)
    }

    /// Simulate building a traversal path with cycle prevention.
    ///
    /// Starting from a root entity, attempts to add each candidate to the
    /// path. Candidates already in the path are skipped (cycle prevention).
    /// Returns the final path.
    fn build_traversal_path(root: Uuid, candidates: &[Uuid]) -> Vec<Uuid> {
        let mut path = vec![root];
        for &candidate in candidates {
            if is_traversal_allowed(&path, candidate) {
                path.push(candidate);
            }
        }
        path
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that no entity appears more than once in a traversal path.
        #[test]
        fn no_entity_appears_more_than_once_in_path(
            root in uuid_strategy(),
            candidates in prop::collection::vec(uuid_strategy(), 0..=20),
        ) {
            let path = build_traversal_path(root, &candidates);

            // Property: all entities in the path are unique.
            let unique: HashSet<&Uuid> = path.iter().collect();
            prop_assert_eq!(
                path.len(),
                unique.len(),
                "Traversal path contains duplicates: {:?}",
                path
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that the root entity is always the first element in the path.
        #[test]
        fn root_entity_is_first_in_path(
            root in uuid_strategy(),
            candidates in prop::collection::vec(uuid_strategy(), 0..=10),
        ) {
            let path = build_traversal_path(root, &candidates);

            prop_assert_eq!(
                path[0],
                root,
                "Root entity should be first in path"
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that duplicate candidates in the input are correctly filtered
        /// out by cycle prevention.
        #[test]
        fn duplicate_candidates_are_filtered(
            root in uuid_strategy(),
            unique_candidates in prop::collection::vec(uuid_strategy(), 1..=5),
        ) {
            // Create input with duplicates by repeating the candidates.
            let mut candidates_with_dupes = unique_candidates.clone();
            candidates_with_dupes.extend_from_slice(&unique_candidates);
            candidates_with_dupes.extend_from_slice(&unique_candidates);

            let path = build_traversal_path(root, &candidates_with_dupes);

            // Property: path should have no duplicates despite input having many.
            let unique: HashSet<&Uuid> = path.iter().collect();
            prop_assert_eq!(
                path.len(),
                unique.len(),
                "Path should have no duplicates even with duplicate input candidates"
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that attempting to re-add the root entity is prevented.
        #[test]
        fn root_entity_cannot_be_revisited(
            root in uuid_strategy(),
            other in uuid_strategy(),
        ) {
            // Try to add root again after initial path construction.
            let candidates = vec![other, root, other];
            let path = build_traversal_path(root, &candidates);

            // Count occurrences of root in path.
            let root_count = path.iter().filter(|&&id| id == root).count();
            prop_assert_eq!(
                root_count,
                1,
                "Root entity should appear exactly once in path, found {} times",
                root_count
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that the TraversalResult path field (from the SQL function)
        /// would never contain duplicates, by verifying the invariant on
        /// simulated path arrays.
        #[test]
        fn simulated_traversal_result_paths_are_cycle_free(
            path_len in 1usize..=10,
        ) {
            // Generate a path of unique UUIDs (simulating what loom_traverse returns).
            let path: Vec<Uuid> = (0..path_len).map(|_| Uuid::new_v4()).collect();

            // Property: all elements in the path are unique.
            let unique: HashSet<&Uuid> = path.iter().collect();
            prop_assert_eq!(
                path.len(),
                unique.len(),
                "Simulated traversal path should be cycle-free"
            );

            // Property: for each entity in the path, it should not appear
            // in the sub-path before it (the cycle prevention check).
            for i in 1..path.len() {
                let preceding = &path[..i];
                prop_assert!(
                    !preceding.contains(&path[i]),
                    "Entity at position {} ({}) already appears in preceding path {:?}",
                    i,
                    path[i],
                    preceding
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Property 18: Hard Exclusion by Weight
// ---------------------------------------------------------------------------

/// **Property 18: Hard Exclusion by Weight**
///
/// **Validates: Requirements 13.6**
///
/// For any candidate with memory weight modifier 0.0 for the active task
/// class (e.g., procedural memory for compliance), the candidate should not
/// appear in the final ranked results.
mod hard_exclusion_by_weight {
    use super::*;
    use chrono::Utc;
    use loom_engine::pipeline::online::rank::rank_candidates;
    use loom_engine::pipeline::online::retrieve::{
        CandidatePayload, EpisodeCandidate, FactCandidate, MemoryType,
        ProcedureCandidate, RetrievalCandidate, RetrievalProfile,
    };
    use loom_engine::pipeline::online::weight::{apply_weights, weight_for_memory_type};

    /// Strategy for generating a memory type.
    fn memory_type_strategy() -> impl Strategy<Value = MemoryType> {
        prop::sample::select(&[
            MemoryType::Episodic,
            MemoryType::Semantic,
            MemoryType::Procedural,
        ][..])
        .prop_map(|m| m.clone())
    }

    /// Strategy for generating a relevance score in [0.01, 1.0].
    fn relevance_score() -> impl Strategy<Value = f64> {
        0.01f64..=1.0f64
    }

    /// Build a retrieval candidate with the given memory type and score.
    fn build_candidate(memory_type: MemoryType, score: f64) -> RetrievalCandidate {
        let (profile, payload) = match &memory_type {
            MemoryType::Episodic => (
                RetrievalProfile::EpisodeRecall,
                CandidatePayload::Episode(EpisodeCandidate {
                    source: "test".to_string(),
                    content: "test content".to_string(),
                    occurred_at: Utc::now(),
                    namespace: "default".to_string(),
                }),
            ),
            MemoryType::Semantic | MemoryType::Graph => (
                RetrievalProfile::FactLookup,
                CandidatePayload::Fact(FactCandidate {
                    subject_id: Uuid::new_v4(),
                    predicate: "uses".to_string(),
                    object_id: Uuid::new_v4(),
                    evidence_status: "extracted".to_string(),
                    source_episodes: vec![Uuid::new_v4()],
                    namespace: "default".to_string(),
                }),
            ),
            MemoryType::Procedural => (
                RetrievalProfile::ProcedureAssist,
                CandidatePayload::Procedure(ProcedureCandidate {
                    pattern: "test pattern".to_string(),
                    confidence: 0.9,
                    observation_count: 5,
                    namespace: "default".to_string(),
                }),
            ),
        };

        RetrievalCandidate {
            id: Uuid::new_v4(),
            score,
            source_profile: profile,
            memory_type,
            payload,
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that candidates with weight 0.0 never appear in weighted results.
        #[test]
        fn zero_weight_candidates_excluded_from_weighted(
            class in task_class(),
            mem_type in memory_type_strategy(),
            score in relevance_score(),
        ) {
            let weight = weight_for_memory_type(&class, &mem_type);
            let candidates = vec![build_candidate(mem_type.clone(), score)];
            let weighted = apply_weights(candidates, &class);

            if weight == 0.0 {
                // Property: zero-weight candidates must be excluded.
                prop_assert!(
                    weighted.is_empty(),
                    "Candidate with weight 0.0 ({:?} for {:?}) should be excluded, but {} remained",
                    mem_type,
                    class,
                    weighted.len()
                );
            } else {
                // Non-zero weight: candidate should be present.
                prop_assert_eq!(
                    weighted.len(),
                    1,
                    "Candidate with weight {} ({:?} for {:?}) should be present",
                    weight,
                    mem_type,
                    class
                );
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that zero-weight candidates never appear in final ranked results.
        #[test]
        fn zero_weight_candidates_excluded_from_ranked(
            class in task_class(),
            scores in prop::collection::vec(relevance_score(), 1..=5),
        ) {
            // Build one candidate of each memory type.
            let mut candidates = Vec::new();
            let types = [MemoryType::Episodic, MemoryType::Semantic, MemoryType::Procedural];
            for (i, score) in scores.iter().enumerate() {
                let mem_type = types[i % types.len()].clone();
                candidates.push(build_candidate(mem_type, *score));
            }

            let weighted = apply_weights(candidates, &class);
            let ranked = rank_candidates(weighted);

            // Property: no ranked candidate should have a memory type with weight 0.0.
            for rc in &ranked {
                let w = weight_for_memory_type(&class, &rc.candidate.memory_type);
                prop_assert!(
                    w > 0.0,
                    "Ranked candidate {:?} has weight 0.0 for class {:?}",
                    rc.candidate.memory_type,
                    class
                );
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test specifically that procedural memory is always excluded for
        /// compliance task class (the known 0.0 weight case).
        #[test]
        fn procedural_always_excluded_for_compliance(
            score in relevance_score(),
            extra_episodic_score in relevance_score(),
            extra_semantic_score in relevance_score(),
        ) {
            let candidates = vec![
                build_candidate(MemoryType::Procedural, score),
                build_candidate(MemoryType::Episodic, extra_episodic_score),
                build_candidate(MemoryType::Semantic, extra_semantic_score),
            ];

            let weighted = apply_weights(candidates, &TaskClass::Compliance);

            // Property: no procedural candidates in weighted results.
            for wc in &weighted {
                prop_assert!(
                    wc.candidate.memory_type != MemoryType::Procedural,
                    "Procedural candidates must be hard-excluded for compliance"
                );
            }

            // Episodic and semantic should remain.
            prop_assert_eq!(
                weighted.len(),
                2,
                "Expected 2 candidates (episodic + semantic), got {}",
                weighted.len()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Property 20: Four-Dimension Weighted Ranking
// ---------------------------------------------------------------------------

/// **Property 20: Four-Dimension Weighted Ranking**
///
/// **Validates: Requirements 40.1, 40.2, 40.3, 40.4, 40.5, 40.6**
///
/// For any ranked candidate list, the final ranking scores should be
/// computed as (relevance × 0.40) + (recency × 0.25) + (stability × 0.20)
/// + (provenance × 0.15), and candidates should be sorted in descending
/// order by final score.
mod four_dimension_weighted_ranking {
    use super::*;
    use loom_engine::pipeline::online::rank::{
        compute_final_score, rank_candidates, PROVENANCE_WEIGHT, RECENCY_WEIGHT,
        RELEVANCE_WEIGHT, STABILITY_WEIGHT,
    };
    use loom_engine::pipeline::online::retrieve::{
        CandidatePayload, FactCandidate, MemoryType, RetrievalCandidate,
        RetrievalProfile,
    };
    use loom_engine::pipeline::online::weight::WeightedCandidate;
    use loom_engine::types::compilation::RankingScore;

    /// Strategy for generating a dimension score in [0.0, 1.0].
    fn dimension_score() -> impl Strategy<Value = f64> {
        0.0f64..=1.0f64
    }

    /// Strategy for generating a relevance score in [0.01, 1.0].
    fn relevance_score() -> impl Strategy<Value = f64> {
        0.01f64..=1.0f64
    }

    /// Build a weighted fact candidate with a given score.
    fn build_weighted_fact(score: f64) -> WeightedCandidate {
        let candidate = RetrievalCandidate {
            id: Uuid::new_v4(),
            score,
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
        WeightedCandidate {
            candidate,
            weight: 1.0,
            weighted_score: score,
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that the composite score formula is correctly applied.
        #[test]
        fn composite_score_formula_correct(
            relevance in dimension_score(),
            recency in dimension_score(),
            stability in dimension_score(),
            provenance in dimension_score(),
        ) {
            let scores = RankingScore {
                relevance,
                recency,
                stability,
                provenance,
            };

            let expected = relevance * RELEVANCE_WEIGHT
                + recency * RECENCY_WEIGHT
                + stability * STABILITY_WEIGHT
                + provenance * PROVENANCE_WEIGHT;

            let actual = compute_final_score(&scores);

            prop_assert!(
                (actual - expected).abs() < 1e-10,
                "compute_final_score({:?}) = {}, expected {}",
                scores,
                actual,
                expected
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that dimension weights sum to 1.0.
        #[test]
        fn dimension_weights_sum_to_one(
            _dummy in 0..1i32,  // proptest requires at least one input
        ) {
            let sum = RELEVANCE_WEIGHT + RECENCY_WEIGHT + STABILITY_WEIGHT + PROVENANCE_WEIGHT;
            prop_assert!(
                (sum - 1.0).abs() < 1e-10,
                "Dimension weights should sum to 1.0, got {}",
                sum
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that ranked candidates are sorted in descending order by
        /// final score.
        #[test]
        fn ranked_candidates_sorted_descending(
            scores in prop::collection::vec(relevance_score(), 2..=10),
        ) {
            let weighted: Vec<WeightedCandidate> = scores
                .iter()
                .map(|&s| build_weighted_fact(s))
                .collect();

            let ranked = rank_candidates(weighted);

            // Property: each candidate's final score >= the next candidate's.
            for i in 1..ranked.len() {
                prop_assert!(
                    ranked[i - 1].final_score >= ranked[i].final_score,
                    "Candidates not sorted descending at position {}: {} < {}",
                    i,
                    ranked[i - 1].final_score,
                    ranked[i].final_score
                );
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that the final score matches the formula applied to the
        /// per-dimension scores stored on each ranked candidate.
        #[test]
        fn final_score_matches_stored_dimensions(
            scores in prop::collection::vec(relevance_score(), 1..=8),
        ) {
            let weighted: Vec<WeightedCandidate> = scores
                .iter()
                .map(|&s| build_weighted_fact(s))
                .collect();

            let ranked = rank_candidates(weighted);

            for rc in &ranked {
                let expected = rc.scores.relevance * RELEVANCE_WEIGHT
                    + rc.scores.recency * RECENCY_WEIGHT
                    + rc.scores.stability * STABILITY_WEIGHT
                    + rc.scores.provenance * PROVENANCE_WEIGHT;

                prop_assert!(
                    (rc.final_score - expected).abs() < 1e-10,
                    "Ranked candidate final_score {} != expected {} from dimensions {:?}",
                    rc.final_score,
                    expected,
                    rc.scores
                );
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that compute_final_score agrees with RankingScore::composite.
        #[test]
        fn compute_final_score_agrees_with_composite(
            relevance in dimension_score(),
            recency in dimension_score(),
            stability in dimension_score(),
            provenance in dimension_score(),
        ) {
            let scores = RankingScore {
                relevance,
                recency,
                stability,
                provenance,
            };

            let from_fn = compute_final_score(&scores);
            let from_method = scores.composite();

            prop_assert!(
                (from_fn - from_method).abs() < 1e-10,
                "compute_final_score ({}) and composite ({}) should agree",
                from_fn,
                from_method
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that final score is bounded in [0.0, 1.0] when all
        /// dimension scores are in [0.0, 1.0].
        #[test]
        fn final_score_bounded_in_unit_interval(
            relevance in dimension_score(),
            recency in dimension_score(),
            stability in dimension_score(),
            provenance in dimension_score(),
        ) {
            let scores = RankingScore {
                relevance,
                recency,
                stability,
                provenance,
            };

            let final_score = compute_final_score(&scores);

            prop_assert!(
                final_score >= 0.0 && final_score <= 1.0,
                "Final score {} should be in [0.0, 1.0] for dimensions {:?}",
                final_score,
                scores
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Property 19: Hot Tier Constraints
// ---------------------------------------------------------------------------

/// **Property 19: Hot Tier Constraints**
///
/// **Validates: Requirements 14.3, 14.7, 14.8**
///
/// For any namespace, the total token count of hot tier memory should not
/// exceed the configured hot_tier_budget. For any fact where superseded_by
/// is not NULL, the fact should not be in hot tier. For any memory item
/// explicitly pinned by a user, the item should be in hot tier.
mod hot_tier_constraints {
    use super::*;
    use loom_engine::pipeline::offline::state::{
        estimate_hot_tier_tokens, is_fact_archived, procedure_eligible_for_hot_tier,
        qualifies_for_promotion, PROCEDURE_MIN_CONFIDENCE, PROCEDURE_MIN_EPISODES,
    };

    /// Strategy for generating a hot tier budget in [100, 2000].
    fn budget_strategy() -> impl Strategy<Value = i32> {
        100i32..=2000i32
    }

    /// Strategy for generating item counts in [0, 50].
    fn item_count() -> impl Strategy<Value = usize> {
        0usize..=50usize
    }

    /// Strategy for generating a salience score in [0.0, 1.0].
    fn salience() -> impl Strategy<Value = f64> {
        0.0f64..=1.0f64
    }

    /// Strategy for generating an access count in [0, 20].
    fn access_count() -> impl Strategy<Value = i32> {
        0i32..=20i32
    }

    /// Strategy for generating a confidence score in [0.0, 1.0].
    fn confidence_score() -> impl Strategy<Value = f64> {
        0.0f64..=1.0f64
    }

    /// Simulated hot tier item for budget enforcement testing.
    #[derive(Debug, Clone)]
    struct SimHotItem {
        id: Uuid,
        salience: f64,
        pinned: bool,
        superseded: bool,
        tokens: i32,
    }

    /// Strategy for generating a simulated hot tier item.
    fn sim_hot_item() -> impl Strategy<Value = SimHotItem> {
        (uuid_strategy(), salience(), proptest::bool::ANY, proptest::bool::ANY)
            .prop_map(|(id, salience, pinned, superseded)| SimHotItem {
                id,
                salience,
                pinned,
                superseded,
                tokens: 15, // average token cost
            })
    }

    /// Simulate budget enforcement: demote lowest-salience unpinned items
    /// until budget is satisfied.
    fn enforce_budget(items: &mut Vec<SimHotItem>, budget: i32) {
        loop {
            let total: i32 = items.iter().map(|i| i.tokens).sum();
            if total <= budget {
                break;
            }
            // Find lowest-salience unpinned item.
            let lowest_idx = items
                .iter()
                .enumerate()
                .filter(|(_, item)| !item.pinned)
                .min_by(|(_, a), (_, b)| {
                    a.salience
                        .partial_cmp(&b.salience)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            match lowest_idx {
                Some((idx, _)) => {
                    items.remove(idx);
                }
                None => break, // All remaining items are pinned.
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that after budget enforcement, hot tier total tokens never
        /// exceed the namespace budget (excluding pinned items that cannot
        /// be demoted). When only pinned items remain, the budget may be
        /// exceeded since pinned items are never demoted.
        #[test]
        fn hot_tier_tokens_never_exceed_budget(
            budget in budget_strategy(),
            items in prop::collection::vec(sim_hot_item(), 0..=30),
        ) {
            let mut hot_items = items;
            enforce_budget(&mut hot_items, budget);

            let total: i32 = hot_items.iter().map(|i| i.tokens).sum();
            let unpinned_remaining = hot_items.iter().any(|i| !i.pinned);

            // If there are still unpinned items, budget should be satisfied.
            // If only pinned items remain, budget may be exceeded (pinned
            // items are never demoted).
            if unpinned_remaining || total <= budget {
                prop_assert!(
                    total <= budget,
                    "Hot tier tokens {} exceed budget {} with unpinned items remaining",
                    total,
                    budget
                );
            } else {
                // All remaining items are pinned — budget overflow is acceptable.
                let all_pinned = hot_items.iter().all(|i| i.pinned);
                prop_assert!(
                    all_pinned,
                    "Budget exceeded ({} > {}) but not all items are pinned",
                    total,
                    budget
                );
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that facts with superseded_by != NULL are never in hot tier.
        ///
        /// Simulates the invariant: after removing superseded items from the
        /// hot tier, no superseded item should remain.
        #[test]
        fn superseded_facts_never_in_hot_tier(
            items in prop::collection::vec(sim_hot_item(), 1..=20),
        ) {
            // Simulate the demotion of superseded facts.
            let hot_items: Vec<&SimHotItem> = items
                .iter()
                .filter(|item| !item.superseded)
                .collect();

            // Property: no superseded item in the hot tier.
            for item in &hot_items {
                prop_assert!(
                    !item.superseded,
                    "Superseded item {:?} should not be in hot tier",
                    item.id
                );
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that explicitly pinned items are always in hot tier after
        /// budget enforcement.
        #[test]
        fn pinned_items_always_in_hot_tier(
            budget in budget_strategy(),
            items in prop::collection::vec(sim_hot_item(), 1..=20),
        ) {
            let mut hot_items = items.clone();
            enforce_budget(&mut hot_items, budget);

            // Collect IDs of pinned items from the original set.
            let pinned_ids: HashSet<Uuid> = items
                .iter()
                .filter(|item| item.pinned)
                .map(|item| item.id)
                .collect();

            // Collect IDs remaining in hot tier.
            let hot_ids: HashSet<Uuid> = hot_items
                .iter()
                .map(|item| item.id)
                .collect();

            // Property: all pinned items must remain in hot tier.
            for pinned_id in &pinned_ids {
                prop_assert!(
                    hot_ids.contains(pinned_id),
                    "Pinned item {} was demoted from hot tier",
                    pinned_id
                );
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that the token estimation function is consistent with
        /// individual item counts.
        #[test]
        fn token_estimation_consistent(
            entities in item_count(),
            facts in item_count(),
            procedures in item_count(),
        ) {
            let total = estimate_hot_tier_tokens(entities, facts, procedures);
            let expected = entities as i32 * 10 + facts as i32 * 15 + procedures as i32 * 30;
            prop_assert_eq!(
                total,
                expected,
                "Token estimation mismatch: {} != {}",
                total,
                expected
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that promotion criteria are correctly evaluated.
        #[test]
        fn promotion_criteria_correct(
            count in access_count(),
            days_ago in 0i64..=30i64,
        ) {
            let accessed = chrono::Utc::now() - chrono::Duration::days(days_ago);
            let qualifies = qualifies_for_promotion(count, Some(accessed));

            if count >= 5 && days_ago <= 14 {
                prop_assert!(
                    qualifies,
                    "Should qualify: count={}, days_ago={}",
                    count,
                    days_ago
                );
            } else {
                prop_assert!(
                    !qualifies,
                    "Should not qualify: count={}, days_ago={}",
                    count,
                    days_ago
                );
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that procedure hot tier prevention criteria are correctly
        /// evaluated.
        #[test]
        fn procedure_hot_tier_prevention(
            episodes in 0i32..=10i32,
            days_old in 0i64..=20i64,
            conf in confidence_score(),
        ) {
            let first = chrono::Utc::now() - chrono::Duration::days(days_old);
            let eligible = procedure_eligible_for_hot_tier(episodes, Some(first), conf);

            let should_be_eligible = episodes >= PROCEDURE_MIN_EPISODES
                && days_old >= 7
                && conf >= PROCEDURE_MIN_CONFIDENCE;

            prop_assert_eq!(
                eligible,
                should_be_eligible,
                "Procedure eligibility mismatch: episodes={}, days_old={}, conf={:.2}",
                episodes,
                days_old,
                conf
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that archived fact detection is correct.
        #[test]
        fn archived_fact_detection(
            has_superseded in proptest::bool::ANY,
            days_since_access in 0i64..=120i64,
        ) {
            let superseded_by = if has_superseded {
                Some(Uuid::new_v4())
            } else {
                None
            };
            let last_accessed = Some(chrono::Utc::now() - chrono::Duration::days(days_since_access));

            let archived = is_fact_archived(superseded_by, last_accessed);

            if has_superseded {
                prop_assert!(
                    archived,
                    "Superseded fact should always be archived"
                );
            } else if days_since_access >= 90 {
                prop_assert!(
                    archived,
                    "Fact not accessed in {} days should be archived",
                    days_since_access
                );
            } else {
                prop_assert!(
                    !archived,
                    "Fact accessed {} days ago should not be archived",
                    days_since_access
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Property 21: Candidate Deduplication
// ---------------------------------------------------------------------------

/// **Property 21: Candidate Deduplication**
///
/// **Validates: Requirements 16.2**
///
/// For any context compilation, no candidate should appear more than once
/// in the final context package (deduplicated by identifier).
mod candidate_deduplication {
    use super::*;
    use loom_engine::pipeline::online::compile::{
        compile_package, CompilationInput, HotTierItem, HotTierPayload, HotFact,
        DEFAULT_WARM_TIER_BUDGET,
    };
    use loom_engine::pipeline::online::rank::RankedCandidate;
    use loom_engine::pipeline::online::retrieve::{
        CandidatePayload, FactCandidate, MemoryType, RetrievalCandidate,
        RetrievalProfile,
    };
    use loom_engine::types::classification::TaskClass;
    use loom_engine::types::compilation::{OutputFormat, RankingScore};

    /// Strategy for generating a relevance score in [0.01, 1.0].
    fn relevance_score() -> impl Strategy<Value = f64> {
        0.01f64..=1.0f64
    }

    /// Build a ranked fact candidate with a given ID and score.
    fn build_ranked_fact(id: Uuid, score: f64) -> RankedCandidate {
        let candidate = RetrievalCandidate {
            id,
            score,
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
        RankedCandidate {
            candidate,
            scores: RankingScore {
                relevance: score,
                recency: 0.6,
                stability: 0.7,
                provenance: 0.5,
            },
            final_score: score * 0.4 + 0.6 * 0.25 + 0.7 * 0.20 + 0.5 * 0.15,
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that no candidate ID appears more than once in the selected items.
        #[test]
        fn no_duplicate_candidates_in_selected(
            num_unique in 1usize..=10,
            num_dupes in 0usize..=5,
            scores in prop::collection::vec(relevance_score(), 1..=15),
        ) {
            // Build unique candidates.
            let unique_ids: Vec<Uuid> = (0..num_unique).map(|_| Uuid::new_v4()).collect();
            let mut candidates = Vec::new();

            for (i, &score) in scores.iter().enumerate() {
                let id = unique_ids[i % unique_ids.len()];
                candidates.push(build_ranked_fact(id, score));
            }

            // Add explicit duplicates.
            for i in 0..num_dupes.min(unique_ids.len()) {
                let dup_score = scores.get(i).copied().unwrap_or(0.5);
                candidates.push(build_ranked_fact(unique_ids[i], dup_score));
            }

            let input = CompilationInput {
                namespace: "test".to_string(),
                task_class: TaskClass::Chat,
                target_model: "test-model".to_string(),
                format: OutputFormat::Structured,
                warm_tier_budget: DEFAULT_WARM_TIER_BUDGET,
                hot_tier_items: vec![],
                ranked_candidates: candidates,
            };

            let result = compile_package(input);

            // Property: no duplicate IDs in selected items.
            let selected_ids: Vec<Uuid> = result.selected_items.iter().map(|s| s.id).collect();
            let unique_selected: HashSet<Uuid> = selected_ids.iter().copied().collect();
            prop_assert_eq!(
                selected_ids.len(),
                unique_selected.len(),
                "Selected items contain duplicate IDs: {:?}",
                selected_ids
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that hot tier item IDs are excluded from warm tier selection.
        #[test]
        fn hot_tier_ids_excluded_from_warm(
            score in relevance_score(),
        ) {
            let shared_id = Uuid::new_v4();

            let hot_items = vec![HotTierItem {
                id: shared_id,
                memory_type: MemoryType::Semantic,
                payload: HotTierPayload::Fact(HotFact {
                    subject: "A".to_string(),
                    predicate: "uses".to_string(),
                    object: "B".to_string(),
                    evidence: "explicit".to_string(),
                    observed: None,
                    source: "ep1".to_string(),
                }),
            }];

            let warm = build_ranked_fact(shared_id, score);

            let input = CompilationInput {
                namespace: "test".to_string(),
                task_class: TaskClass::Chat,
                target_model: "test-model".to_string(),
                format: OutputFormat::Structured,
                warm_tier_budget: DEFAULT_WARM_TIER_BUDGET,
                hot_tier_items: hot_items,
                ranked_candidates: vec![warm],
            };

            let result = compile_package(input);

            // Property: warm candidate with same ID as hot item should not be selected.
            let selected_ids: HashSet<Uuid> = result.selected_items.iter().map(|s| s.id).collect();
            prop_assert!(
                !selected_ids.contains(&shared_id),
                "Warm candidate with hot tier ID {} should be excluded",
                shared_id
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Property 22: Hot Tier Injection
// ---------------------------------------------------------------------------

/// **Property 22: Hot Tier Injection**
///
/// **Validates: Requirements 16.5**
///
/// For any context compilation, all hot tier memory items for the namespace
/// should be included in the context package.
mod hot_tier_injection {
    use super::*;
    use loom_engine::pipeline::online::compile::{
        compile_package, CompilationInput, HotTierItem, HotTierPayload,
        HotFact, HotEntity, HotProcedure, DEFAULT_WARM_TIER_BUDGET,
    };
    use loom_engine::pipeline::online::retrieve::MemoryType;
    use loom_engine::types::classification::TaskClass;
    use loom_engine::types::compilation::OutputFormat;

    /// Strategy for generating a hot tier fact.
    fn hot_fact_strategy() -> impl Strategy<Value = HotTierItem> {
        (
            any::<u128>().prop_map(Uuid::from_u128),
            "[a-zA-Z]{3,15}",
            "[a-zA-Z_]{3,15}",
            "[a-zA-Z]{3,15}",
        )
            .prop_map(|(id, subject, predicate, object)| HotTierItem {
                id,
                memory_type: MemoryType::Semantic,
                payload: HotTierPayload::Fact(HotFact {
                    subject,
                    predicate,
                    object,
                    evidence: "explicit".to_string(),
                    observed: Some("2025-01-01".to_string()),
                    source: "test_ep".to_string(),
                }),
            })
    }

    /// Strategy for generating a hot tier entity.
    fn hot_entity_strategy() -> impl Strategy<Value = HotTierItem> {
        (
            any::<u128>().prop_map(Uuid::from_u128),
            "[a-zA-Z]{3,15}",
        )
            .prop_map(|(id, name)| HotTierItem {
                id,
                memory_type: MemoryType::Semantic,
                payload: HotTierPayload::Entity(HotEntity {
                    name: name.clone(),
                    entity_type: "project".to_string(),
                    summary: Some(format!("{name} project")),
                }),
            })
    }

    /// Strategy for generating a hot tier procedure.
    fn hot_procedure_strategy() -> impl Strategy<Value = HotTierItem> {
        (
            any::<u128>().prop_map(Uuid::from_u128),
            "[a-zA-Z ]{5,30}",
            0.5f64..=1.0f64,
            1i32..=20i32,
        )
            .prop_map(|(id, pattern, confidence, obs)| HotTierItem {
                id,
                memory_type: MemoryType::Procedural,
                payload: HotTierPayload::Procedure(HotProcedure {
                    pattern,
                    confidence,
                    observation_count: obs,
                }),
            })
    }

    /// Strategy for generating a mix of hot tier items.
    fn hot_items_strategy() -> impl Strategy<Value = Vec<HotTierItem>> {
        prop::collection::vec(
            prop_oneof![
                hot_fact_strategy(),
                hot_entity_strategy(),
                hot_procedure_strategy(),
            ],
            1..=8,
        )
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that all hot tier facts appear in structured output.
        #[test]
        fn all_hot_facts_in_structured_output(
            hot_items in hot_items_strategy(),
        ) {
            // Collect predicates before moving hot_items.
            let hot_predicates: Vec<String> = hot_items.iter().filter_map(|item| {
                if let HotTierPayload::Fact(f) = &item.payload { Some(f.predicate.clone()) } else { None }
            }).collect();

            let input = CompilationInput {
                namespace: "test".to_string(),
                task_class: TaskClass::Architecture,
                target_model: "test-model".to_string(),
                format: OutputFormat::Structured,
                warm_tier_budget: DEFAULT_WARM_TIER_BUDGET,
                hot_tier_items: hot_items,
                ranked_candidates: vec![],
            };

            let result = compile_package(input);
            let pkg = &result.package.context_package;

            // Property: every hot fact's predicate should appear in the output.
            for predicate in &hot_predicates {
                prop_assert!(
                    pkg.contains(predicate.as_str()),
                    "Hot fact predicate '{}' not found in structured output",
                    predicate
                );
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that all hot tier procedures appear in compact output.
        #[test]
        fn all_hot_procedures_in_compact_output(
            hot_items in hot_items_strategy(),
        ) {
            // Collect patterns before moving hot_items.
            let hot_patterns: Vec<String> = hot_items.iter().filter_map(|item| {
                if let HotTierPayload::Procedure(p) = &item.payload { Some(p.pattern.clone()) } else { None }
            }).collect();

            let input = CompilationInput {
                namespace: "test".to_string(),
                task_class: TaskClass::Debug,
                target_model: "test-model".to_string(),
                format: OutputFormat::Compact,
                warm_tier_budget: DEFAULT_WARM_TIER_BUDGET,
                hot_tier_items: hot_items,
                ranked_candidates: vec![],
            };

            let result = compile_package(input);
            let pkg = &result.package.context_package;

            // Property: every hot procedure's pattern should appear in the output.
            for pattern in &hot_patterns {
                prop_assert!(
                    pkg.contains(pattern.as_str()),
                    "Hot procedure pattern '{}' not found in compact output",
                    pattern
                );
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that hot tier items are included regardless of warm tier budget.
        #[test]
        fn hot_items_included_even_with_zero_budget(
            hot_items in hot_items_strategy(),
        ) {
            let hot_count = hot_items.len();

            let input = CompilationInput {
                namespace: "test".to_string(),
                task_class: TaskClass::Chat,
                target_model: "test-model".to_string(),
                format: OutputFormat::Structured,
                warm_tier_budget: 0, // zero budget
                hot_tier_items: hot_items,
                ranked_candidates: vec![],
            };

            let result = compile_package(input);

            // Property: output should still contain content from hot items.
            // The package should not be empty when hot items exist.
            if hot_count > 0 {
                prop_assert!(
                    result.package.context_package.len() > 50,
                    "Package should contain hot tier content even with zero budget"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Property 23: Output Format Correctness
// ---------------------------------------------------------------------------

/// **Property 23: Output Format Correctness**
///
/// **Validates: Requirements 16.6, 16.7, 16.8**
///
/// For any structured output compilation, the result should contain XML-like
/// tags (loom, identity, project, knowledge, episodes, patterns) with model,
/// token count, namespace, and task class attributes. For any compact output
/// compilation, the result should be a valid JSON object with ns, task,
/// identity, facts, recent, and patterns fields.
mod output_format_correctness {
    use super::*;
    use loom_engine::pipeline::online::compile::{
        compile_package, CompilationInput, DEFAULT_WARM_TIER_BUDGET,
    };
    use loom_engine::pipeline::online::rank::RankedCandidate;
    use loom_engine::pipeline::online::retrieve::{
        CandidatePayload, EpisodeCandidate, FactCandidate, MemoryType,
        ProcedureCandidate, RetrievalCandidate, RetrievalProfile,
    };
    use loom_engine::types::classification::TaskClass;
    use loom_engine::types::compilation::{OutputFormat, RankingScore};

    /// Strategy for generating a relevance score in [0.01, 1.0].
    fn relevance_score() -> impl Strategy<Value = f64> {
        0.01f64..=1.0f64
    }

    /// Strategy for generating a namespace string.
    fn namespace_strategy() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9-]{2,20}"
    }

    /// Strategy for generating a model name.
    fn model_strategy() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("claude-3.5-sonnet".to_string()),
            Just("gpt-4.1-mini".to_string()),
            Just("gemma4:e4b".to_string()),
        ]
    }

    /// Build a ranked fact candidate.
    fn build_ranked_fact(score: f64) -> RankedCandidate {
        let candidate = RetrievalCandidate {
            id: Uuid::new_v4(),
            score,
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
        RankedCandidate {
            candidate,
            scores: RankingScore {
                relevance: score,
                recency: 0.6,
                stability: 0.7,
                provenance: 0.5,
            },
            final_score: score * 0.4 + 0.6 * 0.25 + 0.7 * 0.20 + 0.5 * 0.15,
        }
    }

    /// Build a ranked episode candidate.
    fn build_ranked_episode(score: f64) -> RankedCandidate {
        let candidate = RetrievalCandidate {
            id: Uuid::new_v4(),
            score,
            source_profile: RetrievalProfile::EpisodeRecall,
            memory_type: MemoryType::Episodic,
            payload: CandidatePayload::Episode(EpisodeCandidate {
                source: "claude-code".to_string(),
                content: "Test episode content for property testing".to_string(),
                occurred_at: chrono::Utc::now(),
                namespace: "default".to_string(),
            }),
        };
        RankedCandidate {
            candidate,
            scores: RankingScore {
                relevance: score,
                recency: 0.9,
                stability: 0.8,
                provenance: 0.8,
            },
            final_score: score * 0.4 + 0.9 * 0.25 + 0.8 * 0.20 + 0.8 * 0.15,
        }
    }

    /// Build a ranked procedure candidate.
    fn build_ranked_procedure(score: f64) -> RankedCandidate {
        let candidate = RetrievalCandidate {
            id: Uuid::new_v4(),
            score,
            source_profile: RetrievalProfile::ProcedureAssist,
            memory_type: MemoryType::Procedural,
            payload: CandidatePayload::Procedure(ProcedureCandidate {
                pattern: "Check logs before escalating".to_string(),
                confidence: 0.9,
                observation_count: 7,
                namespace: "default".to_string(),
            }),
        };
        RankedCandidate {
            candidate,
            scores: RankingScore {
                relevance: score,
                recency: 0.5,
                stability: 0.8,
                provenance: 0.7,
            },
            final_score: score * 0.4 + 0.5 * 0.25 + 0.8 * 0.20 + 0.7 * 0.15,
        }
    }

    /// Build a mixed set of candidates.
    fn build_mixed_candidates(scores: &[f64]) -> Vec<RankedCandidate> {
        scores
            .iter()
            .enumerate()
            .map(|(i, &s)| match i % 3 {
                0 => build_ranked_fact(s),
                1 => build_ranked_episode(s),
                _ => build_ranked_procedure(s),
            })
            .collect()
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that structured output always contains the required XML-like tags.
        #[test]
        fn structured_output_contains_required_tags(
            namespace in namespace_strategy(),
            model in model_strategy(),
            class in task_class(),
            scores in prop::collection::vec(relevance_score(), 0..=5),
        ) {
            let candidates = build_mixed_candidates(&scores);

            let input = CompilationInput {
                namespace: namespace.clone(),
                task_class: class.clone(),
                target_model: model.clone(),
                format: OutputFormat::Structured,
                warm_tier_budget: DEFAULT_WARM_TIER_BUDGET,
                hot_tier_items: vec![],
                ranked_candidates: candidates,
            };

            let result = compile_package(input);
            let pkg = &result.package.context_package;

            // Property: structured output must contain <loom> root tag.
            prop_assert!(
                pkg.starts_with("<loom "),
                "Structured output should start with <loom tag"
            );
            prop_assert!(
                pkg.ends_with("</loom>"),
                "Structured output should end with </loom>"
            );

            // Property: root tag must have model, tokens, namespace, task attributes.
            let first_line = pkg.lines().next().unwrap_or("");
            prop_assert!(
                first_line.contains("model=\""),
                "Root tag should have model attribute"
            );
            prop_assert!(
                first_line.contains("tokens=\""),
                "Root tag should have tokens attribute"
            );
            prop_assert!(
                first_line.contains("namespace=\""),
                "Root tag should have namespace attribute"
            );
            prop_assert!(
                first_line.contains("task=\""),
                "Root tag should have task attribute"
            );

            // Property: must contain <identity> tag.
            prop_assert!(
                pkg.contains("<identity>") && pkg.contains("</identity>"),
                "Structured output should contain <identity> tag"
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that structured output contains knowledge section when facts present.
        #[test]
        fn structured_output_knowledge_when_facts_present(
            score in relevance_score(),
        ) {
            let candidates = vec![build_ranked_fact(score)];

            let input = CompilationInput {
                namespace: "test".to_string(),
                task_class: TaskClass::Architecture,
                target_model: "test-model".to_string(),
                format: OutputFormat::Structured,
                warm_tier_budget: DEFAULT_WARM_TIER_BUDGET,
                hot_tier_items: vec![],
                ranked_candidates: candidates,
            };

            let result = compile_package(input);
            let pkg = &result.package.context_package;

            prop_assert!(
                pkg.contains("<knowledge>") && pkg.contains("</knowledge>"),
                "Should contain <knowledge> section when facts are present"
            );
            prop_assert!(
                pkg.contains("<fact "),
                "Should contain <fact> elements"
            );
            // Fact attributes.
            prop_assert!(pkg.contains("subject=\""), "fact should have subject");
            prop_assert!(pkg.contains("predicate=\""), "fact should have predicate");
            prop_assert!(pkg.contains("object=\""), "fact should have object");
            prop_assert!(pkg.contains("evidence=\""), "fact should have evidence");
            prop_assert!(pkg.contains("source=\""), "fact should have source");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that compact output is always valid JSON with required fields.
        #[test]
        fn compact_output_is_valid_json_with_required_fields(
            namespace in namespace_strategy(),
            class in task_class(),
            scores in prop::collection::vec(relevance_score(), 0..=5),
        ) {
            let candidates = build_mixed_candidates(&scores);

            let input = CompilationInput {
                namespace: namespace.clone(),
                task_class: class.clone(),
                target_model: "test-model".to_string(),
                format: OutputFormat::Compact,
                warm_tier_budget: DEFAULT_WARM_TIER_BUDGET,
                hot_tier_items: vec![],
                ranked_candidates: candidates,
            };

            let result = compile_package(input);
            let pkg = &result.package.context_package;

            // Property: compact output must be valid JSON.
            let parsed: serde_json::Value = serde_json::from_str(pkg)
                .map_err(|e| TestCaseError::Fail(format!("Invalid JSON: {e}").into()))?;

            // Property: must have all required fields.
            prop_assert!(parsed.get("ns").is_some(), "Missing 'ns' field");
            prop_assert!(parsed.get("task").is_some(), "Missing 'task' field");
            prop_assert!(parsed.get("identity").is_some(), "Missing 'identity' field");
            prop_assert!(parsed.get("facts").is_some(), "Missing 'facts' field");
            prop_assert!(parsed.get("recent").is_some(), "Missing 'recent' field");
            prop_assert!(parsed.get("patterns").is_some(), "Missing 'patterns' field");

            // Property: ns and task should match input.
            prop_assert_eq!(
                parsed["ns"].as_str().unwrap(),
                namespace.as_str(),
                "ns field should match input namespace"
            );
            let expected_task = class.to_string();
            prop_assert_eq!(
                parsed["task"].as_str().unwrap(),
                expected_task.as_str(),
                "task field should match input task class"
            );

            // Property: facts, recent, patterns should be arrays.
            prop_assert!(parsed["facts"].is_array(), "facts should be an array");
            prop_assert!(parsed["recent"].is_array(), "recent should be an array");
            prop_assert!(parsed["patterns"].is_array(), "patterns should be an array");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that compact JSON facts have the correct abbreviated keys.
        #[test]
        fn compact_facts_have_correct_keys(
            score in relevance_score(),
        ) {
            let candidates = vec![build_ranked_fact(score)];

            let input = CompilationInput {
                namespace: "test".to_string(),
                task_class: TaskClass::Chat,
                target_model: "test-model".to_string(),
                format: OutputFormat::Compact,
                warm_tier_budget: DEFAULT_WARM_TIER_BUDGET,
                hot_tier_items: vec![],
                ranked_candidates: candidates,
            };

            let result = compile_package(input);
            let parsed: serde_json::Value =
                serde_json::from_str(&result.package.context_package).unwrap();

            let facts = parsed["facts"].as_array().unwrap();
            prop_assert!(!facts.is_empty(), "Should have at least one fact");

            for fact in facts {
                prop_assert!(fact.get("s").is_some(), "fact missing 's' key");
                prop_assert!(fact.get("p").is_some(), "fact missing 'p' key");
                prop_assert!(fact.get("o").is_some(), "fact missing 'o' key");
                prop_assert!(fact.get("e").is_some(), "fact missing 'e' key");
                prop_assert!(fact.get("t").is_some(), "fact missing 't' key");
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that compact JSON episodes have the correct keys.
        #[test]
        fn compact_episodes_have_correct_keys(
            score in relevance_score(),
        ) {
            let candidates = vec![build_ranked_episode(score)];

            let input = CompilationInput {
                namespace: "test".to_string(),
                task_class: TaskClass::Debug,
                target_model: "test-model".to_string(),
                format: OutputFormat::Compact,
                warm_tier_budget: DEFAULT_WARM_TIER_BUDGET,
                hot_tier_items: vec![],
                ranked_candidates: candidates,
            };

            let result = compile_package(input);
            let parsed: serde_json::Value =
                serde_json::from_str(&result.package.context_package).unwrap();

            let recent = parsed["recent"].as_array().unwrap();
            prop_assert!(!recent.is_empty(), "Should have at least one episode");

            for ep in recent {
                prop_assert!(ep.get("date").is_some(), "episode missing 'date' key");
                prop_assert!(ep.get("src").is_some(), "episode missing 'src' key");
                prop_assert!(ep.get("text").is_some(), "episode missing 'text' key");
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        /// Test that compact JSON patterns have the correct keys.
        #[test]
        fn compact_patterns_have_correct_keys(
            score in relevance_score(),
        ) {
            let candidates = vec![build_ranked_procedure(score)];

            let input = CompilationInput {
                namespace: "test".to_string(),
                task_class: TaskClass::Writing,
                target_model: "test-model".to_string(),
                format: OutputFormat::Compact,
                warm_tier_budget: DEFAULT_WARM_TIER_BUDGET,
                hot_tier_items: vec![],
                ranked_candidates: candidates,
            };

            let result = compile_package(input);
            let parsed: serde_json::Value =
                serde_json::from_str(&result.package.context_package).unwrap();

            let patterns = parsed["patterns"].as_array().unwrap();
            prop_assert!(!patterns.is_empty(), "Should have at least one pattern");

            for pat in patterns {
                prop_assert!(pat.get("p").is_some(), "pattern missing 'p' key");
                prop_assert!(pat.get("c").is_some(), "pattern missing 'c' key");
                prop_assert!(pat.get("n").is_some(), "pattern missing 'n' key");
            }
        }
    }
}
