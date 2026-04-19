//! Fact types for temporal graph edges between entities.
//!
//! Facts are subject-predicate-object triples with provenance tracking,
//! temporal validity, and supersession chains. Fact state (embedding, tier,
//! salience) is separated into a derived table that can be recomputed.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A fact record matching the `loom_facts` table schema.
///
/// Each fact represents a relationship between two entities with temporal
/// validity and evidence provenance.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Fact {
    /// Unique fact identifier.
    pub id: Uuid,
    /// Subject entity identifier.
    pub subject_id: Uuid,
    /// Relationship type (canonical or custom predicate).
    pub predicate: String,
    /// Object entity identifier.
    pub object_id: Uuid,
    /// Namespace isolation boundary.
    pub namespace: String,
    /// When this fact became valid.
    pub valid_from: DateTime<Utc>,
    /// When this fact stopped being valid (NULL = currently valid).
    pub valid_until: Option<DateTime<Utc>>,
    /// Which episodes this fact was extracted from.
    pub source_episodes: Vec<Uuid>,
    /// Points to the newer contradicting fact.
    pub superseded_by: Option<Uuid>,
    /// Reliability classification.
    pub evidence_status: String,
    /// Evidence strength: "explicit" or "implied".
    pub evidence_strength: Option<String>,
    /// Flexible additional properties (JSONB).
    pub properties: Option<serde_json::Value>,
    /// When the fact was created.
    pub created_at: Option<DateTime<Utc>>,
    /// Soft-delete timestamp.
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Fact serving state matching the `loom_fact_state` table schema.
///
/// Derived and recomputable. Tracks embedding, tier placement, salience,
/// and access patterns for retrieval ranking.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct FactState {
    /// References `loom_facts.id`.
    pub fact_id: Uuid,
    /// 768-dimension embedding from nomic-embed-text.
    #[serde(skip)]
    pub embedding: Option<pgvector::Vector>,
    /// Salience score for ranking.
    pub salience_score: Option<f64>,
    /// Number of times accessed in compilations.
    pub access_count: Option<i32>,
    /// Last time this fact was accessed.
    pub last_accessed: Option<DateTime<Utc>>,
    /// Tier placement: "hot" or "warm".
    pub tier: Option<String>,
    /// Whether the user pinned this fact to hot tier.
    pub pinned: Option<bool>,
    /// Last state update timestamp.
    pub updated_at: Option<DateTime<Utc>>,
}

/// A fact extracted from an LLM response before predicate resolution.
///
/// This is the raw extraction output that feeds into predicate matching
/// and fact storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedFact {
    /// Subject entity name as extracted.
    pub subject: String,
    /// Predicate (relationship type) as extracted.
    pub predicate: String,
    /// Object entity name as extracted.
    pub object: String,
    /// Whether this uses a custom (non-canonical) predicate.
    #[serde(default)]
    pub custom: bool,
    /// Evidence strength classification.
    pub evidence_strength: Option<String>,
    /// Temporal markers for validity period.
    #[serde(default)]
    pub temporal_markers: Option<TemporalMarkers>,
}

/// Reliability classification for facts.
///
/// Maps to the CHECK constraint on `loom_facts.evidence_status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStatus {
    /// Explicitly stated by user.
    UserAsserted,
    /// Directly observed in episode.
    Observed,
    /// Extracted by LLM.
    Extracted,
    /// Inferred from other facts.
    Inferred,
    /// Promoted from candidate.
    Promoted,
    /// Marked as no longer relevant.
    Deprecated,
    /// Replaced by a newer fact.
    Superseded,
}

impl std::fmt::Display for EvidenceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::UserAsserted => "user_asserted",
            Self::Observed => "observed",
            Self::Extracted => "extracted",
            Self::Inferred => "inferred",
            Self::Promoted => "promoted",
            Self::Deprecated => "deprecated",
            Self::Superseded => "superseded",
        };
        write!(f, "{s}")
    }
}

/// Evidence strength classification for facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EvidenceStrength {
    /// Directly and clearly stated.
    Explicit,
    /// Indirectly suggested or inferred from context.
    Implied,
}

impl std::fmt::Display for EvidenceStrength {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Explicit => "explicit",
            Self::Implied => "implied",
        };
        write!(f, "{s}")
    }
}

/// Temporal markers extracted from episode content.
///
/// Used to set `valid_from` and `valid_until` on facts when the LLM
/// identifies time-bounded relationships.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalMarkers {
    /// When the relationship became valid.
    pub valid_from: Option<DateTime<Utc>>,
    /// When the relationship stopped being valid.
    pub valid_until: Option<DateTime<Utc>>,
}
