//! Episode types for the immutable evidence layer.
//!
//! Episodes represent raw interaction records from source systems (claude-code,
//! manual, github). They are the foundational evidence that all extracted
//! knowledge traces back to.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// An episode record matching the `loom_episodes` table schema.
///
/// Episodes are immutable once ingested. Derived fields (embedding, tags,
/// processed) are recomputable from canonical data.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Episode {
    /// Unique episode identifier.
    pub id: Uuid,
    /// Source system (e.g. "claude-code", "manual", "github").
    pub source: String,
    /// External system identifier.
    pub source_id: Option<String>,
    /// Deduplication key within source.
    pub source_event_id: Option<String>,
    /// Raw episode text content.
    pub content: String,
    /// SHA-256 content hash for deduplication.
    pub content_hash: String,
    /// When the interaction happened.
    pub occurred_at: DateTime<Utc>,
    /// When the episode was ingested.
    pub ingested_at: DateTime<Utc>,
    /// Namespace isolation boundary.
    pub namespace: String,
    /// Flexible source-specific metadata (JSONB).
    pub metadata: Option<serde_json::Value>,
    /// People involved in the interaction.
    pub participants: Option<Vec<String>>,
    /// Model used for entity/fact extraction.
    pub extraction_model: Option<String>,
    /// Model used for intent classification.
    pub classification_model: Option<String>,
    /// Per-episode extraction statistics (JSONB).
    pub extraction_metrics: Option<serde_json::Value>,
    /// Ingestion provenance class: `user_authored_seed`, `vendor_import`, or
    /// `live_mcp_capture`. Enforced by CHECK constraint in migration 015.
    /// Stored as String for row mapping; cross-reference [`crate::types::ingestion::IngestionMode`]
    /// for the typed enum used at API and ranking boundaries.
    pub ingestion_mode: String,
    /// Semantic version of the parser that produced this episode. Populated
    /// only when `ingestion_mode = 'vendor_import'`.
    pub parser_version: Option<String>,
    /// Vendor export schema version asserted against during parsing. Populated
    /// only when `ingestion_mode = 'vendor_import'`.
    pub parser_source_schema: Option<String>,
    /// 768-dimension embedding from nomic-embed-text.
    #[serde(skip)]
    pub embedding: Option<pgvector::Vector>,
    /// Derived tags.
    pub tags: Option<Vec<String>>,
    /// Whether the extraction pipeline has completed.
    pub processed: Option<bool>,
    /// Soft-delete timestamp.
    pub deleted_at: Option<DateTime<Utc>>,
    /// Reason for soft deletion.
    pub deletion_reason: Option<String>,
}

/// Structured extraction metrics stored as JSONB on each episode.
///
/// Tracks counts by resolution method, predicate type, evidence type,
/// processing time, and which model performed the extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionMetrics {
    // Entity counts by resolution method
    /// Entities extracted from the episode.
    pub extracted: i32,
    /// Entities resolved via exact name match.
    pub resolved_exact: i32,
    /// Entities resolved via alias match.
    pub resolved_alias: i32,
    /// Entities resolved via semantic similarity.
    pub resolved_semantic: i32,
    /// New entities created (no match found).
    pub new: i32,
    /// Entities flagged due to resolution conflicts.
    pub conflict_flagged: i32,

    // Fact counts by predicate type
    /// Total facts extracted from the episode.
    pub facts_extracted: i32,
    /// Facts using canonical predicates.
    pub canonical_predicate: i32,
    /// Facts using custom (non-canonical) predicates.
    pub custom_predicate: i32,

    // Evidence counts
    /// Facts with explicit evidence strength.
    pub explicit: i32,
    /// Facts with implied evidence strength.
    pub implied: i32,

    /// Total processing time in milliseconds.
    pub processing_time_ms: i64,
    /// Model identifier used for extraction.
    pub extraction_model: String,
}
