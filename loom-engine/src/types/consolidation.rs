//! Consolidation pipeline types for telemetry and logging.
//!
//! Tracks execution, diagnostics, and statistics from consolidation and
//! pruning runs for observability and dashboard telemetry.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A consolidation pipeline run record matching the `loom_consolidation_log` table.
///
/// Each consolidation or pruning run produces a log entry capturing what
/// happened, how long it took, and any errors encountered.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ConsolidationLog {
    /// Unique log entry identifier.
    pub id: Uuid,
    /// Namespace this run executed in.
    pub namespace: String,
    /// Type of run: 'consolidation' (synthesis) or 'pruning' (TTL-based cleanup).
    pub run_type: String,
    /// When the run started.
    pub started_at: DateTime<Utc>,
    /// When the run completed (NULL if still running).
    pub completed_at: Option<DateTime<Utc>>,
    /// Current status: 'running', 'completed', or 'failed'.
    pub status: String,
    /// Number of fact clusters identified (consolidation only).
    pub clusters_found: Option<i32>,
    /// Number of new summaries created (consolidation only).
    pub summaries_created: Option<i32>,
    /// Number of existing summaries refreshed (consolidation only).
    pub summaries_refreshed: Option<i32>,
    /// Number of stale procedure candidates deleted (pruning only).
    pub procedures_pruned: Option<i32>,
    /// Number of stale conflicts auto-resolved (pruning only).
    pub conflicts_resolved: Option<i32>,
    /// Number of long-invalidated summaries soft-deleted (pruning only).
    pub summaries_invalidated: Option<i32>,
    /// Error message if status='failed'.
    pub error_detail: Option<String>,
    /// Duration of the run in milliseconds.
    pub duration_ms: Option<i32>,
}

/// Status enum for consolidation log entries.
///
/// Maps to the CHECK constraint on `loom_consolidation_log.status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConsolidationStatus {
    /// Run is currently executing.
    Running,
    /// Run completed successfully.
    Completed,
    /// Run failed; check error_detail.
    Failed,
}

impl std::fmt::Display for ConsolidationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        };
        write!(f, "{s}")
    }
}

/// Type of consolidation run.
///
/// Maps to the CHECK constraint on `loom_consolidation_log.run_type`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConsolidationRunType {
    /// Synthesis phase: identify clusters and create/refresh summaries.
    Consolidation,
    /// Pruning phase: delete stale procedures, auto-resolve conflicts, clean summaries.
    Pruning,
}

impl std::fmt::Display for ConsolidationRunType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Consolidation => "consolidation",
            Self::Pruning => "pruning",
        };
        write!(f, "{s}")
    }
}
