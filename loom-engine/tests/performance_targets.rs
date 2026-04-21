//! Performance target validation tests.
//!
//! These tests validate that the Rust code paths meet latency targets
//! using mocked LLM responses (no real Ollama or database). They measure
//! the overhead of the pipeline stages themselves, not LLM inference time.
//!
//! Targets (from Requirement 35.7):
//! - loom_think p95 < 500ms, p99 < 1000ms (Rust code path only)
//! - loom_learn < 100ms (async return, no extraction)
//! - Episode processing < 3 seconds per episode
//! - 10 concurrent loom_think calls
//! - 100 episodes per minute ingestion rate

use std::time::{Duration, Instant};

use loom_engine::pipeline::online::classify::{self, ClassifyStageOutput};
use loom_engine::pipeline::online::compile::{
    self, CompilationInput, HotFact, HotTierItem, HotTierPayload,
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
// Test helpers
// ---------------------------------------------------------------------------

/// Build a mock ClassifyStageOutput for testing.
fn mock_classify_output() -> ClassifyStageOutput {
    classify::apply_override(TaskClass::Architecture)
}

/// Build a vector of mock retrieval candidates.
fn mock_candidates(count: usize) -> Vec<RetrievalCandidate> {
    (0..count)
        .map(|i| {
            let score = 1.0 - (i as f64 * 0.05);
            if i % 2 == 0 {
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
            } else {
                RetrievalCandidate {
                    id: Uuid::new_v4(),
                    score,
                    source_profile: RetrievalProfile::EpisodeRecall,
                    memory_type: MemoryType::Episodic,
                    payload: CandidatePayload::Episode(EpisodeCandidate {
                        source: "test".to_string(),
                        content: format!("Episode content for test candidate {i}"),
                        occurred_at: Utc::now(),
                        namespace: "test".to_string(),
                    }),
                }
            }
        })
        .collect()
}

/// Build mock hot tier items.
fn mock_hot_tier_items(count: usize) -> Vec<HotTierItem> {
    (0..count)
        .map(|i| HotTierItem {
            id: Uuid::new_v4(),
            memory_type: MemoryType::Semantic,
            payload: HotTierPayload::Fact(HotFact {
                subject: format!("Entity{i}"),
                predicate: "uses".to_string(),
                object: format!("Tech{i}"),
                evidence: "explicit".to_string(),
                observed: Some("2025-01-01".to_string()),
                source: Uuid::new_v4().to_string(),
            }),
        })
        .collect()
}

/// Run the full in-memory pipeline (classify override → weight → rank → compile)
/// and return the elapsed duration.
fn run_pipeline_in_memory(candidate_count: usize, hot_tier_count: usize) -> Duration {
    let start = Instant::now();

    // Stage 1: Classification (override — instant)
    let _classify_output = mock_classify_output();
    let task_class = TaskClass::Architecture;

    // Stage 2: Retrieval (mocked — just build candidates)
    let candidates = mock_candidates(candidate_count);

    // Stage 3: Weight application
    let weighted = weight::apply_weights(candidates, &task_class);

    // Stage 4: Ranking
    let ranked = rank::rank_candidates(weighted);

    // Stage 5: Compilation
    let hot_items = mock_hot_tier_items(hot_tier_count);
    let input = CompilationInput {
        namespace: "test-namespace".to_string(),
        task_class,
        target_model: "claude-3.5-sonnet".to_string(),
        format: OutputFormat::Structured,
        warm_tier_budget: 3000,
        hot_tier_items: hot_items,
        ranked_candidates: ranked,
    };

    let _result = compile::compile_package(input);

    start.elapsed()
}

/// Compute percentile from a sorted list of durations.
fn percentile(sorted_durations: &[Duration], pct: f64) -> Duration {
    if sorted_durations.is_empty() {
        return Duration::ZERO;
    }
    let idx = ((sorted_durations.len() as f64 * pct / 100.0).ceil() as usize)
        .saturating_sub(1)
        .min(sorted_durations.len() - 1);
    sorted_durations[idx]
}

// ---------------------------------------------------------------------------
// Performance target tests
// ---------------------------------------------------------------------------

/// Validate that the in-memory pipeline (weight → rank → compile) completes
/// well under the 500ms p95 target. Since we're measuring only the Rust
/// code path (no DB, no LLM), this should be sub-millisecond.
#[test]
fn pipeline_p95_under_500ms() {
    let iterations = 100;
    let mut durations: Vec<Duration> = (0..iterations)
        .map(|_| run_pipeline_in_memory(20, 5))
        .collect();

    durations.sort();

    let p95 = percentile(&durations, 95.0);
    let p99 = percentile(&durations, 99.0);

    println!(
        "Pipeline p50={:?}, p95={:?}, p99={:?}",
        percentile(&durations, 50.0),
        p95,
        p99,
    );

    assert!(
        p95 < Duration::from_millis(500),
        "p95 latency {:?} exceeds 500ms target",
        p95
    );
    assert!(
        p99 < Duration::from_millis(1000),
        "p99 latency {:?} exceeds 1000ms target",
        p99
    );
}

/// Validate that classification override (simulating loom_learn's sync path)
/// completes under 100ms. The actual loom_learn stores an episode and returns
/// immediately — this tests the overhead of the sync code path.
#[test]
fn classify_override_under_100ms() {
    let iterations = 100;
    let mut durations: Vec<Duration> = Vec::with_capacity(iterations);

    for _ in 0..iterations {
        let start = Instant::now();
        let _output = classify::apply_override(TaskClass::Debug);
        durations.push(start.elapsed());
    }

    durations.sort();
    let p99 = percentile(&durations, 99.0);

    assert!(
        p99 < Duration::from_millis(100),
        "classify override p99 {:?} exceeds 100ms",
        p99
    );
}

/// Validate that ranking 50 candidates completes quickly.
#[test]
fn ranking_50_candidates_under_10ms() {
    let candidates = mock_candidates(50);
    let task_class = TaskClass::Architecture;
    let weighted = weight::apply_weights(candidates, &task_class);

    let start = Instant::now();
    let _ranked = rank::rank_candidates(weighted);
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_millis(10),
        "ranking 50 candidates took {:?}, expected < 10ms",
        elapsed
    );
}

