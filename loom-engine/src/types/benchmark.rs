//! Benchmark evaluation types for A/B/C condition comparison.
//!
//! These types support the benchmark evaluation protocol (Requirement 47)
//! which measures precision, token reduction, and task success rate across
//! three conditions: A (no memory), B (episode-only), C (full Loom).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A benchmark task definition with expected results for precision measurement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkTask {
    /// Human-readable task name.
    pub name: String,
    /// The query to execute against the pipeline.
    pub query: String,
    /// Target namespace for retrieval.
    pub namespace: String,
    /// Entity names expected in the retrieval results.
    pub expected_entities: Vec<String>,
    /// Fact predicates expected in the retrieval results.
    pub expected_facts: Vec<String>,
}

/// The three benchmark evaluation conditions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BenchmarkCondition {
    /// Condition A: No memory (baseline).
    A,
    /// Condition B: Episode-only retrieval.
    B,
    /// Condition C: Full Loom pipeline (entities + facts + episodes).
    C,
}

impl std::fmt::Display for BenchmarkCondition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::A => write!(f, "A"),
            Self::B => write!(f, "B"),
            Self::C => write!(f, "C"),
        }
    }
}

/// A single benchmark result row matching the `loom_benchmark_results` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    /// Result identifier.
    pub id: Uuid,
    /// Parent benchmark run identifier.
    pub run_id: Uuid,
    /// Name of the benchmark task.
    pub task_name: String,
    /// Which condition was tested (A, B, or C).
    pub condition: String,
    /// Precision: relevant retrieved / total retrieved.
    pub precision: f64,
    /// Number of tokens in the compiled context.
    pub token_count: i32,
    /// Whether the task was considered successful.
    pub task_success: bool,
    /// End-to-end latency in milliseconds.
    pub latency_ms: i32,
    /// Additional details as JSON.
    pub details: serde_json::Value,
    /// When this result was recorded.
    pub created_at: DateTime<Utc>,
}

/// Summary of a benchmark run for API list responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkRunSummary {
    /// Run identifier.
    pub id: Uuid,
    /// Human-readable run name.
    pub name: String,
    /// When the run was created.
    pub created_at: DateTime<Utc>,
    /// Current status: pending, running, completed, failed.
    pub status: String,
}

/// Aggregated metrics for a single condition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConditionSummary {
    /// Average precision across all tasks.
    pub avg_precision: f64,
    /// Average token count across all tasks.
    pub avg_token_count: f64,
    /// Fraction of tasks that succeeded.
    pub success_rate: f64,
    /// Average latency in milliseconds.
    pub avg_latency_ms: f64,
}

/// Full benchmark comparison for the dashboard view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkComparison {
    /// The benchmark run metadata.
    pub run: BenchmarkRunSummary,
    /// All individual task results.
    pub results: Vec<BenchmarkResult>,
    /// Per-condition aggregated summaries.
    pub summary: BenchmarkComparisonSummary,
}

/// The three condition summaries grouped together.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkComparisonSummary {
    /// Condition A (no memory) summary.
    pub condition_a: ConditionSummary,
    /// Condition B (episode-only) summary.
    pub condition_b: ConditionSummary,
    /// Condition C (full Loom) summary.
    pub condition_c: ConditionSummary,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benchmark_condition_display() {
        assert_eq!(BenchmarkCondition::A.to_string(), "A");
        assert_eq!(BenchmarkCondition::B.to_string(), "B");
        assert_eq!(BenchmarkCondition::C.to_string(), "C");
    }

    #[test]
    fn benchmark_condition_serializes() {
        let json = serde_json::to_string(&BenchmarkCondition::C).unwrap();
        assert_eq!(json, "\"C\"");
    }

    #[test]
    fn condition_summary_serializes() {
        let summary = ConditionSummary {
            avg_precision: 0.85,
            avg_token_count: 1200.0,
            success_rate: 0.9,
            avg_latency_ms: 150.0,
        };
        let json = serde_json::to_string(&summary).unwrap();
        assert!(json.contains("0.85"));
        assert!(json.contains("1200"));
    }

    #[test]
    fn benchmark_task_serializes() {
        let task = BenchmarkTask {
            name: "test".to_string(),
            query: "query".to_string(),
            namespace: "ns".to_string(),
            expected_entities: vec!["Entity1".to_string()],
            expected_facts: vec!["uses".to_string()],
        };
        let json = serde_json::to_string(&task).unwrap();
        assert!(json.contains("Entity1"));
    }

    #[test]
    fn benchmark_run_summary_serializes() {
        let run = BenchmarkRunSummary {
            id: Uuid::nil(),
            name: "test run".to_string(),
            created_at: Utc::now(),
            status: "completed".to_string(),
        };
        let json = serde_json::to_string(&run).unwrap();
        assert!(json.contains("test run"));
        assert!(json.contains("completed"));
    }
}
