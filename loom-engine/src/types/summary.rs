//! Knowledge summary types for consolidation pipeline.
//!
//! Summaries are derived abstractions synthesized from clusters of facts.
//! They sit between facts and procedures in the authority hierarchy and
//! participate in ranking and retrieval like all memory artifacts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A knowledge summary record matching the `loom_summaries` table schema.
///
/// Summaries are produced by the consolidation pipeline when it identifies
/// clusters of related facts about the same entity. They represent compressed,
/// generalized knowledge that is traceable back to source facts.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct KnowledgeSummary {
    /// Unique summary identifier.
    pub id: Uuid,
    /// Namespace isolation boundary.
    pub namespace: String,
    /// The entity this summary describes.
    pub subject_entity_id: Uuid,
    /// The consolidated summary text (single paragraph).
    pub summary_text: String,
    /// UUIDs of facts this summary was synthesized from.
    pub source_facts: Vec<Uuid>,
    /// Denormalized count of source facts for query optimization.
    pub fact_count: i32,
    /// Reliability classification: 'extracted' or 'confirmed'.
    pub evidence_status: String,
    /// Whether any source fact has evidence_status='sole_source_flagged'.
    pub contains_sole_source: bool,
    /// Model used to synthesize this summary (e.g., 'qwen2.5:32b').
    pub synthesis_model: String,
    /// Version of the consolidation prompt (e.g., 'consolidation_v1').
    pub synthesis_prompt_ver: String,
    /// Tier placement: 'hot' or 'warm'.
    pub tier: String,
    /// Salience score for ranking.
    pub salience_score: f32,
    /// When this summary was created.
    pub created_at: DateTime<Utc>,
    /// When this summary was last re-synthesized.
    pub refreshed_at: DateTime<Utc>,
    /// Set when a source fact is superseded; summary is stale until re-consolidated.
    pub invalidated_at: Option<DateTime<Utc>>,
    /// Soft-delete timestamp.
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Summary serving state matching the `loom_summary_state` table schema.
///
/// Derived and recomputable. Tracks embedding, token budget, and access
/// patterns for retrieval ranking, same pattern as FactState.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct SummaryState {
    /// References `loom_summaries.id`.
    pub summary_id: Uuid,
    /// 768-dimension embedding from nomic-embed-text.
    #[serde(skip)]
    pub embedding: Option<pgvector::Vector>,
    /// Estimated token count for budget accounting.
    pub token_count: Option<i32>,
    /// Number of times this summary was accessed in compilations.
    pub access_count: Option<i32>,
    /// Last time this summary was accessed.
    pub last_accessed: Option<DateTime<Utc>>,
    /// Last state update timestamp.
    pub updated_at: DateTime<Utc>,
}

/// The synthesis response from the consolidation pipeline's LLM call.
///
/// This is the structured output Ollama returns when asked to consolidate
/// a cluster of facts. The coverage map validates that all claims trace
/// to source facts, preventing hallucination.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesisResponse {
    /// The consolidated summary paragraph.
    pub summary_text: String,
    /// Mapping of claims to source fact IDs for traceability.
    pub coverage: Vec<CoverageItem>,
    /// Conflicts detected among source facts during synthesis.
    pub conflicts_detected: Vec<ConflictItem>,
}

/// A single claim in a summary with its source facts.
///
/// Used in SynthesisResponse to prove that every claim in the summary
/// is traceable to at least one source fact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageItem {
    /// Short description of the claim being made.
    pub claim: String,
    /// UUIDs of facts that support this claim.
    pub source_fact_ids: Vec<Uuid>,
}

/// A conflict detected during fact consolidation.
///
/// When the consolidation model identifies contradictions or temporal
/// inconsistencies in source facts, it records them here for manual
/// review rather than resolving them automatically.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictItem {
    /// Description of the conflict (e.g., "Fact A says X, Fact B says Y").
    pub description: String,
    /// UUIDs of the conflicting facts.
    pub fact_ids: Vec<Uuid>,
}

/// Result of a consolidation operation on a single cluster.
pub enum ConsolidationResult {
    /// A new summary was created for this entity.
    Created(Uuid),
    /// An existing summary was refreshed with new source facts.
    Refreshed(Uuid),
}