/// Validate that compilation with 20 warm candidates and 5 hot items
/// completes quickly.
#[test]
fn compilation_under_50ms() {
    let candidates = mock_candidates(20);
    let task_class = TaskClass::Architecture;
    let weighted = weight::apply_weights(candidates, &task_class);
    let ranked = rank::rank_candidates(weighted);
    let hot_items = mock_hot_tier_items(5);

    let start = Instant::now();
    let input = CompilationInput {
        namespace: "test".to_string(),
        task_class,
        target_model: "claude".to_string(),
        format: OutputFormat::Structured,
        warm_tier_budget: 3000,
        hot_tier_items: hot_items,
        ranked_candidates: ranked,
    };
    let _result = compile::compile_package(input);
    let elapsed = start.elapsed();

    assert!(
        elapsed < Duration::from_millis(50),
        "compilation took {:?}, expected < 50ms",
        elapsed
    );
}

/// Validate 10 concurrent pipeline executions complete within target.
#[tokio::test]
async fn ten_concurrent_pipeline_executions() {
    let mut handles = Vec::new();

    for _ in 0..10 {
        handles.push(tokio::spawn(async move {
            run_pipeline_in_memory(20, 5)
        }));
    }

    let mut durations = Vec::new();
    for handle in handles {
        let duration = handle.await.expect("task should complete");
        durations.push(duration);
    }

    durations.sort();
    let max = durations.last().copied().unwrap_or(Duration::ZERO);
    let p95 = percentile(&durations, 95.0);

    println!(
        "10 concurrent: max={:?}, p95={:?}",
        max, p95
    );

    assert!(
        max < Duration::from_millis(500),
        "max concurrent latency {:?} exceeds 500ms",
        max
    );
}

/// Validate that 100 pipeline executions can complete within 60 seconds
/// (simulating 100 episodes per minute ingestion rate for the Rust code
/// path only — no LLM or DB overhead).
#[test]
fn hundred_pipelines_per_minute() {
    let start = Instant::now();

    for _ in 0..100 {
        let _ = run_pipeline_in_memory(10, 3);
    }

    let total = start.elapsed();

    println!("100 pipeline executions: {:?}", total);

    assert!(
        total < Duration::from_secs(60),
        "100 pipeline executions took {:?}, exceeds 60s target",
        total
    );
}

/// Validate percentile calculation correctness.
#[test]
fn percentile_calculation_correctness() {
    let durations: Vec<Duration> = (1..=100)
        .map(|i| Duration::from_millis(i))
        .collect();

    let p50 = percentile(&durations, 50.0);
    let p95 = percentile(&durations, 95.0);
    let p99 = percentile(&durations, 99.0);

    // p50 should be around 50ms
    assert!(
        p50 >= Duration::from_millis(49) && p50 <= Duration::from_millis(51),
        "p50 should be ~50ms, got {:?}",
        p50
    );

    // p95 should be around 95ms
    assert!(
        p95 >= Duration::from_millis(94) && p95 <= Duration::from_millis(96),
        "p95 should be ~95ms, got {:?}",
        p95
    );

    // p99 should be around 99ms
    assert!(
        p99 >= Duration::from_millis(98) && p99 <= Duration::from_millis(100),
        "p99 should be ~99ms, got {:?}",
        p99
    );
}

/// Validate percentile with empty input.
#[test]
fn percentile_empty_returns_zero() {
    let empty: Vec<Duration> = vec![];
    assert_eq!(percentile(&empty, 50.0), Duration::ZERO);
}

/// Validate that weight application is fast for large candidate sets.
#[test]
fn weight_application_scales_linearly() {
    let small_candidates = mock_candidates(10);
    let large_candidates = mock_candidates(100);
    let task_class = TaskClass::Debug;

    let start_small = Instant::now();
    let _small = weight::apply_weights(small_candidates, &task_class);
    let small_time = start_small.elapsed();

    let start_large = Instant::now();
    let _large = weight::apply_weights(large_candidates, &task_class);
    let large_time = start_large.elapsed();

    // Large should be no more than 20x slower than small (generous bound).
    // In practice it should be ~10x since it's 10x more candidates.
    assert!(
        large_time < small_time * 20 + Duration::from_millis(1),
        "weight application doesn't scale linearly: small={:?}, large={:?}",
        small_time,
        large_time
    );
}
