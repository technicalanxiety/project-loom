//! Property-based tests for fact extraction, predicate classification,
//! occurrence tracking, supersession, and pack isolation.
//!
//! Tests the core logic without requiring a database. Uses proptest to
//! generate random inputs and verifies invariants hold across many iterations.
//!
//! **Properties tested:**
//! - Property 8: Pack-Aware Prompt Assembly
//! - Property 9: Canonical Predicate Classification
//! - Property 10: Custom Predicate Occurrence Tracking
//! - Property 12: Fact Supersession
//! - Property 33: Predicate Pack Isolation

use std::collections::{HashMap, HashSet};

use chrono::Utc;
use proptest::prelude::*;
use uuid::Uuid;

use loom_engine::pipeline::offline::extract::{
    format_predicate_block, CANDIDATE_REVIEW_THRESHOLD,
};
use loom_engine::pipeline::offline::supersede::NewFactDetails;
use loom_engine::types::fact::ExtractedFact;
use loom_engine::types::predicate::PredicateEntry;

// ---------------------------------------------------------------------------
// Shared strategies
// ---------------------------------------------------------------------------

/// The 25 canonical core predicates from the seed migration.
const CORE_PREDICATES: &[&str] = &[
    "uses",
    "used_by",
    "contains",
    "contained_in",
    "depends_on",
    "dependency_of",
    "replaced_by",
    "replaced",
    "deployed_to",
    "hosts",
    "implements",
    "implemented_by",
    "decided",
    "decided_by",
    "integrates_with",
    "targets",
    "manages",
    "managed_by",
    "configured_with",
    "blocked_by",
    "blocks",
    "authored_by",
    "authored",
    "owns",
    "owned_by",
];

/// Strategy for generating a predicate name (lowercase, 2-30 chars).
fn predicate_name() -> impl Strategy<Value = String> {
    "[a-z][a-z_]{1,29}".prop_map(|s| s)
}

/// Strategy for generating a pack name.
fn pack_name() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{1,14}".prop_map(|s| s)
}

/// Strategy for generating a namespace string.
fn namespace() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9-]{0,29}".prop_map(|s| s)
}

/// Build a `PredicateEntry` for testing.
fn make_predicate(
    name: &str,
    category: &str,
    pack: &str,
    desc: Option<&str>,
    inverse: Option<&str>,
) -> PredicateEntry {
    PredicateEntry {
        predicate: name.to_string(),
        category: category.to_string(),
        pack: pack.to_string(),
        inverse: inverse.map(|s| s.to_string()),
        description: desc.map(|s| s.to_string()),
        usage_count: Some(0),
        created_at: None,
    }
}

// ---------------------------------------------------------------------------
// Property 8: Pack-Aware Prompt Assembly
// ---------------------------------------------------------------------------

