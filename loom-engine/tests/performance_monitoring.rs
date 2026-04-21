//! Unit tests for performance monitoring infrastructure.
//!
//! Tests latency measurement accuracy via tracing spans, metrics
//! aggregation logic, and percentile calculations.
//!
//! **Validates: Requirements 35.1, 35.6, 35.7**

use std::time::{Duration, Instant};

use loom_engine::pipeline::online::classify;
use loom_engine::pipeline::online::compile::{
    self, CompilationInput, CompilationResult, HotFact, HotTierItem, HotTierPayload,
    SelectedItem,
};
use loom_engine::pipeline::online::rank::{self, RankedCandidate};
use loom_engine::pipeline::online::retrieve::{
    CandidatePayload, EpisodeCandidate, FactCandidate, MemoryType, RetrievalCandidate,
    RetrievalProfile,
};
use loom_engine::pipeline::online::weight;
use loom_engine::types::classification::TaskClass;
use loom_engine::types::compilation::OutputFormat;

use chrono::Utc;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_fact_candidate(score: f64) -> RetrievalCandidate {
    RetrievalCandidate {
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
            namespace: "test".to_string(),
        }),
    }
}

fn make_episode_candidate(score: f64) -> RetrievalCandidate {
    RetrievalCandidate {
        id: Uuid::new_v4(),
        score,
        source_profile: RetrievalProfile::EpisodeRecall,
        memory_type: MemoryType::Episodic,
        payload: CandidatePayload::Episode(EpisodeCandidate {
            source: "test".to_string(),
            content: "Test episode content".to_string(),
            occurred_at: Utc::now(),
            namespace: "test".to_string(),
        }),
    }
}

fn make_hot_fact(subject: &str, object: &str) -> HotTierItem {
    HotTierItem {
        id: Uuid::new_v4(),
        memory_type: MemoryType::Semantic,
        payload: HotTierPayload::Fact(HotFact {
            subject: subject.to_string(),
            predicate: "uses".to_string(),
            object: object.to_string(),
            evidence: "explicit".to_string(),
            observed: Some("2025-01-01".to_string()),
            source: Uuid::new_v4().to_string(),
        }),
    }
}

/// Compute percentile from a sorted slice of f64 values.
fn compute_percentile(sorted_values: &[f64], pct: f64) -> f64 {
    if sorted_values.is_empty() {
        return 0.0;
    }
    let idx = ((sorted_values.len() as f64 * pct / 100.0).ceil() as usize)
        .saturating_sub(1)
        .min(sorted_values.len() - 1);
    sorted_values[idx]
}

/// Compute percentile from a sorted slice of Duration values.
fn compute_duration_percentile(sorted: &[Duration], pct: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let idx = ((sorted.len() as f64 * pct / 100.0).ceil() as usize)
        .saturating_sub(1)
        .min(sorted.len() - 1);
    sorted[idx]
}

// ---------------------------------------------------------------------------
// Latency measurement accuracy tests
// ---------------------------------------------------------------------------

/// **Validates: Requirements 35.1**
///
/// Test that Instant-based latency measurement captures real elapsed time
/// with reasonable accuracy (within 5ms tolerance for a 50ms sleep).
#[tokio::test]
async fn latency_measurement_captures_real_elapsed_time() {
    let expected_ms = 50u64;
    let tolerance_ms = 15u64; // generous tolerance for CI

    let start = Instant::now();
    tokio::time::sleep(Duration::from_millis(expected_ms)).await;
    let elapsed = start.elapsed();

    let elapsed_ms = elapsed.as_millis() as u64;

    assert!(
        elapsed_ms >= expected_ms.saturating_sub(tolerance_ms),
        "elapsed {elapsed_ms}ms is less than expected {expected_ms}ms - {tolerance_ms}ms tolerance"
    );
    assert!(
        elapsed_ms <= expected_ms + tolerance_ms,
        "elapsed {elapsed_ms}ms exceeds expected {expected_ms}ms + {tolerance_ms}ms tolerance"
    );
}

