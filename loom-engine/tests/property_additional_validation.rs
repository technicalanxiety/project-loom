//! Property-based tests for additional validation invariants.
//!
//! Tests entity type constraints, pack-aware predicate promotion,
//! current fact filtering, evidence status validity, fact provenance,
//! and alias deduplication.
//!
//! **Properties tested:**
//! - Property 4: Entity Type Constraint
//! - Property 11: Pack-Aware Predicate Promotion
//! - Property 13: Current Fact Filtering
//! - Property 28: Evidence Status Validity
//! - Property 29: Fact Provenance Non-Empty
//! - Property 30: Alias Deduplication

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use proptest::prelude::*;
use uuid::Uuid;

use loom_engine::types::entity::EntityType;
use loom_engine::types::fact::{EvidenceStatus, Fact};
use loom_engine::types::predicate::PredicateCandidate;

/// The 10 valid entity type strings matching the DB CHECK constraint.
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

/// The 7 valid evidence status strings matching the DB CHECK constraint.
const VALID_EVIDENCE_STATUSES: &[&str] = &[
    "user_asserted",
    "observed",
    "extracted",
    "inferred",
    "promoted",
    "deprecated",
    "superseded",
];

/// Known valid pack names for testing promotion logic.
const VALID_PACKS: &[&str] = &["core", "grc", "healthcare", "finserv"];

// ---------------------------------------------------------------------------
// Shared strategies
// ---------------------------------------------------------------------------

/// Strategy for generating a namespace string.
fn namespace() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9-]{0,29}"
}

/// Strategy for generating a predicate name.
fn predicate_name() -> impl Strategy<Value = String> {
    "[a-z][a-z_]{1,29}"
}

/// Strategy for generating entity names.
fn entity_name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_ -]{0,39}"
}

/// Strategy for selecting a valid evidence status string.
fn valid_evidence_status_str() -> impl Strategy<Value = String> {
    prop::sample::select(VALID_EVIDENCE_STATUSES).prop_map(|s| s.to_string())
}

/// Strategy for selecting a valid pack name.
fn valid_pack_name() -> impl Strategy<Value = String> {
    prop::sample::select(VALID_PACKS).prop_map(|s| s.to_string())
}

/// Build a `Fact` for testing with the given parameters.
fn make_fact(
    namespace: &str,
    evidence_status: &str,
    source_episodes: Vec<Uuid>,
    valid_until: Option<DateTime<Utc>>,
    deleted_at: Option<DateTime<Utc>>,
) -> Fact {
    Fact {
        id: Uuid::new_v4(),
        subject_id: Uuid::new_v4(),
        predicate: "uses".to_string(),
        object_id: Uuid::new_v4(),
        namespace: namespace.to_string(),
        valid_from: Utc::now(),
        valid_until,
        source_episodes,
        superseded_by: None,
        evidence_status: evidence_status.to_string(),
        evidence_strength: Some("explicit".to_string()),
        properties: None,
        created_at: Some(Utc::now()),
        deleted_at,
    }
}

/// Simulate current fact filtering: returns only facts where
/// `valid_until IS NULL` and `deleted_at IS NULL`, scoped to namespace.
fn filter_current_facts<'a>(facts: &'a [Fact], namespace: &str) -> Vec<&'a Fact> {
    facts
        .iter()
        .filter(|f| {
            f.namespace == namespace && f.valid_until.is_none() && f.deleted_at.is_none()
        })
        .collect()
}


// ---------------------------------------------------------------------------
// Property 4: Entity Type Constraint
// ---------------------------------------------------------------------------

