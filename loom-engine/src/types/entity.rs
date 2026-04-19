//! Entity types for graph nodes representing real-world concepts.
//!
//! Entities are extracted from episodes and represent people, organizations,
//! projects, services, technologies, patterns, environments, documents,
//! metrics, and decisions. Entity state (embedding, tier, salience) is
//! separated into a derived table that can be recomputed.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The 10 constrained entity types matching the CHECK constraint on `loom_entities`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntityType {
    /// A human individual.
    Person,
    /// A company, team, or group.
    Organization,
    /// A software project or initiative.
    Project,
    /// A running service or API.
    Service,
    /// A language, framework, or tool.
    Technology,
    /// A design or architectural pattern.
    Pattern,
    /// A deployment environment (dev, staging, prod).
    Environment,
    /// A document, spec, or reference.
    Document,
    /// A measurable quantity or KPI.
    Metric,
    /// An architectural or design decision.
    Decision,
}

impl std::fmt::Display for EntityType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Person => "person",
            Self::Organization => "organization",
            Self::Project => "project",
            Self::Service => "service",
            Self::Technology => "technology",
            Self::Pattern => "pattern",
            Self::Environment => "environment",
            Self::Document => "document",
            Self::Metric => "metric",
            Self::Decision => "decision",
        };
        write!(f, "{s}")
    }
}

impl std::str::FromStr for EntityType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "person" => Ok(Self::Person),
            "organization" => Ok(Self::Organization),
            "project" => Ok(Self::Project),
            "service" => Ok(Self::Service),
            "technology" => Ok(Self::Technology),
            "pattern" => Ok(Self::Pattern),
            "environment" => Ok(Self::Environment),
            "document" => Ok(Self::Document),
            "metric" => Ok(Self::Metric),
            "decision" => Ok(Self::Decision),
            other => Err(format!("unknown entity type: {other}")),
        }
    }
}

/// An entity record matching the `loom_entities` table schema.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Entity {
    /// Unique entity identifier.
    pub id: Uuid,
    /// Most specific common name.
    pub name: String,
    /// Constrained entity type (stored as TEXT in DB).
    pub entity_type: String,
    /// Namespace isolation boundary.
    pub namespace: String,
    /// Flexible properties including aliases array (JSONB).
    pub properties: Option<serde_json::Value>,
    /// When the entity was created.
    pub created_at: Option<DateTime<Utc>>,
    /// Which episodes mentioned this entity.
    pub source_episodes: Option<Vec<Uuid>>,
    /// Soft-delete timestamp.
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Entity serving state matching the `loom_entity_state` table schema.
///
/// Derived and recomputable. Tracks embedding, tier placement, salience,
/// and access patterns for retrieval ranking.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct EntityState {
    /// References `loom_entities.id`.
    pub entity_id: Uuid,
    /// 768-dimension embedding from nomic-embed-text.
    #[serde(skip)]
    pub embedding: Option<pgvector::Vector>,
    /// Generated entity summary.
    pub summary: Option<String>,
    /// Tier placement: "hot" or "warm".
    pub tier: Option<String>,
    /// Salience score for ranking.
    pub salience_score: Option<f64>,
    /// Number of times accessed in compilations.
    pub access_count: Option<i32>,
    /// Last time this entity was accessed.
    pub last_accessed: Option<DateTime<Utc>>,
    /// Whether the user pinned this entity to hot tier.
    pub pinned: Option<bool>,
    /// Last state update timestamp.
    pub updated_at: Option<DateTime<Utc>>,
}

/// An entity extracted from an LLM response before resolution.
///
/// This is the raw extraction output that feeds into the 3-pass
/// resolution algorithm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedEntity {
    /// Entity name as extracted by the LLM.
    pub name: String,
    /// Entity type as classified by the LLM.
    pub entity_type: String,
    /// Alias names found in the episode.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Additional properties extracted by the LLM.
    #[serde(default)]
    pub properties: serde_json::Value,
}

