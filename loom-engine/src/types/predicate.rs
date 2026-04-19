//! Predicate types for the canonical relationship registry.
//!
//! Predicates define the relationship types used in fact triples. They are
//! organized into packs (domain vocabulary sets) that namespaces can opt into.
//! Custom predicates extracted by the LLM are tracked as candidates for
//! operator review and potential promotion.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A canonical predicate entry matching the `loom_predicates` table schema.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PredicateEntry {
    /// Canonical relationship name (primary key).
    pub predicate: String,
    /// Category: structural, temporal, decisional, operational, or regulatory.
    pub category: String,
    /// Which pack this predicate belongs to (e.g. "core", "grc").
    pub pack: String,
    /// Inverse predicate name (e.g. "uses" <-> "used_by").
    pub inverse: Option<String>,
    /// Human-readable description.
    pub description: Option<String>,
    /// Number of times this predicate has been used in facts.
    pub usage_count: Option<i32>,
    /// When this predicate was created.
    pub created_at: Option<DateTime<Utc>>,
}

/// A predicate pack matching the `loom_predicate_packs` table schema.
///
/// Packs group related predicates into domain vocabulary sets.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PredicatePack {
    /// Pack name (primary key, e.g. "core", "grc").
    pub pack: String,
    /// Human-readable description.
    pub description: Option<String>,
    /// When this pack was created.
    pub created_at: Option<DateTime<Utc>>,
}

/// A predicate candidate matching the `loom_predicate_candidates` table schema.
///
/// Tracks custom predicates extracted by the LLM that don't match any
/// canonical predicate. Flagged for operator review at 5 occurrences.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PredicateCandidate {
    /// Unique candidate identifier.
    pub id: Uuid,
    /// The custom predicate text.
    pub predicate: String,
    /// How many facts use this predicate.
    pub occurrences: Option<i32>,
    /// Sample fact IDs for operator review.
    pub example_facts: Option<Vec<Uuid>>,
    /// If mapped to an existing canonical predicate.
    pub mapped_to: Option<String>,
    /// Target pack when promoted to canonical status.
    pub promoted_to_pack: Option<String>,
    /// When this candidate was created.
    pub created_at: Option<DateTime<Utc>>,
    /// When the operator resolved this candidate.
    pub resolved_at: Option<DateTime<Utc>>,
}
