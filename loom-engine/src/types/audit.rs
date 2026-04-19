//! Audit log types for compilation trace tracking.
//!
//! Every `loom_think` call writes a full trace to the audit log, capturing
//! classification, retrieval profiles, candidate decisions, token counts,
//! output format, and latency breakdown across pipeline stages.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// An audit log entry matching the `loom_audit_log` table schema.
///
/// Provides full observability into every compilation decision for the
/// dashboard trace viewer and retrieval quality metrics.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AuditLogEntry {
    /// Unique audit entry identifier.
    pub id: Uuid,
    /// When this entry was created.
    pub created_at: Option<DateTime<Utc>>,

    // Query context
    /// Classified intent (debug, architecture, compliance, writing, chat).
    pub task_class: String,
    /// Which namespace was queried.
    pub namespace: String,
    /// The original query text.
    pub query_text: Option<String>,
    /// Which model the context was compiled for.
    pub target_model: Option<String>,

    // Classification results
    /// Primary task class.
    pub primary_class: String,
    /// Secondary task class (if confidence gap < 0.3).
    pub secondary_class: Option<String>,
    /// Confidence score for primary class.
    pub primary_confidence: Option<f64>,
    /// Confidence score for secondary class.
    pub secondary_confidence: Option<f64>,

    // Retrieval execution
    /// Which retrieval profiles ran.
    pub profiles_executed: Option<Vec<String>>,
    /// Primary retrieval profile used.
    pub retrieval_profile: String,
    /// Total candidates from all profiles.
    pub candidates_found: Option<i32>,
    /// Candidates included in final package.
    pub candidates_selected: Option<i32>,
    /// Candidates excluded from final package.
    pub candidates_rejected: Option<i32>,

    // Candidate details (JSONB)
    /// Selected items with score breakdowns.
    pub selected_items: Option<serde_json::Value>,
    /// Rejected items with rejection reasons.
    pub rejected_items: Option<serde_json::Value>,

    // Output
    /// Total tokens in compiled package.
    pub compiled_tokens: Option<i32>,
    /// Output format: "structured" or "compact".
    pub output_format: Option<String>,

    // Latency breakdown (milliseconds)
    /// End-to-end latency.
    pub latency_total_ms: Option<i32>,
    /// Intent classification stage.
    pub latency_classify_ms: Option<i32>,
    /// Retrieval profile execution stage.
    pub latency_retrieve_ms: Option<i32>,
    /// Ranking and trimming stage.
    pub latency_rank_ms: Option<i32>,
    /// Package compilation stage.
    pub latency_compile_ms: Option<i32>,

    // Feedback
    /// Optional user feedback score.
    pub user_rating: Option<f64>,
}