/// Result of the 3-pass entity resolution algorithm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolutionResult {
    /// The resolved entity identifier.
    pub entity_id: Uuid,
    /// Resolution method used: "exact", "alias", "semantic", or "new".
    pub method: String,
    /// Confidence score (1.0 for exact, 0.95 for alias, similarity for semantic).
    pub confidence: f64,
}

/// A resolution conflict matching the `loom_resolution_conflicts` table schema.
///
/// Created when semantic resolution finds multiple candidates within 0.03
/// similarity, requiring operator review.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ResolutionConflict {
    /// Unique conflict identifier.
    pub id: Uuid,
    /// The entity name that triggered the conflict.
    pub entity_name: String,
    /// The entity type of the ambiguous entity.
    pub entity_type: String,
    /// Namespace isolation boundary.
    pub namespace: String,
    /// Candidate matches as JSONB: [{id, name, score, method}].
    pub candidates: serde_json::Value,
    /// Whether an operator has resolved this conflict.
    pub resolved: Option<bool>,
    /// Resolution decision: "merged:id", "kept_separate", or "split:id1,id2".
    pub resolution: Option<String>,
    /// When the operator resolved this conflict.
    pub resolved_at: Option<DateTime<Utc>>,
    /// When the conflict was created.
    pub created_at: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- EntityType serde deserialization (Validates: Requirement 2.2) -------

    #[test]
    fn entity_type_deserializes_all_ten_types() {
        let types = [
            ("\"person\"", EntityType::Person),
            ("\"organization\"", EntityType::Organization),
            ("\"project\"", EntityType::Project),
            ("\"service\"", EntityType::Service),
            ("\"technology\"", EntityType::Technology),
            ("\"pattern\"", EntityType::Pattern),
            ("\"environment\"", EntityType::Environment),
            ("\"document\"", EntityType::Document),
            ("\"metric\"", EntityType::Metric),
            ("\"decision\"", EntityType::Decision),
        ];

        for (json_str, expected) in &types {
            let result: EntityType =
                serde_json::from_str(json_str).unwrap_or_else(|e| panic!("failed to deserialize {json_str}: {e}"));
            assert_eq!(&result, expected, "mismatch for {json_str}");
        }
    }

    #[test]
    fn entity_type_rejects_invalid_type() {
        let invalid_types = [
            "\"animal\"",
            "\"concept\"",
            "\"widget\"",
            "\"\"",
            "\"Person\"",  // case-sensitive: serde rename_all = lowercase
            "\"TECHNOLOGY\"",
        ];

        for json_str in &invalid_types {
            let result = serde_json::from_str::<EntityType>(json_str);
            assert!(
                result.is_err(),
                "should reject invalid entity type {json_str}, got: {:?}",
                result.unwrap()
            );
        }
    }

    #[test]
    fn entity_type_serializes_lowercase() {
        let cases = [
            (EntityType::Person, "\"person\""),
            (EntityType::Organization, "\"organization\""),
            (EntityType::Project, "\"project\""),
            (EntityType::Service, "\"service\""),
            (EntityType::Technology, "\"technology\""),
            (EntityType::Pattern, "\"pattern\""),
            (EntityType::Environment, "\"environment\""),
            (EntityType::Document, "\"document\""),
            (EntityType::Metric, "\"metric\""),
            (EntityType::Decision, "\"decision\""),
        ];

        for (variant, expected_json) in &cases {
            let json = serde_json::to_string(variant).expect("should serialize");
            assert_eq!(&json, expected_json, "mismatch for {:?}", variant);
        }
    }

    #[test]
    fn entity_type_display_matches_serde() {
        let variants = [
            EntityType::Person,
            EntityType::Organization,
            EntityType::Project,
            EntityType::Service,
            EntityType::Technology,
            EntityType::Pattern,
            EntityType::Environment,
            EntityType::Document,
            EntityType::Metric,
            EntityType::Decision,
        ];

        for variant in &variants {
            let display = variant.to_string();
            let serde_str = serde_json::to_string(variant)
                .expect("should serialize")
                .trim_matches('"')
                .to_string();
            assert_eq!(display, serde_str, "Display and serde mismatch for {:?}", variant);
        }
    }

    #[test]
    fn entity_type_from_str_roundtrips() {
        let type_strings = [
            "person", "organization", "project", "service", "technology",
            "pattern", "environment", "document", "metric", "decision",
        ];

        for s in &type_strings {
            let parsed: EntityType = s.parse().unwrap_or_else(|e| panic!("failed to parse {s}: {e}"));
            assert_eq!(&parsed.to_string(), s, "roundtrip failed for {s}");
        }
    }

    #[test]
    fn entity_type_from_str_rejects_invalid() {
        let invalid = ["animal", "concept", "", "Person", "TECHNOLOGY"];
        for s in &invalid {
            let result = s.parse::<EntityType>();
            assert!(result.is_err(), "should reject invalid type '{s}'");
        }
    }

    // -- ExtractedEntity serde deserialization --------------------------------

    #[test]
    fn extracted_entity_deserializes_with_all_fields() {
        let json = serde_json::json!({
            "name": "Rust",
            "entity_type": "technology",
            "aliases": ["rust-lang", "Rust Language"],
            "properties": {"version": "1.80"}
        });

        let entity: ExtractedEntity =
            serde_json::from_value(json).expect("should deserialize");
        assert_eq!(entity.name, "Rust");
        assert_eq!(entity.entity_type, "technology");
        assert_eq!(entity.aliases, vec!["rust-lang", "Rust Language"]);
        assert_eq!(entity.properties["version"], "1.80");
    }

    #[test]
    fn extracted_entity_defaults_aliases_and_properties() {
        let json = serde_json::json!({
            "name": "Alice",
            "entity_type": "person"
        });

        let entity: ExtractedEntity =
            serde_json::from_value(json).expect("should deserialize");
        assert_eq!(entity.name, "Alice");
        assert_eq!(entity.entity_type, "person");
        assert!(entity.aliases.is_empty());
        // properties defaults to null via serde default
        assert!(entity.properties.is_null());
    }

    // -- ResolutionResult serde roundtrip ------------------------------------

    #[test]
    fn resolution_result_serde_roundtrip() {
        let original = ResolutionResult {
            entity_id: Uuid::new_v4(),
            method: "semantic".to_string(),
            confidence: 0.95,
        };

        let json = serde_json::to_string(&original).expect("should serialize");
        let deserialized: ResolutionResult =
            serde_json::from_str(&json).expect("should deserialize");

        assert_eq!(deserialized.entity_id, original.entity_id);
        assert_eq!(deserialized.method, original.method);
        assert!((deserialized.confidence - original.confidence).abs() < f64::EPSILON);
    }

    // -- ResolutionConflict structure ----------------------------------------

    #[test]
    fn resolution_conflict_candidates_json_structure() {
        let conflict = ResolutionConflict {
            id: Uuid::new_v4(),
            entity_name: "APIM".to_string(),
            entity_type: "service".to_string(),
            namespace: "default".to_string(),
            candidates: serde_json::json!([
                {"id": Uuid::new_v4().to_string(), "name": "APIM", "score": 0.95, "method": "semantic"},
                {"id": Uuid::new_v4().to_string(), "name": "API Management", "score": 0.94, "method": "semantic"}
            ]),
            resolved: Some(false),
            resolution: None,
            resolved_at: None,
            created_at: None,
        };

        assert_eq!(conflict.entity_name, "APIM");
        assert!(conflict.candidates.is_array());
        assert_eq!(conflict.candidates.as_array().unwrap().len(), 2);
        assert_eq!(conflict.resolved, Some(false));
        assert!(conflict.resolution.is_none());
    }
}