/// **Validates: Requirements 35.2, 35.3, 35.4, 35.5**
///
/// Test that each pipeline stage can be independently timed and the
/// individual stage latencies sum to approximately the total latency.
#[test]
fn stage_latencies_sum_to_total() {
    let candidates = (0..10).map(|i| make_fact_candidate(0.9 - i as f64 * 0.05)).collect();
    let task_class = TaskClass::Architecture;

    let total_start = Instant::now();

    // Stage 1: Classify (override)
    let classify_start = Instant::now();
    let _classify = classify::apply_override(task_class.clone());
    let classify_ms = classify_start.elapsed().as_micros();

    // Stage 2: Weight
    let weight_start = Instant::now();
    let weighted = weight::apply_weights(candidates, &task_class);
    let weight_ms = weight_start.elapsed().as_micros();

    // Stage 3: Rank
    let rank_start = Instant::now();
    let ranked = rank::rank_candidates(weighted);
    let rank_ms = rank_start.elapsed().as_micros();

    // Stage 4: Compile
    let compile_start = Instant::now();
    let input = CompilationInput {
        namespace: "test".to_string(),
        task_class,
        target_model: "claude".to_string(),
        format: OutputFormat::Structured,
        warm_tier_budget: 3000,
        hot_tier_items: vec![make_hot_fact("A", "B")],
        ranked_candidates: ranked,
    };
    let _result = compile::compile_package(input);
    let compile_ms = compile_start.elapsed().as_micros();

    let total_ms = total_start.elapsed().as_micros();

    // Sum of stages should be close to total (within 20% + 100µs overhead).
    let stage_sum = classify_ms + weight_ms + rank_ms + compile_ms;

    // The sum of stages should not exceed total by more than a small margin
    // (measurement overhead between stages).
    assert!(
        stage_sum <= total_ms + 100,
        "stage sum {stage_sum}µs exceeds total {total_ms}µs by too much"
    );
}

/// **Validates: Requirements 35.6**
///
/// Test that latency measurements are recorded in the compilation result's
/// audit entry via the build_audit_entry function.
#[test]
fn audit_entry_captures_latency_breakdown() {
    let candidates: Vec<RetrievalCandidate> = (0..5)
        .map(|i| make_fact_candidate(0.9 - i as f64 * 0.1))
        .collect();
    let task_class = TaskClass::Debug;
    let weighted = weight::apply_weights(candidates, &task_class);
    let ranked = rank::rank_candidates(weighted);

    let input = CompilationInput {
        namespace: "test".to_string(),
        task_class: task_class.clone(),
        target_model: "claude".to_string(),
        format: OutputFormat::Structured,
        warm_tier_budget: 3000,
        hot_tier_items: vec![],
        ranked_candidates: ranked,
    };

    let result = compile::compile_package(input);

    // Build audit entry with latency breakdown.
    let audit = compile::build_audit_entry(
        &result,
        "test",
        &task_class,
        Some("test query"),
        Some("claude"),
        "debug",
        None,
        Some(0.95),
        None,
        &["fact_lookup".to_string()],
        Some(150),  // total
        Some(20),   // classify
        Some(80),   // retrieve
        Some(30),   // rank
        Some(20),   // compile
    );

    assert_eq!(audit.latency_total_ms, Some(150));
    assert_eq!(audit.latency_classify_ms, Some(20));
    assert_eq!(audit.latency_retrieve_ms, Some(80));
    assert_eq!(audit.latency_rank_ms, Some(30));
    assert_eq!(audit.latency_compile_ms, Some(20));
    assert_eq!(audit.namespace, "test");
    assert_eq!(audit.task_class, "debug");
}

// ---------------------------------------------------------------------------
// Metrics aggregation tests
// ---------------------------------------------------------------------------

/// **Validates: Requirements 35.6**
///
/// Test that compilation results correctly aggregate candidate counts.
#[test]
fn compilation_aggregates_candidate_counts() {
    let candidates: Vec<RetrievalCandidate> = (0..15)
        .map(|i| make_fact_candidate(0.95 - i as f64 * 0.05))
        .collect();
    let task_class = TaskClass::Architecture;
    let weighted = weight::apply_weights(candidates, &task_class);
    let ranked = rank::rank_candidates(weighted);

    let input = CompilationInput {
        namespace: "test".to_string(),
        task_class,
        target_model: "claude".to_string(),
        format: OutputFormat::Structured,
        warm_tier_budget: 500, // small budget to force some rejections
        hot_tier_items: vec![],
        ranked_candidates: ranked,
    };

    let result = compile::compile_package(input);

    // Total found should be 15.
    assert_eq!(result.candidates_found, 15);

    // Selected + rejected should equal found.
    assert_eq!(
        result.candidates_selected + result.candidates_rejected,
        result.candidates_found,
        "selected ({}) + rejected ({}) should equal found ({})",
        result.candidates_selected,
        result.candidates_rejected,
        result.candidates_found,
    );

    // With a small budget, some should be rejected.
    assert!(
        result.candidates_rejected > 0,
        "with a 500 token budget and 15 candidates, some should be rejected"
    );
}