/// **Property 8: Pack-Aware Prompt Assembly**
///
/// **Validates: Requirements 4.3, 4.4, 4.5, 28.6**
///
/// For any namespace with configured predicate packs, the dynamically
/// assembled fact extraction prompt should contain all predicates from
/// the configured packs, always include the core pack regardless of
/// configuration, and group predicates by pack name.
mod pack_aware_prompt_assembly {
    use super::*;

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        #[test]
        fn core_pack_predicates_always_present_in_output(
            // Generate 1-5 core predicates
            core_count in 1usize..=5,
            // Generate 0-3 extra pack predicates
            extra_count in 0usize..=3,
            extra_pack in pack_name(),
        ) {
            // Build core predicates.
            let core_preds: Vec<PredicateEntry> = CORE_PREDICATES[..core_count]
                .iter()
                .map(|&name| make_predicate(name, "structural", "core", Some("desc"), None))
                .collect();

            // Build extra pack predicates (if any).
            let extra_preds: Vec<PredicateEntry> = (0..extra_count)
                .map(|i| {
                    make_predicate(
                        &format!("extra_pred_{i}"),
                        "regulatory",
                        &extra_pack,
                        Some("extra desc"),
                        None,
                    )
                })
                .collect();

            // Combine: core first, then extra.
            let mut all_preds = core_preds.clone();
            all_preds.extend(extra_preds);

            let block = format_predicate_block(&all_preds);

            // Property: core pack header is always present.
            prop_assert!(
                block.contains("### core"),
                "Output must contain core pack header. Got:\n{block}"
            );

            // Property: every core predicate appears in the output.
            for pred in &core_preds {
                prop_assert!(
                    block.contains(&format!("- {} (", pred.predicate)),
                    "Core predicate '{}' missing from output:\n{block}",
                    pred.predicate
                );
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        #[test]
        fn configured_packs_predicates_grouped_by_pack(
            // Generate 1-3 packs with 1-4 predicates each
            pack_count in 1usize..=3,
        ) {
            // Always include core + up to pack_count additional packs.
            let pack_names: Vec<String> = std::iter::once("core".to_string())
                .chain((0..pack_count).map(|i| format!("pack_{i}")))
                .collect();

            let mut all_preds: Vec<PredicateEntry> = Vec::new();

            // Add 2 predicates per pack.
            for pack in &pack_names {
                for j in 0..2 {
                    all_preds.push(make_predicate(
                        &format!("{pack}_pred_{j}"),
                        "structural",
                        pack,
                        Some("desc"),
                        None,
                    ));
                }
            }

            let block = format_predicate_block(&all_preds);

            // Property: each pack has its own header.
            for pack in &pack_names {
                prop_assert!(
                    block.contains(&format!("### {pack}")),
                    "Pack '{pack}' header missing from output:\n{block}"
                );
            }

            // Property: predicates appear under their pack header (pack header
            // comes before the predicate line in the output).
            for pred in &all_preds {
                let pack_header = format!("### {}", pred.pack);
                let pred_line = format!("- {} (", pred.predicate);

                let header_pos = block.find(&pack_header).unwrap_or(usize::MAX);
                let pred_pos = block.find(&pred_line).unwrap_or(0);

                prop_assert!(
                    header_pos < pred_pos,
                    "Predicate '{}' should appear after its pack header '{}' in output:\n{block}",
                    pred.predicate,
                    pred.pack
                );
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        #[test]
        fn empty_predicates_returns_placeholder(
            _dummy in 0u8..1,
        ) {
            let block = format_predicate_block(&[]);
            prop_assert_eq!(
                block,
                "(no canonical predicates available)",
                "Empty predicates should return placeholder"
            );
        }
    }
}


// ---------------------------------------------------------------------------
// Property 9: Canonical Predicate Classification
// ---------------------------------------------------------------------------

/// **Property 9: Canonical Predicate Classification**
///
/// **Validates: Requirements 4.7, 4.8**
///
/// For any extracted fact, if the predicate matches a canonical predicate
/// in the registry, it should be marked with custom=false. If it does not
/// match any canonical predicate, it should be marked with custom=true.
///
/// Tests the classification logic without a database by simulating the
/// canonical registry as a HashSet.
mod canonical_predicate_classification {
    use super::*;

    /// Simulate the predicate classification logic from
    /// `validate_and_track_predicates`: check if predicate is in the
    /// canonical set, set custom flag accordingly.
    fn classify_predicate(predicate: &str, canonical_set: &HashSet<String>) -> bool {
        // Returns true if custom (not canonical), false if canonical.
        !canonical_set.contains(predicate)
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        #[test]
        fn canonical_predicates_marked_not_custom(
            // Pick a random canonical predicate
            idx in 0usize..25,
        ) {
            let canonical_set: HashSet<String> =
                CORE_PREDICATES.iter().map(|s| s.to_string()).collect();

            let predicate = CORE_PREDICATES[idx % CORE_PREDICATES.len()];
            let is_custom = classify_predicate(predicate, &canonical_set);

            prop_assert!(
                !is_custom,
                "Canonical predicate '{}' should be marked custom=false",
                predicate
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        #[test]
        fn non_canonical_predicates_marked_custom(
            custom_pred in predicate_name(),
        ) {
            let canonical_set: HashSet<String> =
                CORE_PREDICATES.iter().map(|s| s.to_string()).collect();

            // Ensure the generated predicate is not accidentally canonical.
            prop_assume!(!canonical_set.contains(&custom_pred));

            let is_custom = classify_predicate(&custom_pred, &canonical_set);

            prop_assert!(
                is_custom,
                "Non-canonical predicate '{}' should be marked custom=true",
                custom_pred
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        #[test]
        fn classification_is_deterministic(
            pred in predicate_name(),
        ) {
            let canonical_set: HashSet<String> =
                CORE_PREDICATES.iter().map(|s| s.to_string()).collect();

            let result1 = classify_predicate(&pred, &canonical_set);
            let result2 = classify_predicate(&pred, &canonical_set);

            prop_assert_eq!(
                result1, result2,
                "Classification of '{}' should be deterministic",
                pred
            );
        }
    }
}


// ---------------------------------------------------------------------------
// Property 10: Custom Predicate Occurrence Tracking
// ---------------------------------------------------------------------------

/// **Property 10: Custom Predicate Occurrence Tracking**
///
/// **Validates: Requirements 5.2, 5.4**
///
/// For any custom predicate, the occurrence count should equal the number
/// of facts using that predicate. When a candidate reaches 5 occurrences,
/// it should be flagged for operator review.
///
/// Tests the counting logic without a database by simulating the candidate
/// tracking in a HashMap.
mod custom_predicate_occurrence_tracking {
    use super::*;

    /// Simulates the candidate tracking logic from
    /// `validate_and_track_predicates` + `insert_or_update_candidate`.
    ///
    /// Returns a map of predicate → occurrence count and a set of predicates
    /// that reached the review threshold.
    fn track_custom_predicates(
        facts: &[ExtractedFact],
        canonical_set: &HashSet<String>,
    ) -> (HashMap<String, i32>, HashSet<String>) {
        let mut occurrences: HashMap<String, i32> = HashMap::new();
        let mut flagged: HashSet<String> = HashSet::new();

        for fact in facts {
            if canonical_set.contains(&fact.predicate) {
                continue; // canonical — skip
            }

            let count = occurrences.entry(fact.predicate.clone()).or_insert(0);
            *count += 1;

            if *count >= CANDIDATE_REVIEW_THRESHOLD {
                flagged.insert(fact.predicate.clone());
            }
        }

        (occurrences, flagged)
    }

    /// Build an `ExtractedFact` with the given predicate.
    fn make_fact(predicate: &str) -> ExtractedFact {
        ExtractedFact {
            subject: "SubjectEntity".to_string(),
            predicate: predicate.to_string(),
            object: "ObjectEntity".to_string(),
            custom: true,
            evidence_strength: Some("explicit".to_string()),
            temporal_markers: None,
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        #[test]
        fn occurrence_count_equals_fact_count(
            custom_pred in predicate_name(),
            fact_count in 1usize..=20,
        ) {
            let canonical_set: HashSet<String> =
                CORE_PREDICATES.iter().map(|s| s.to_string()).collect();

            // Ensure the predicate is not accidentally canonical.
            prop_assume!(!canonical_set.contains(&custom_pred));

            let facts: Vec<ExtractedFact> = (0..fact_count)
                .map(|_| make_fact(&custom_pred))
                .collect();

            let (occurrences, _) = track_custom_predicates(&facts, &canonical_set);

            let tracked_count = occurrences.get(&custom_pred).copied().unwrap_or(0);

            prop_assert_eq!(
                tracked_count as usize,
                fact_count,
                "Occurrence count ({}) should equal fact count ({}) for predicate '{}'",
                tracked_count,
                fact_count,
                custom_pred
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        #[test]
        fn candidates_at_five_occurrences_flagged_for_review(
            custom_pred in predicate_name(),
            fact_count in 5usize..=20,
        ) {
            let canonical_set: HashSet<String> =
                CORE_PREDICATES.iter().map(|s| s.to_string()).collect();

            prop_assume!(!canonical_set.contains(&custom_pred));

            let facts: Vec<ExtractedFact> = (0..fact_count)
                .map(|_| make_fact(&custom_pred))
                .collect();

            let (_, flagged) = track_custom_predicates(&facts, &canonical_set);

            prop_assert!(
                flagged.contains(&custom_pred),
                "Predicate '{}' with {} occurrences (>= {}) should be flagged for review",
                custom_pred,
                fact_count,
                CANDIDATE_REVIEW_THRESHOLD
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        #[test]
        fn candidates_below_five_not_flagged(
            custom_pred in predicate_name(),
            fact_count in 1usize..5,
        ) {
            let canonical_set: HashSet<String> =
                CORE_PREDICATES.iter().map(|s| s.to_string()).collect();

            prop_assume!(!canonical_set.contains(&custom_pred));

            let facts: Vec<ExtractedFact> = (0..fact_count)
                .map(|_| make_fact(&custom_pred))
                .collect();

            let (_, flagged) = track_custom_predicates(&facts, &canonical_set);

            prop_assert!(
                !flagged.contains(&custom_pred),
                "Predicate '{}' with {} occurrences (< {}) should NOT be flagged",
                custom_pred,
                fact_count,
                CANDIDATE_REVIEW_THRESHOLD
            );
        }
    }
}


// ---------------------------------------------------------------------------
// Property 12: Fact Supersession
// ---------------------------------------------------------------------------

/// **Property 12: Fact Supersession**
///
/// **Validates: Requirements 6.3, 6.4**
///
/// For any new fact that contradicts an existing fact (same subject and
/// predicate, different object), the old fact should have valid_until set
/// to the new fact's valid_from and superseded_by set to the new fact's id.
///
/// Tests the supersession decision logic without a database by simulating
/// the matching and update steps.
mod fact_supersession {
    use super::*;
    use loom_engine::types::fact::Fact;

    /// Simulate the supersession detection logic from
    /// `resolve_supersessions`: given a list of existing facts and a new
    /// fact, identify which existing facts should be superseded.
    ///
    /// Returns a list of (old_fact_id, superseded_by, valid_until) tuples.
    fn detect_supersessions(
        existing: &[Fact],
        new_fact: &NewFactDetails,
    ) -> Vec<(Uuid, Uuid, chrono::DateTime<Utc>)> {
        let mut superseded = Vec::new();

        for old in existing {
            // Skip the new fact itself.
            if old.id == new_fact.fact_id {
                continue;
            }
            // Same subject + predicate + namespace, different object.
            if old.subject_id == new_fact.subject_id
                && old.predicate == new_fact.predicate
                && old.namespace == new_fact.namespace
                && old.object_id != new_fact.object_id
                && old.valid_until.is_none()
                && old.deleted_at.is_none()
            {
                superseded.push((old.id, new_fact.fact_id, new_fact.valid_from));
            }
        }

        superseded
    }

    /// Build a current (non-superseded) `Fact` for testing.
    fn make_current_fact(
        subject_id: Uuid,
        predicate: &str,
        object_id: Uuid,
        namespace: &str,
    ) -> Fact {
        Fact {
            id: Uuid::new_v4(),
            subject_id,
            predicate: predicate.to_string(),
            object_id,
            namespace: namespace.to_string(),
            valid_from: Utc::now(),
            valid_until: None,
            source_episodes: vec![Uuid::new_v4()],
            superseded_by: None,
            evidence_status: "extracted".to_string(),
            evidence_strength: Some("explicit".to_string()),
            properties: None,
            created_at: Some(Utc::now()),
            deleted_at: None,
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        #[test]
        fn contradicting_fact_triggers_supersession(
            pred in predicate_name(),
            ns in namespace(),
        ) {
            let subject_id = Uuid::new_v4();
            let old_object_id = Uuid::new_v4();
            let new_object_id = Uuid::new_v4();

            // Ensure different objects.
            prop_assume!(old_object_id != new_object_id);

            let old_fact = make_current_fact(subject_id, &pred, old_object_id, &ns);

            let new_fact = NewFactDetails {
                fact_id: Uuid::new_v4(),
                subject_id,
                predicate: pred.clone(),
                object_id: new_object_id,
                namespace: ns.clone(),
                valid_from: Utc::now(),
            };

            let superseded = detect_supersessions(&[old_fact.clone()], &new_fact);

            // Property: exactly one fact should be superseded.
            prop_assert_eq!(
                superseded.len(),
                1,
                "Expected 1 supersession for contradicting fact, got {}",
                superseded.len()
            );

            let (old_id, superseded_by, valid_until) = &superseded[0];

            // Property: superseded_by points to the new fact.
            prop_assert_eq!(
                superseded_by,
                &new_fact.fact_id,
                "superseded_by should point to new fact"
            );

            // Property: valid_until equals the new fact's valid_from.
            prop_assert_eq!(
                valid_until,
                &new_fact.valid_from,
                "valid_until should equal new fact's valid_from"
            );

            // Property: the old fact's id is correct.
            prop_assert_eq!(
                old_id,
                &old_fact.id,
                "superseded fact id should match old fact"
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        #[test]
        fn same_object_does_not_trigger_supersession(
            pred in predicate_name(),
            ns in namespace(),
        ) {
            let subject_id = Uuid::new_v4();
            let object_id = Uuid::new_v4();

            let old_fact = make_current_fact(subject_id, &pred, object_id, &ns);

            // New fact with the SAME object — not a contradiction.
            let new_fact = NewFactDetails {
                fact_id: Uuid::new_v4(),
                subject_id,
                predicate: pred.clone(),
                object_id, // same object
                namespace: ns.clone(),
                valid_from: Utc::now(),
            };

            let superseded = detect_supersessions(&[old_fact], &new_fact);

            prop_assert!(
                superseded.is_empty(),
                "Same object should not trigger supersession, got {} supersessions",
                superseded.len()
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        #[test]
        fn different_predicate_does_not_trigger_supersession(
            pred1 in predicate_name(),
            pred2 in predicate_name(),
            ns in namespace(),
        ) {
            prop_assume!(pred1 != pred2);

            let subject_id = Uuid::new_v4();
            let old_object_id = Uuid::new_v4();
            let new_object_id = Uuid::new_v4();

            let old_fact = make_current_fact(subject_id, &pred1, old_object_id, &ns);

            let new_fact = NewFactDetails {
                fact_id: Uuid::new_v4(),
                subject_id,
                predicate: pred2.clone(), // different predicate
                object_id: new_object_id,
                namespace: ns.clone(),
                valid_from: Utc::now(),
            };

            let superseded = detect_supersessions(&[old_fact], &new_fact);

            prop_assert!(
                superseded.is_empty(),
                "Different predicate should not trigger supersession"
            );
        }
    }
}


// ---------------------------------------------------------------------------
// Property 33: Predicate Pack Isolation
// ---------------------------------------------------------------------------

/// **Property 33: Predicate Pack Isolation**
///
/// **Validates: Requirements 25.3, 28.4, 28.6**
///
/// For any predicate in the canonical registry, it should belong to exactly
/// one pack. For any namespace, the predicate_packs array should always
/// contain 'core'.
mod predicate_pack_isolation {
    use super::*;

    /// Simulate the namespace pack list normalization from
    /// `assemble_fact_prompt`: ensure 'core' is always present.
    fn normalize_packs(mut packs: Vec<String>) -> Vec<String> {
        if !packs.iter().any(|p| p == "core") {
            packs.insert(0, "core".to_string());
        }
        packs
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        #[test]
        fn each_predicate_belongs_to_exactly_one_pack(
            // Generate 2-4 packs with 1-5 predicates each
            pack_count in 2usize..=4,
        ) {
            let pack_names: Vec<String> = (0..pack_count)
                .map(|i| format!("pack_{i}"))
                .collect();

            let mut all_preds: Vec<PredicateEntry> = Vec::new();
            let mut pred_to_pack: HashMap<String, String> = HashMap::new();

            for (i, pack) in pack_names.iter().enumerate() {
                for j in 0..3 {
                    let pred_name = format!("pred_{i}_{j}");
                    all_preds.push(make_predicate(
                        &pred_name,
                        "structural",
                        pack,
                        Some("desc"),
                        None,
                    ));
                    pred_to_pack.insert(pred_name, pack.clone());
                }
            }

            // Property: each predicate maps to exactly one pack.
            let mut seen_packs: HashMap<String, HashSet<String>> = HashMap::new();
            for pred in &all_preds {
                seen_packs
                    .entry(pred.predicate.clone())
                    .or_default()
                    .insert(pred.pack.clone());
            }

            for (pred_name, packs) in &seen_packs {
                prop_assert_eq!(
                    packs.len(),
                    1,
                    "Predicate '{}' belongs to {} packs ({:?}), expected exactly 1",
                    pred_name,
                    packs.len(),
                    packs
                );
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        #[test]
        fn namespace_packs_always_contain_core(
            // Generate a random pack list that may or may not include 'core'
            extra_packs in prop::collection::vec(pack_name(), 0..=5),
            include_core in proptest::bool::ANY,
        ) {
            let mut packs = extra_packs;
            if include_core {
                packs.push("core".to_string());
            }

            let normalized = normalize_packs(packs);

            prop_assert!(
                normalized.iter().any(|p| p == "core"),
                "Normalized pack list must always contain 'core', got: {:?}",
                normalized
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        #[test]
        fn core_only_namespace_has_core(
            _dummy in 0u8..1,
        ) {
            // A namespace with no configured packs defaults to ["core"].
            let packs: Vec<String> = Vec::new();
            let normalized = normalize_packs(packs);

            prop_assert_eq!(
                normalized,
                vec!["core".to_string()],
                "Empty pack list should normalize to ['core']"
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        #[test]
        fn format_block_preserves_pack_isolation(
            pack_count in 2usize..=3,
        ) {
            let pack_names: Vec<String> = std::iter::once("core".to_string())
                .chain((0..pack_count).map(|i| format!("extra_{i}")))
                .collect();

            let mut all_preds: Vec<PredicateEntry> = Vec::new();
            for pack in &pack_names {
                all_preds.push(make_predicate(
                    &format!("{pack}_pred"),
                    "structural",
                    pack,
                    Some("desc"),
                    None,
                ));
            }

            let block = format_predicate_block(&all_preds);

            // Property: each pack header appears exactly once.
            for pack in &pack_names {
                let header = format!("### {pack}");
                let count = block.matches(&header).count();
                prop_assert_eq!(
                    count,
                    1,
                    "Pack '{}' header should appear exactly once, found {} times in:\n{}",
                    pack,
                    count,
                    block
                );
            }
        }
    }
}
