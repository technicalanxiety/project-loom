//! Compilation types for context package assembly.
//!
//! The context compiler merges, deduplicates, ranks, and trims memory
//! candidates into a final context package for AI queries.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A compiled context package ready for delivery to an AI model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledPackage {
    /// The assembled context content (XML structured or JSON compact).
    pub context_package: String,
    /// Total token count of the compiled package.
    pub token_count: i32,
    /// Unique identifier for this compilation (for audit trail).
    pub compilation_id: Uuid,
    /// Output format used.
    pub format: OutputFormat,
}

/// Output format for compiled context packages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputFormat {
    /// XML-like tags: loom, identity, project, knowledge, episodes, patterns.
    Structured,
    /// JSON object with ns, task, identity, facts, recent, patterns fields.
    Compact,
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Structured => "structured",
            Self::Compact => "compact",
        };
        write!(f, "{s}")
    }
}

/// Score breakdown for a ranked memory candidate.
///
/// The four dimensions are weighted: relevance (0.40), recency (0.25),
/// stability (0.20), provenance (0.15).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankingScore {
    /// Semantic similarity to the query (weight 0.40).
    pub relevance: f64,
    /// How recently the memory was created or accessed (weight 0.25).
    pub recency: f64,
    /// How stable/consistent the memory is over time (weight 0.20).
    pub stability: f64,
    /// Strength of evidence provenance (weight 0.15).
    pub provenance: f64,
}

impl RankingScore {
    /// Compute the weighted composite score.
    pub fn composite(&self) -> f64 {
        self.relevance * 0.40
            + self.recency * 0.25
            + self.stability * 0.20
            + self.provenance * 0.15
    }
}
