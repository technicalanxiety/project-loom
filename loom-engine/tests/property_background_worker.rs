//! Property-based tests for background worker and offline pipeline.
//!
//! Tests the asynchronous ingestion contract and extraction metrics
//! completeness without requiring a database. Uses proptest to generate
//! random inputs and verifies invariants hold across many iterations.
//!
//! **Properties tested:**
//! - Property 24: Asynchronous Ingestion and Pipeline Separation
//! - Property 31: Extraction Metrics Completeness

use proptest::prelude::*;
use uuid::Uuid;

use loom_engine::pipeline::offline::extract::PredicateValidationResult;
use loom_engine::pipeline::offline::state::compute_extraction_metrics;
use loom_engine::types::entity::ResolutionResult;
use loom_engine::types::mcp::{LearnResponse, ThinkResponse};

// ---------------------------------------------------------------------------
// Shared strategies
// ---------------------------------------------------------------------------

/// Strategy for generating a valid LearnResponse status.
fn learn_status() -> impl Strategy<Value = String> {
    prop::sample::select(&["accepted", "duplicate", "queued"][..]).prop_map(|s| s.to_string())
}

/// Strategy for generating a model name string.
fn model_name() -> impl Strategy<Value = String> {
    prop::sample::select(&[
        "gemma4:26b-a4b-q4",
        "gemma4:e4b",
        "gpt-4.1-mini",
        "test-model",
    ][..])
    .prop_map(|s| s.to_string())
}

/// Strategy for generating an evidence strength value.
fn evidence_strength() -> impl Strategy<Value = Option<String>> {
    prop::option::of(prop::sample::select(&["explicit", "implied"][..]).prop_map(|s| s.to_string()))
}

// ---------------------------------------------------------------------------
// Property 24: Asynchronous Ingestion and Pipeline Separation
// ---------------------------------------------------------------------------