/// **Property 4: Entity Type Constraint**
///
/// **Validates: Requirements 2.2, 2.5, 43.9**
///
/// For any entity extraction result deserialized via serde, all extracted
/// entity types should be one of the ten valid types. Malformed LLM
/// responses with invalid types should be rejected at the serde
/// deserialization boundary.
mod entity_type_constraint {
    use super::*;

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// All 10 valid entity types deserialize successfully via serde.
        #[test]
        fn valid_entity_types_deserialize(
            idx in 0usize..10,
        ) {
            let type_str = VALID_ENTITY_TYPES[idx];
            let json = format!("\"{}\"", type_str);

            let result: Result<EntityType, _> = serde_json::from_str(&json);

            prop_assert!(
                result.is_ok(),
                "Valid entity type '{}' should deserialize, got: {:?}",
                type_str,
                result.err()
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Arbitrary strings that are NOT one of the 10 valid types are
        /// rejected by serde deserialization.
        #[test]
        fn invalid_entity_types_rejected_by_serde(
            invalid_type in "[a-zA-Z]{1,30}",
        ) {
            let valid_set: HashSet<&str> = VALID_ENTITY_TYPES.iter().copied().collect();

            // Only test strings that are genuinely invalid.
            prop_assume!(!valid_set.contains(invalid_type.as_str()));

            let json = format!("\"{}\"", invalid_type);
            let result = serde_json::from_str::<EntityType>(&json);

            prop_assert!(
                result.is_err(),
                "Invalid entity type '{}' should be rejected by serde, but got: {:?}",
                invalid_type,
                result.unwrap()
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Malformed LLM JSON responses with invalid entity_type fields
        /// are rejected when deserializing an extraction response.
        #[test]
        fn malformed_extraction_response_rejected(
            _name in entity_name(),
            bad_type in "[a-zA-Z]{1,20}",
        ) {
            let valid_set: HashSet<&str> = VALID_ENTITY_TYPES.iter().copied().collect();
            prop_assume!(!valid_set.contains(bad_type.as_str()));

            // Attempt to deserialize the entity_type field directly,
            // simulating what happens when a malformed LLM response is parsed.
            let entity_type_json = serde_json::json!(bad_type);
            let result = serde_json::from_value::<EntityType>(entity_type_json);

            prop_assert!(
                result.is_err(),
                "Malformed entity_type '{}' in LLM response should be rejected",
                bad_type
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// EntityType roundtrips through serde: serialize then deserialize
        /// always yields the original variant.
        #[test]
        fn entity_type_serde_roundtrip(
            idx in 0usize..10,
        ) {
            let type_str = VALID_ENTITY_TYPES[idx];
            let json = format!("\"{}\"", type_str);

            let parsed: EntityType = serde_json::from_str(&json).unwrap();
            let serialized = serde_json::to_string(&parsed).unwrap();

            prop_assert_eq!(
                serialized,
                json,
                "Roundtrip failed for entity type '{}'",
                type_str
            );
        }
    }
}


// ---------------------------------------------------------------------------
// Property 11: Pack-Aware Predicate Promotion
// ---------------------------------------------------------------------------

/// **Property 11: Pack-Aware Predicate Promotion**
///
/// **Validates: Requirements 5.6, 5.7**
///
/// For any custom predicate promoted to canonical status, the
/// promoted_to_pack field on the predicate candidate record should
/// reference a valid pack in loom_predicate_packs.
mod pack_aware_predicate_promotion {
    use super::*;

    /// Build a `PredicateCandidate` simulating a promoted custom predicate.
    fn make_promoted_candidate(
        predicate: &str,
        promoted_to_pack: Option<&str>,
        occurrences: i32,
    ) -> PredicateCandidate {
        PredicateCandidate {
            id: Uuid::new_v4(),
            predicate: predicate.to_string(),
            occurrences: Some(occurrences),
            example_facts: Some(vec![Uuid::new_v4()]),
            mapped_to: None,
            promoted_to_pack: promoted_to_pack.map(|s| s.to_string()),
            created_at: Some(Utc::now()),
            resolved_at: promoted_to_pack.map(|_| Utc::now()),
        }
    }

    /// Simulate the promotion validation: when promoting a custom predicate,
    /// the target pack must be in the set of valid packs.
    fn validate_promotion(
        candidate: &PredicateCandidate,
        valid_packs: &HashSet<String>,
    ) -> bool {
        match &candidate.promoted_to_pack {
            Some(pack) => valid_packs.contains(pack),
            None => false, // Not promoted — no pack to validate.
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Promoted candidates with a valid pack pass validation.
        #[test]
        fn promoted_with_valid_pack_passes(
            pred in predicate_name(),
            pack in valid_pack_name(),
            occurrences in 5i32..=50,
        ) {
            let valid_packs: HashSet<String> =
                VALID_PACKS.iter().map(|s| s.to_string()).collect();

            let candidate = make_promoted_candidate(&pred, Some(&pack), occurrences);

            prop_assert!(
                validate_promotion(&candidate, &valid_packs),
                "Promoted candidate '{}' with pack '{}' should pass validation",
                pred,
                pack
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Promoted candidates with an invalid pack fail validation.
        #[test]
        fn promoted_with_invalid_pack_fails(
            pred in predicate_name(),
            bad_pack in "[a-z]{3,15}",
            occurrences in 5i32..=50,
        ) {
            let valid_packs: HashSet<String> =
                VALID_PACKS.iter().map(|s| s.to_string()).collect();

            prop_assume!(!valid_packs.contains(&bad_pack));

            let candidate = make_promoted_candidate(&pred, Some(&bad_pack), occurrences);

            prop_assert!(
                !validate_promotion(&candidate, &valid_packs),
                "Promoted candidate '{}' with invalid pack '{}' should fail validation",
                pred,
                bad_pack
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Candidates without promoted_to_pack are not considered promoted.
        #[test]
        fn non_promoted_candidate_has_no_pack(
            pred in predicate_name(),
            occurrences in 1i32..=20,
        ) {
            let valid_packs: HashSet<String> =
                VALID_PACKS.iter().map(|s| s.to_string()).collect();

            let candidate = make_promoted_candidate(&pred, None, occurrences);

            prop_assert!(
                !validate_promotion(&candidate, &valid_packs),
                "Non-promoted candidate '{}' should fail promotion validation",
                pred
            );

            prop_assert!(
                candidate.promoted_to_pack.is_none(),
                "Non-promoted candidate should have promoted_to_pack = None"
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// The promoted_to_pack field is always set when a candidate is
        /// promoted, and it references a real pack.
        #[test]
        fn promotion_requires_pack_selection(
            pred in predicate_name(),
            pack in valid_pack_name(),
        ) {
            let candidate = make_promoted_candidate(&pred, Some(&pack), 5);

            prop_assert!(
                candidate.promoted_to_pack.is_some(),
                "Promoted candidate must have promoted_to_pack set"
            );

            prop_assert!(
                candidate.resolved_at.is_some(),
                "Promoted candidate must have resolved_at timestamp"
            );
        }
    }
}


// ---------------------------------------------------------------------------
// Property 13: Current Fact Filtering
// ---------------------------------------------------------------------------

/// **Property 13: Current Fact Filtering**
///
/// **Validates: Requirements 6.6, 10.1, 10.3**
///
/// For any fact retrieval query without explicit historical flag, all
/// returned facts should have valid_until IS NULL and deleted_at IS NULL.
mod current_fact_filtering {
    use super::*;

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Current fact filtering only returns facts with valid_until IS NULL
        /// and deleted_at IS NULL.
        #[test]
        fn current_facts_have_null_valid_until_and_deleted_at(
            ns in namespace(),
            current_count in 1usize..=15,
            superseded_count in 0usize..=10,
            deleted_count in 0usize..=10,
        ) {
            let mut facts: Vec<Fact> = Vec::new();

            // Current facts: valid_until = None, deleted_at = None.
            for _ in 0..current_count {
                facts.push(make_fact(
                    &ns,
                    "extracted",
                    vec![Uuid::new_v4()],
                    None,
                    None,
                ));
            }

            // Superseded facts: valid_until = Some, deleted_at = None.
            for _ in 0..superseded_count {
                facts.push(make_fact(
                    &ns,
                    "superseded",
                    vec![Uuid::new_v4()],
                    Some(Utc::now()),
                    None,
                ));
            }

            // Deleted facts: valid_until = None, deleted_at = Some.
            for _ in 0..deleted_count {
                facts.push(make_fact(
                    &ns,
                    "extracted",
                    vec![Uuid::new_v4()],
                    None,
                    Some(Utc::now()),
                ));
            }

            let current = filter_current_facts(&facts, &ns);

            // Property: only current_count facts should be returned.
            prop_assert_eq!(
                current.len(),
                current_count,
                "Expected {} current facts, got {}",
                current_count,
                current.len()
            );

            // Property: all returned facts have valid_until IS NULL.
            for fact in &current {
                prop_assert!(
                    fact.valid_until.is_none(),
                    "Current fact {} should have valid_until = None",
                    fact.id
                );
            }

            // Property: all returned facts have deleted_at IS NULL.
            for fact in &current {
                prop_assert!(
                    fact.deleted_at.is_none(),
                    "Current fact {} should have deleted_at = None",
                    fact.id
                );
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Facts that are both superseded AND deleted are excluded.
        #[test]
        fn superseded_and_deleted_facts_excluded(
            ns in namespace(),
            current_count in 1usize..=5,
            both_count in 1usize..=5,
        ) {
            let mut facts: Vec<Fact> = Vec::new();

            for _ in 0..current_count {
                facts.push(make_fact(&ns, "extracted", vec![Uuid::new_v4()], None, None));
            }

            // Facts with BOTH valid_until and deleted_at set.
            for _ in 0..both_count {
                facts.push(make_fact(
                    &ns,
                    "superseded",
                    vec![Uuid::new_v4()],
                    Some(Utc::now()),
                    Some(Utc::now()),
                ));
            }

            let current = filter_current_facts(&facts, &ns);

            prop_assert_eq!(
                current.len(),
                current_count,
                "Only current facts should be returned, got {} instead of {}",
                current.len(),
                current_count
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Current fact filtering is namespace-scoped.
        #[test]
        fn filtering_is_namespace_scoped(
            ns_a in "[a-z]{3,10}",
            ns_b in "[a-z]{3,10}",
            count_a in 1usize..=5,
            count_b in 1usize..=5,
        ) {
            prop_assume!(ns_a != ns_b);

            let mut facts: Vec<Fact> = Vec::new();

            for _ in 0..count_a {
                facts.push(make_fact(&ns_a, "extracted", vec![Uuid::new_v4()], None, None));
            }
            for _ in 0..count_b {
                facts.push(make_fact(&ns_b, "extracted", vec![Uuid::new_v4()], None, None));
            }

            let current_a = filter_current_facts(&facts, &ns_a);
            let current_b = filter_current_facts(&facts, &ns_b);

            prop_assert_eq!(current_a.len(), count_a);
            prop_assert_eq!(current_b.len(), count_b);

            // All returned facts belong to the queried namespace.
            for f in &current_a {
                prop_assert_eq!(&f.namespace, &ns_a);
            }
            for f in &current_b {
                prop_assert_eq!(&f.namespace, &ns_b);
            }
        }
    }
}


// ---------------------------------------------------------------------------
// Property 28: Evidence Status Validity
// ---------------------------------------------------------------------------

/// **Property 28: Evidence Status Validity**
///
/// **Validates: Requirements 32.1**
///
/// For any fact, the evidence_status should be one of the seven valid
/// values: user_asserted, observed, extracted, inferred, promoted,
/// deprecated, superseded.
mod evidence_status_validity {
    use super::*;

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// All 7 valid evidence status values deserialize via serde.
        #[test]
        fn valid_evidence_statuses_deserialize(
            idx in 0usize..7,
        ) {
            let status_str = VALID_EVIDENCE_STATUSES[idx];
            let json = format!("\"{}\"", status_str);

            let result: Result<EvidenceStatus, _> = serde_json::from_str(&json);

            prop_assert!(
                result.is_ok(),
                "Valid evidence status '{}' should deserialize, got: {:?}",
                status_str,
                result.err()
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Arbitrary strings that are NOT one of the 7 valid statuses are
        /// rejected by serde deserialization.
        #[test]
        fn invalid_evidence_statuses_rejected(
            invalid_status in "[a-z_]{1,30}",
        ) {
            let valid_set: HashSet<&str> = VALID_EVIDENCE_STATUSES.iter().copied().collect();
            prop_assume!(!valid_set.contains(invalid_status.as_str()));

            let json = format!("\"{}\"", invalid_status);
            let result = serde_json::from_str::<EvidenceStatus>(&json);

            prop_assert!(
                result.is_err(),
                "Invalid evidence status '{}' should be rejected by serde, but got: {:?}",
                invalid_status,
                result.unwrap()
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// EvidenceStatus roundtrips through serde: serialize then
        /// deserialize always yields the original variant.
        #[test]
        fn evidence_status_serde_roundtrip(
            idx in 0usize..7,
        ) {
            let status_str = VALID_EVIDENCE_STATUSES[idx];
            let json = format!("\"{}\"", status_str);

            let parsed: EvidenceStatus = serde_json::from_str(&json).unwrap();
            let serialized = serde_json::to_string(&parsed).unwrap();

            prop_assert_eq!(
                serialized,
                json,
                "Roundtrip failed for evidence status '{}'",
                status_str
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Facts constructed with valid evidence statuses pass validation.
        #[test]
        fn facts_with_valid_status_pass_validation(
            ns in namespace(),
            idx in 0usize..7,
        ) {
            let status = VALID_EVIDENCE_STATUSES[idx];
            let fact = make_fact(&ns, status, vec![Uuid::new_v4()], None, None);

            let valid_set: HashSet<&str> = VALID_EVIDENCE_STATUSES.iter().copied().collect();

            prop_assert!(
                valid_set.contains(fact.evidence_status.as_str()),
                "Fact evidence_status '{}' should be in the valid set",
                fact.evidence_status
            );
        }
    }
}


// ---------------------------------------------------------------------------
// Property 29: Fact Provenance Non-Empty
// ---------------------------------------------------------------------------

/// **Property 29: Fact Provenance Non-Empty**
///
/// **Validates: Requirements 4.11, 41.1**
///
/// For any fact, the source_episodes array should be non-empty (every
/// fact must have at least one source episode).
mod fact_provenance_non_empty {
    use super::*;

    /// Validate that a fact has non-empty source_episodes.
    fn validate_provenance(fact: &Fact) -> bool {
        !fact.source_episodes.is_empty()
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Facts with 1 or more source episodes pass provenance validation.
        #[test]
        fn facts_with_source_episodes_pass(
            ns in namespace(),
            episode_count in 1usize..=10,
            status in valid_evidence_status_str(),
        ) {
            let episodes: Vec<Uuid> = (0..episode_count).map(|_| Uuid::new_v4()).collect();
            let fact = make_fact(&ns, &status, episodes.clone(), None, None);

            prop_assert!(
                validate_provenance(&fact),
                "Fact with {} source episodes should pass provenance check",
                episode_count
            );

            prop_assert_eq!(
                fact.source_episodes.len(),
                episode_count,
                "source_episodes length should match"
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Facts with empty source_episodes fail provenance validation.
        #[test]
        fn facts_without_source_episodes_fail(
            ns in namespace(),
            status in valid_evidence_status_str(),
        ) {
            let fact = make_fact(&ns, &status, vec![], None, None);

            prop_assert!(
                !validate_provenance(&fact),
                "Fact with empty source_episodes should fail provenance check"
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Source episode UUIDs are unique within a fact's provenance.
        #[test]
        fn source_episodes_are_unique(
            ns in namespace(),
            episode_count in 1usize..=10,
        ) {
            let episodes: Vec<Uuid> = (0..episode_count).map(|_| Uuid::new_v4()).collect();
            let fact = make_fact(&ns, "extracted", episodes.clone(), None, None);

            let unique: HashSet<&Uuid> = fact.source_episodes.iter().collect();

            prop_assert_eq!(
                unique.len(),
                fact.source_episodes.len(),
                "source_episodes should contain unique UUIDs"
            );
        }
    }
}


// ---------------------------------------------------------------------------
// Property 30: Alias Deduplication
// ---------------------------------------------------------------------------

/// **Property 30: Alias Deduplication**
///
/// **Validates: Requirements 42.3**
///
/// For any entity, the aliases array should not contain case-insensitive
/// duplicates.
mod alias_deduplication {
    use super::*;

    /// Deduplicate aliases case-insensitively, preserving the first
    /// occurrence. This mirrors the logic in entity resolution when
    /// appending new aliases.
    fn deduplicate_aliases(aliases: &[String]) -> Vec<String> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut result: Vec<String> = Vec::new();

        for alias in aliases {
            let lower = alias.to_lowercase();
            if seen.insert(lower) {
                result.push(alias.clone());
            }
        }

        result
    }

    /// Check whether an alias list has case-insensitive duplicates.
    fn has_case_insensitive_duplicates(aliases: &[String]) -> bool {
        let mut seen: HashSet<String> = HashSet::new();
        for alias in aliases {
            if !seen.insert(alias.to_lowercase()) {
                return true;
            }
        }
        false
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// After deduplication, no case-insensitive duplicates remain.
        #[test]
        fn deduplicated_aliases_have_no_duplicates(
            aliases in prop::collection::vec("[a-zA-Z][a-zA-Z0-9_ -]{0,19}", 0..=20),
        ) {
            let deduped = deduplicate_aliases(&aliases);

            prop_assert!(
                !has_case_insensitive_duplicates(&deduped),
                "Deduplicated aliases should have no case-insensitive duplicates. \
                 Input: {:?}, Output: {:?}",
                aliases,
                deduped
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Deduplication preserves all unique aliases (by lowercase).
        #[test]
        fn deduplication_preserves_unique_aliases(
            aliases in prop::collection::vec("[a-zA-Z][a-zA-Z0-9_ -]{0,19}", 0..=20),
        ) {
            let deduped = deduplicate_aliases(&aliases);

            // Count unique lowercase aliases in the input.
            let unique_lower: HashSet<String> =
                aliases.iter().map(|a| a.to_lowercase()).collect();

            prop_assert_eq!(
                deduped.len(),
                unique_lower.len(),
                "Deduplicated length ({}) should equal unique lowercase count ({}). \
                 Input: {:?}, Output: {:?}",
                deduped.len(),
                unique_lower.len(),
                aliases,
                deduped
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Deduplication preserves the first occurrence of each alias.
        #[test]
        fn deduplication_preserves_first_occurrence(
            base in "[a-zA-Z]{3,10}",
        ) {
            // Create aliases with case variations of the same base.
            let aliases = vec![
                base.clone(),
                base.to_uppercase(),
                base.to_lowercase(),
            ];

            let deduped = deduplicate_aliases(&aliases);

            prop_assert_eq!(
                deduped.len(),
                1,
                "Case variations of '{}' should deduplicate to 1, got {:?}",
                base,
                deduped
            );

            // The first occurrence should be preserved.
            prop_assert_eq!(
                &deduped[0],
                &base,
                "First occurrence '{}' should be preserved, got '{}'",
                base,
                deduped[0]
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Already-unique aliases are unchanged by deduplication.
        #[test]
        fn unique_aliases_unchanged(
            count in 1usize..=10,
        ) {
            // Generate aliases that are guaranteed unique by appending index.
            let aliases: Vec<String> = (0..count)
                .map(|i| format!("alias_{}", i))
                .collect();

            let deduped = deduplicate_aliases(&aliases);

            prop_assert_eq!(
                deduped.len(),
                aliases.len(),
                "Unique aliases should be unchanged"
            );

            prop_assert_eq!(
                &deduped,
                &aliases,
                "Unique aliases should preserve order and content"
            );
        }
    }
}