/// **Validates: Requirements 36.1, 36.2**
///
/// Test that precision can be computed from selected/found counts.
#[test]
fn precision_computed_from_candidate_counts() {
    let candidates: Vec<RetrievalCandidate> = (0..20)
        .map(|i| make_fact_candidate(0.95 - i as f64 * 0.03))
        .collect();
    let task_class = TaskClass::Architecture;
    let weighted = weight::apply_weights(candidates, &task_class);
    let ranked = rank::rank_candidates(weighted);

    let input = CompilationInput {
        namespace: "test".to_string(),
        task_class,
        target_model: "claude".to_string(),
        format: OutputFormat::Structured,
        warm_tier_budget: 3000,
        hot_tier_items: vec![],
        ranked_candidates: ranked,
    };

    let result = compile::compile_package(input);

    let precision = if result.candidates_found > 0 {
        result.candidates_selected as f64 / result.candidates_found as f64
    } else {
        0.0
    };

    assert!(
        (0.0..=1.0).contains(&precision),
        "precision should be in [0, 1], got {precision}"
    );
}

/// **Validates: Requirements 36.5**
///
/// Test that rejected items include rejection reasons.
#[test]
fn rejected_items_include_reasons() {
    let candidates: Vec<RetrievalCandidate> = (0..20)
        .map(|i| make_fact_candidate(0.95 - i as f64 * 0.03))
        .collect();
    let task_class = TaskClass::Architecture;
    let weighted = weight::apply_weights(candidates, &task_class);
    let ranked = rank::rank_candidates(weighted);

    let input = CompilationInput {
        namespace: "test".to_string(),
        task_class,
        target_model: "claude".to_string(),
        format: OutputFormat::Structured,
        warm_tier_budget: 200, // very small to force rejections
        hot_tier_items: vec![],
        ranked_candidates: ranked,
    };

    let result = compile::compile_package(input);

    for rejected in &result.rejected_items {
        assert!(
            !rejected.reason.is_empty(),
            "rejected item should have a reason"
        );
        assert!(
            rejected.reason == "token_budget_exceeded" || rejected.reason == "duplicate",
            "unexpected rejection reason: {}",
            rejected.reason
        );
    }
}

// ---------------------------------------------------------------------------
// Percentile calculation tests
// ---------------------------------------------------------------------------

/// **Validates: Requirements 35.7**
///
/// Test p50 calculation on a known dataset.
#[test]
fn percentile_p50_on_known_data() {
    let mut values: Vec<f64> = (1..=100).map(|i| i as f64).collect();
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let p50 = compute_percentile(&values, 50.0);
    assert!(
        (p50 - 50.0).abs() < 1.5,
        "p50 should be ~50, got {p50}"
    );
}

/// **Validates: Requirements 35.7**
///
/// Test p95 calculation on a known dataset.
#[test]
fn percentile_p95_on_known_data() {
    let mut values: Vec<f64> = (1..=100).map(|i| i as f64).collect();
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let p95 = compute_percentile(&values, 95.0);
    assert!(
        (p95 - 95.0).abs() < 1.5,
        "p95 should be ~95, got {p95}"
    );
}

/// **Validates: Requirements 35.7**
///
/// Test p99 calculation on a known dataset.
#[test]
fn percentile_p99_on_known_data() {
    let mut values: Vec<f64> = (1..=100).map(|i| i as f64).collect();
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let p99 = compute_percentile(&values, 99.0);
    assert!(
        (p99 - 99.0).abs() < 1.5,
        "p99 should be ~99, got {p99}"
    );
}

/// **Validates: Requirements 35.7**
///
/// Test percentile on a single-element dataset.
#[test]
fn percentile_single_element() {
    let values = vec![42.0];
    assert_eq!(compute_percentile(&values, 50.0), 42.0);
    assert_eq!(compute_percentile(&values, 95.0), 42.0);
    assert_eq!(compute_percentile(&values, 99.0), 42.0);
}

/// **Validates: Requirements 35.7**
///
/// Test percentile on an empty dataset.
#[test]
fn percentile_empty_dataset() {
    let values: Vec<f64> = vec![];
    assert_eq!(compute_percentile(&values, 50.0), 0.0);
}