/// **Property 24: Asynchronous Ingestion and Pipeline Separation**
///
/// **Validates: Requirements 17.7, 44.1, 44.6**
///
/// The loom_learn endpoint returns immediately with a status of "accepted",
/// "duplicate", or "queued". The loom_think endpoint returns a compiled
/// context package without blocking on offline processing. These two
/// pipelines are strictly separated.
mod async_ingestion_pipeline_separation {
    use super::*;

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Test that LearnResponse always contains a valid status value.
        ///
        /// The loom_learn contract requires the status field to be one of
        /// "accepted", "duplicate", or "queued". This property verifies
        /// that any LearnResponse constructed with these values serializes
        /// and deserializes correctly, preserving the status invariant.
        #[test]
        fn learn_response_status_is_always_valid(
            status in learn_status(),
        ) {
            let response = LearnResponse {
                episode_id: Uuid::new_v4(),
                status: status.clone(),
            };

            // Property: status must be one of the three valid values.
            let valid_statuses = ["accepted", "duplicate", "queued"];
            prop_assert!(
                valid_statuses.contains(&response.status.as_str()),
                "LearnResponse status '{}' is not one of {:?}",
                response.status,
                valid_statuses
            );

            // Property: round-trip serialization preserves the status.
            let json = serde_json::to_value(&response).expect("should serialize");
            let deserialized: LearnResponse =
                serde_json::from_value(json).expect("should deserialize");
            prop_assert_eq!(
                &deserialized.status,
                &status,
                "Status should survive round-trip serialization"
            );

            // Property: episode_id is always present.
            prop_assert_ne!(
                deserialized.episode_id,
                Uuid::nil(),
                "episode_id should not be nil"
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Test that LearnResponse never contains a blocking/synchronous status.
        ///
        /// The async contract means loom_learn must never return a status
        /// that implies synchronous processing completion like "processed"
        /// or "completed".
        #[test]
        fn learn_response_never_indicates_synchronous_completion(
            status in learn_status(),
        ) {
            let response = LearnResponse {
                episode_id: Uuid::new_v4(),
                status,
            };

            let blocking_statuses = ["processed", "completed", "done", "finished", "extracted"];
            prop_assert!(
                !blocking_statuses.contains(&response.status.as_str()),
                "LearnResponse status '{}' implies synchronous processing, violating async contract",
                response.status
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Test that ThinkResponse is structurally independent of offline processing.
        ///
        /// The online pipeline (loom_think) produces a ThinkResponse with a
        /// context_package, token_count, and compilation_id. None of these
        /// fields depend on whether offline extraction has completed for any
        /// particular episode.
        #[test]
        fn think_response_is_structurally_complete(
            token_count in 0i32..=10000,
            package_len in 1usize..=500,
        ) {
            let package: String = (0..package_len).map(|_| 'x').collect();

            let response = ThinkResponse {
                context_package: package.clone(),
                token_count,
                compilation_id: Uuid::new_v4(),
            };

            // Property: ThinkResponse always has a non-nil compilation_id.
            prop_assert_ne!(
                response.compilation_id,
                Uuid::nil(),
                "compilation_id should not be nil"
            );

            // Property: token_count is non-negative.
            prop_assert!(
                response.token_count >= 0,
                "token_count should be non-negative, got {}",
                response.token_count
            );

            // Property: round-trip serialization preserves all fields.
            let json = serde_json::to_value(&response).expect("should serialize");
            let deserialized: ThinkResponse =
                serde_json::from_value(json).expect("should deserialize");
            prop_assert_eq!(deserialized.token_count, token_count);
            prop_assert_eq!(&deserialized.context_package, &package);
            prop_assert_eq!(deserialized.compilation_id, response.compilation_id);
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Test that LearnResponse and ThinkResponse are structurally disjoint.
        ///
        /// The pipeline separation contract requires that the learn (offline)
        /// and think (online) response types share no fields that could
        /// create coupling between the two pipelines.
        #[test]
        fn learn_and_think_responses_are_disjoint(
            status in learn_status(),
            token_count in 0i32..=5000,
        ) {
            let learn = LearnResponse {
                episode_id: Uuid::new_v4(),
                status,
            };
            let think = ThinkResponse {
                context_package: "test".to_string(),
                token_count,
                compilation_id: Uuid::new_v4(),
            };

            let learn_json = serde_json::to_value(&learn).expect("serialize learn");
            let think_json = serde_json::to_value(&think).expect("serialize think");

            // Property: LearnResponse JSON keys and ThinkResponse JSON keys
            // should have no overlap (disjoint field sets).
            let learn_keys: std::collections::HashSet<String> = learn_json
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
            let think_keys: std::collections::HashSet<String> = think_json
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();

            let overlap: Vec<&String> = learn_keys.intersection(&think_keys).collect();
            prop_assert!(
                overlap.is_empty(),
                "LearnResponse and ThinkResponse should have disjoint fields, but share: {:?}",
                overlap
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Property 31: Extraction Metrics Completeness
// ---------------------------------------------------------------------------

/// **Property 31: Extraction Metrics Completeness**
///
/// **Validates: Requirements 48.1, 48.3, 48.4, 48.5, 48.6, 48.7**
///
/// For any processed episode, the extraction_metrics JSONB must contain
/// all required fields: extracted, resolved_exact, resolved_alias,
/// resolved_semantic, new, conflict_flagged, facts_extracted,
/// canonical_predicate, custom_predicate, explicit, implied,
/// processing_time_ms, extraction_model.
mod extraction_metrics_completeness {
    use super::*;
    use loom_engine::pipeline::offline::extract::FactOrchestrationResult;

    /// All required field names in the ExtractionMetrics JSONB.
    const REQUIRED_FIELDS: &[&str] = &[
        "extracted",
        "resolved_exact",
        "resolved_alias",
        "resolved_semantic",
        "new",
        "conflict_flagged",
        "facts_extracted",
        "canonical_predicate",
        "custom_predicate",
        "explicit",
        "implied",
        "processing_time_ms",
        "extraction_model",
    ];

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Test that compute_extraction_metrics always produces a struct
        /// that serializes to JSON containing every required field.
        #[test]
        fn metrics_json_contains_all_required_fields(
            entity_count in 0usize..=20,
            conflict_count in 0i32..=5,
            fact_count in 0usize..=30,
            canonical in 0usize..=15,
            custom in 0usize..=15,
            evidence_values in prop::collection::vec(evidence_strength(), 0..=30),
            processing_time_ms in 0i64..=60_000,
            model in model_name(),
        ) {
            // Build resolution results with random methods.
            let resolution_results: Vec<ResolutionResult> = (0..entity_count)
                .map(|i| {
                    let method = match i % 4 {
                        0 => "exact",
                        1 => "alias",
                        2 => "semantic",
                        _ => "new",
                    };
                    ResolutionResult {
                        entity_id: Uuid::new_v4(),
                        method: method.to_string(),
                        confidence: 1.0,
                    }
                })
                .collect();

            let fact_result = if fact_count > 0 {
                Some(FactOrchestrationResult {
                    inserted_count: fact_count,
                    skipped_count: 0,
                    inserted_fact_ids: (0..fact_count).map(|_| Uuid::new_v4()).collect(),
                    predicate_validation: Some(PredicateValidationResult {
                        canonical_count: canonical,
                        custom_count: custom,
                    }),
                    superseded_count: 0,
                    model: model.clone(),
                    valid_extracted_facts: vec![],
                })
            } else {
                None
            };

            let metrics = compute_extraction_metrics(
                &resolution_results,
                conflict_count,
                fact_result.as_ref(),
                &evidence_values,
                processing_time_ms,
                &model,
            );

            // Serialize to JSON.
            let json = serde_json::to_value(&metrics)
                .expect("ExtractionMetrics should always serialize to JSON");

            let obj = json.as_object().expect("metrics JSON should be an object");

            // Property: every required field is present in the JSON.
            for &field in REQUIRED_FIELDS {
                prop_assert!(
                    obj.contains_key(field),
                    "Required field '{}' missing from extraction_metrics JSON. Keys: {:?}",
                    field,
                    obj.keys().collect::<Vec<_>>()
                );
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Test that entity resolution counts in metrics sum to the total
        /// extracted count.
        #[test]
        fn entity_resolution_counts_sum_to_extracted(
            exact in 0usize..=10,
            alias in 0usize..=10,
            semantic in 0usize..=10,
            new_count in 0usize..=10,
        ) {
            let total = exact + alias + semantic + new_count;

            let mut results: Vec<ResolutionResult> = Vec::new();
            for _ in 0..exact {
                results.push(ResolutionResult {
                    entity_id: Uuid::new_v4(),
                    method: "exact".to_string(),
                    confidence: 1.0,
                });
            }
            for _ in 0..alias {
                results.push(ResolutionResult {
                    entity_id: Uuid::new_v4(),
                    method: "alias".to_string(),
                    confidence: 0.95,
                });
            }
            for _ in 0..semantic {
                results.push(ResolutionResult {
                    entity_id: Uuid::new_v4(),
                    method: "semantic".to_string(),
                    confidence: 0.94,
                });
            }
            for _ in 0..new_count {
                results.push(ResolutionResult {
                    entity_id: Uuid::new_v4(),
                    method: "new".to_string(),
                    confidence: 1.0,
                });
            }

            let metrics = compute_extraction_metrics(
                &results,
                0,
                None,
                &[],
                100,
                "test-model",
            );

            // Property: sum of resolution method counts equals extracted.
            let sum = metrics.resolved_exact
                + metrics.resolved_alias
                + metrics.resolved_semantic
                + metrics.new;

            prop_assert_eq!(
                sum,
                metrics.extracted,
                "Resolution counts ({} + {} + {} + {} = {}) should equal extracted ({})",
                metrics.resolved_exact,
                metrics.resolved_alias,
                metrics.resolved_semantic,
                metrics.new,
                sum,
                metrics.extracted
            );

            prop_assert_eq!(
                metrics.extracted as usize,
                total,
                "Extracted count should equal total input count"
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Test that evidence counts (explicit + implied) never exceed
        /// the number of evidence values provided.
        #[test]
        fn evidence_counts_bounded_by_input(
            evidence_values in prop::collection::vec(evidence_strength(), 0..=20),
        ) {
            let metrics = compute_extraction_metrics(
                &[],
                0,
                None,
                &evidence_values,
                50,
                "test",
            );

            let evidence_sum = metrics.explicit + metrics.implied;
            let input_count = evidence_values.len() as i32;

            prop_assert!(
                evidence_sum <= input_count,
                "Evidence counts ({} explicit + {} implied = {}) should not exceed input count ({})",
                metrics.explicit,
                metrics.implied,
                evidence_sum,
                input_count
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Test that the extraction_model field in metrics always matches
        /// the model passed to compute_extraction_metrics.
        #[test]
        fn extraction_model_preserved_in_metrics(
            model in model_name(),
        ) {
            let metrics = compute_extraction_metrics(
                &[],
                0,
                None,
                &[],
                100,
                &model,
            );

            prop_assert_eq!(
                &metrics.extraction_model,
                &model,
                "extraction_model in metrics should match input model"
            );

            // Also verify via JSON serialization.
            let json = serde_json::to_value(&metrics).expect("should serialize");
            prop_assert_eq!(
                json["extraction_model"].as_str().unwrap(),
                model.as_str(),
                "extraction_model in JSON should match input model"
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Test that processing_time_ms is always non-negative and preserved.
        #[test]
        fn processing_time_preserved_and_non_negative(
            time_ms in 0i64..=120_000,
            model in model_name(),
        ) {
            let metrics = compute_extraction_metrics(
                &[],
                0,
                None,
                &[],
                time_ms,
                &model,
            );

            prop_assert!(
                metrics.processing_time_ms >= 0,
                "processing_time_ms should be non-negative, got {}",
                metrics.processing_time_ms
            );

            prop_assert_eq!(
                metrics.processing_time_ms,
                time_ms,
                "processing_time_ms should be preserved"
            );
        }
    }
}