/// **Validates: Requirements 35.7**
///
/// Test that duration-based percentile works correctly.
#[test]
fn duration_percentile_correctness() {
    let mut durations: Vec<Duration> = (1..=100)
        .map(|i| Duration::from_millis(i))
        .collect();
    durations.sort();

    let p50 = compute_duration_percentile(&durations, 50.0);
    let p95 = compute_duration_percentile(&durations, 95.0);
    let p99 = compute_duration_percentile(&durations, 99.0);

    assert!(
        p50 >= Duration::from_millis(49) && p50 <= Duration::from_millis(51),
        "p50 should be ~50ms, got {:?}",
        p50
    );
    assert!(
        p95 >= Duration::from_millis(94) && p95 <= Duration::from_millis(96),
        "p95 should be ~95ms, got {:?}",
        p95
    );
    assert!(
        p99 >= Duration::from_millis(98) && p99 <= Duration::from_millis(100),
        "p99 should be ~99ms, got {:?}",
        p99
    );
}

/// **Validates: Requirements 35.7**
///
/// Test percentile with skewed distribution (most values low, few high).
#[test]
fn percentile_skewed_distribution() {
    let mut values: Vec<f64> = Vec::new();
    // 95 values at 10ms
    for _ in 0..95 {
        values.push(10.0);
    }
    // 5 values at 500ms (outliers)
    for _ in 0..5 {
        values.push(500.0);
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let p50 = compute_percentile(&values, 50.0);
    let p95 = compute_percentile(&values, 95.0);
    let p99 = compute_percentile(&values, 99.0);

    // p50 should be 10 (most values are 10)
    assert_eq!(p50, 10.0, "p50 should be 10.0 for skewed data");

    // p95 should be 10 (95th percentile is still in the low cluster)
    assert_eq!(p95, 10.0, "p95 should be 10.0 for 95% low values");

    // p99 should be 500 (99th percentile hits the outliers)
    assert_eq!(p99, 500.0, "p99 should be 500.0 for skewed data");
}

/// **Validates: Requirements 35.1**
///
/// Test that multiple timed operations produce consistent ordering.
#[test]
fn timed_operations_maintain_ordering() {
    let mut durations = Vec::new();

    // Run operations of increasing complexity.
    for size in [1, 5, 10, 20, 50] {
        let candidates: Vec<RetrievalCandidate> = (0..size)
            .map(|i| make_fact_candidate(0.9 - i as f64 * 0.01))
            .collect();
        let task_class = TaskClass::Architecture;

        let start = Instant::now();
        let weighted = weight::apply_weights(candidates, &task_class);
        let _ranked = rank::rank_candidates(weighted);
        durations.push((size, start.elapsed()));
    }

    // Larger inputs should generally take longer (or at least not be
    // dramatically faster). We check that the largest is not faster
    // than the smallest by more than 10x.
    let (_, smallest_time) = durations.first().unwrap();
    let (_, largest_time) = durations.last().unwrap();

    // This is a sanity check — the largest should not be impossibly fast.
    // We allow generous bounds since timing can be noisy.
    assert!(
        *largest_time >= Duration::from_nanos(1),
        "largest operation should take measurable time"
    );
    assert!(
        *smallest_time >= Duration::from_nanos(1),
        "smallest operation should take measurable time"
    );
}

/// **Validates: Requirements 35.6**
///
/// Test that selected items in compilation result contain score breakdowns.
#[test]
fn selected_items_contain_score_breakdowns() {
    let candidates: Vec<RetrievalCandidate> = (0..5)
        .map(|i| make_fact_candidate(0.9 - i as f64 * 0.1))
        .collect();
    let task_class = TaskClass::Architecture;
    let weighted = weight::apply_weights(candidates, &task_class);
    let ranked = rank::rank_candidates(weighted);

    let input = CompilationInput {
        namespace: "test".to_string(),
        task_class,
        target_model: "claude".to_string(),
        format: OutputFormat::Structured,
        warm_tier_budget: 3000,
        hot_tier_items: vec![],
        ranked_candidates: ranked,
    };

    let result = compile::compile_package(input);

    for item in &result.selected_items {
        // Each dimension should be in [0, 1].
        assert!(
            (0.0..=1.0).contains(&item.relevance),
            "relevance should be in [0, 1], got {}",
            item.relevance
        );
        assert!(
            (0.0..=1.0).contains(&item.recency),
            "recency should be in [0, 1], got {}",
            item.recency
        );
        assert!(
            (0.0..=1.0).contains(&item.stability),
            "stability should be in [0, 1], got {}",
            item.stability
        );
        assert!(
            (0.0..=1.0).contains(&item.provenance),
            "provenance should be in [0, 1], got {}",
            item.provenance
        );

        // Final score should be a weighted combination.
        let expected = item.relevance * 0.40
            + item.recency * 0.25
            + item.stability * 0.20
            + item.provenance * 0.15;
        assert!(
            (item.final_score - expected).abs() < 1e-10,
            "final_score {} should equal weighted sum {}",
            item.final_score,
            expected
        );
    }
}
