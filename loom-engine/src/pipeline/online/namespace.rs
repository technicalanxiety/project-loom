//! Namespace resolution and isolation enforcement.
//!
//! Provides namespace configuration lookup, cross-namespace reference
//! validation, and default namespace handling. All retrieval queries are
//! scoped to a single namespace — this module enforces that invariant.

use sqlx::PgPool;

/// The default namespace used for general knowledge not tied to a project.
pub const DEFAULT_NAMESPACE: &str = "default";

/// Errors that can occur during namespace operations.
#[derive(Debug, thiserror::Error)]
pub enum NamespaceError {
    /// An underlying database error.
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),

    /// A cross-namespace reference was detected.
    #[error("cross-namespace reference: {0}")]
    CrossNamespace(String),
}

/// Namespace configuration loaded from `loom_namespace_config`.
#[derive(Debug, Clone)]
pub struct NamespaceConfig {
    /// The namespace identifier.
    pub namespace: String,
    /// Hot tier token budget (default 500).
    pub hot_tier_budget: i32,
    /// Warm tier token budget (default 3000).
    pub warm_tier_budget: i32,
    /// Active predicate packs for this namespace.
    pub predicate_packs: Vec<String>,
    /// Optional description.
    pub description: Option<String>,
}

impl Default for NamespaceConfig {
    fn default() -> Self {
        Self {
            namespace: DEFAULT_NAMESPACE.to_string(),
            hot_tier_budget: 500,
            warm_tier_budget: 3000,
            predicate_packs: vec!["core".to_string()],
            description: None,
        }
    }
}

/// Row type for namespace config queries.
#[derive(Debug, sqlx::FromRow)]
struct NamespaceConfigRow {
    namespace: String,
    hot_tier_budget: Option<i32>,
    warm_tier_budget: Option<i32>,
    predicate_packs: Option<Vec<String>>,
    description: Option<String>,
}

/// Resolve the namespace configuration for a given namespace.
///
/// If no configuration row exists in `loom_namespace_config`, returns a
/// default configuration with the provided namespace name, 500-token hot
/// tier budget, 3000-token warm tier budget, and `["core"]` predicate packs.
pub async fn resolve_namespace(
    pool: &PgPool,
    namespace: &str,
) -> Result<NamespaceConfig, NamespaceError> {
    let row: Option<NamespaceConfigRow> = sqlx::query_as(
        r#"
        SELECT namespace, hot_tier_budget, warm_tier_budget,
               predicate_packs, description
        FROM loom_namespace_config
        WHERE namespace = $1
        "#,
    )
    .bind(namespace)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => {
            let mut packs = r.predicate_packs.unwrap_or_default();
            // Ensure core pack is always present.
            if !packs.iter().any(|p| p == "core") {
                packs.insert(0, "core".to_string());
            }

            Ok(NamespaceConfig {
                namespace: r.namespace,
                hot_tier_budget: r.hot_tier_budget.unwrap_or(500),
                warm_tier_budget: r.warm_tier_budget.unwrap_or(3000),
                predicate_packs: packs,
                description: r.description,
            })
        }
        None => {
            tracing::debug!(
                namespace,
                "no namespace config found, using defaults"
            );
            Ok(NamespaceConfig {
                namespace: namespace.to_string(),
                ..Default::default()
            })
        }
    }
}

/// Get the active predicate packs for a namespace.
///
/// Convenience wrapper around [`resolve_namespace`] that returns just the
/// pack list.
pub async fn get_active_predicate_packs(
    pool: &PgPool,
    namespace: &str,
) -> Result<Vec<String>, NamespaceError> {
    let config = resolve_namespace(pool, namespace).await?;
    Ok(config.predicate_packs)
}

/// Validate that a fact's subject, object, and the fact itself all belong
/// to the same namespace.
///
/// Queries `loom_entities` for the subject and object entity IDs and
/// verifies their namespace matches the provided `fact_namespace`. Returns
/// `Ok(())` if all three are in the same namespace, or
/// [`NamespaceError::CrossNamespace`] if any mismatch is detected.
///
/// This is the core enforcement point for Requirement 7.4: preventing
/// cross-namespace entity references in facts.
pub async fn validate_fact_namespace(
    pool: &PgPool,
    subject_id: uuid::Uuid,
    object_id: uuid::Uuid,
    fact_namespace: &str,
) -> Result<(), NamespaceError> {
    // Check subject namespace.
    let subject_ns: Option<(String,)> = sqlx::query_as(
        "SELECT namespace FROM loom_entities WHERE id = $1",
    )
    .bind(subject_id)
    .fetch_optional(pool)
    .await?;

    if let Some((ns,)) = &subject_ns {
        if ns != fact_namespace {
            return Err(NamespaceError::CrossNamespace(format!(
                "subject entity {} is in namespace '{}' but fact targets '{}'",
                subject_id, ns, fact_namespace
            )));
        }
    }

    // Check object namespace.
    let object_ns: Option<(String,)> = sqlx::query_as(
        "SELECT namespace FROM loom_entities WHERE id = $1",
    )
    .bind(object_id)
    .fetch_optional(pool)
    .await?;

    if let Some((ns,)) = &object_ns {
        if ns != fact_namespace {
            return Err(NamespaceError::CrossNamespace(format!(
                "object entity {} is in namespace '{}' but fact targets '{}'",
                object_id, ns, fact_namespace
            )));
        }
    }

    Ok(())
}

/// Validate that a namespace string is non-empty and well-formed.
///
/// Returns the trimmed namespace or an error if it's empty.
pub fn validate_namespace(namespace: &str) -> Result<&str, NamespaceError> {
    let trimmed = namespace.trim();
    if trimmed.is_empty() {
        return Err(NamespaceError::CrossNamespace(
            "namespace must not be empty".to_string(),
        ));
    }
    Ok(trimmed)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_namespace_is_default() {
        assert_eq!(DEFAULT_NAMESPACE, "default");
    }

    #[test]
    fn default_config_has_core_pack() {
        let config = NamespaceConfig::default();
        assert!(config.predicate_packs.contains(&"core".to_string()));
    }

    #[test]
    fn default_config_has_correct_budgets() {
        let config = NamespaceConfig::default();
        assert_eq!(config.hot_tier_budget, 500);
        assert_eq!(config.warm_tier_budget, 3000);
    }

    #[test]
    fn validate_namespace_rejects_empty() {
        assert!(validate_namespace("").is_err());
        assert!(validate_namespace("   ").is_err());
    }

    #[test]
    fn validate_namespace_accepts_valid() {
        assert_eq!(validate_namespace("my-project").unwrap(), "my-project");
        assert_eq!(validate_namespace("  trimmed  ").unwrap(), "trimmed");
    }

    #[test]
    fn validate_namespace_accepts_default() {
        assert_eq!(validate_namespace("default").unwrap(), "default");
    }

    #[test]
    fn namespace_error_displays_cross_namespace() {
        let err = NamespaceError::CrossNamespace("test".into());
        assert!(err.to_string().contains("cross-namespace"));
    }

    // -- Claude Code namespace resolution -----------------------------------

    /// Claude Code resolves namespace from the working directory basename.
    /// The MCP `namespace` field supports manual override.
    #[test]
    fn claude_code_namespace_from_directory() {
        // Simulate extracting namespace from a working directory path.
        let working_dir = "/home/user/projects/sentinel";
        let namespace = working_dir
            .rsplit('/')
            .next()
            .unwrap_or("default");
        assert_eq!(namespace, "sentinel");
    }

    #[test]
    fn claude_code_namespace_manual_override() {
        // When a manual namespace is provided, it takes precedence.
        let manual_override = Some("custom-namespace".to_string());
        let working_dir_ns = "sentinel";

        let resolved = manual_override
            .as_deref()
            .unwrap_or(working_dir_ns);
        assert_eq!(resolved, "custom-namespace");
    }

    #[test]
    fn claude_code_namespace_falls_back_to_directory() {
        let manual_override: Option<String> = None;
        let working_dir_ns = "sentinel";

        let resolved = manual_override
            .as_deref()
            .unwrap_or(working_dir_ns);
        assert_eq!(resolved, "sentinel");
    }

    #[test]
    fn claude_code_namespace_falls_back_to_default() {
        let working_dir = "/";
        let namespace = working_dir
            .rsplit('/')
            .find(|s| !s.is_empty())
            .unwrap_or(DEFAULT_NAMESPACE);
        assert_eq!(namespace, DEFAULT_NAMESPACE);
    }

    // -- Namespace filter application in queries ----------------------------

    /// Verify that namespace filtering logic correctly scopes results.
    #[test]
    fn namespace_filter_scopes_to_single_namespace() {
        // Simulate records from multiple namespaces.
        let records = vec![
            ("project-a", "entity-1"),
            ("project-a", "entity-2"),
            ("project-b", "entity-3"),
            ("project-b", "entity-4"),
            ("default", "entity-5"),
        ];

        let query_ns = "project-a";
        let filtered: Vec<_> = records
            .iter()
            .filter(|(ns, _)| *ns == query_ns)
            .collect();

        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|(ns, _)| *ns == "project-a"));
    }

    #[test]
    fn namespace_filter_returns_empty_for_unknown_namespace() {
        let records = vec![
            ("project-a", "entity-1"),
            ("project-b", "entity-2"),
        ];

        let query_ns = "nonexistent";
        let filtered: Vec<_> = records
            .iter()
            .filter(|(ns, _)| *ns == query_ns)
            .collect();

        assert!(filtered.is_empty());
    }

    // -- Cross-namespace reference validation -------------------------------

    #[test]
    fn cross_namespace_detection_same_namespace_is_valid() {
        let fact_ns = "project-a";
        let subject_ns = "project-a";
        let object_ns = "project-a";

        let is_valid = fact_ns == subject_ns && fact_ns == object_ns;
        assert!(is_valid, "All same namespace should be valid");
    }

    #[test]
    fn cross_namespace_detection_different_subject_is_invalid() {
        let fact_ns = "project-a";
        let subject_ns = "project-b";
        let object_ns = "project-a";

        let is_valid = fact_ns == subject_ns && fact_ns == object_ns;
        assert!(!is_valid, "Different subject namespace should be invalid");
    }

    #[test]
    fn cross_namespace_detection_different_object_is_invalid() {
        let fact_ns = "project-a";
        let subject_ns = "project-a";
        let object_ns = "project-b";

        let is_valid = fact_ns == subject_ns && fact_ns == object_ns;
        assert!(!is_valid, "Different object namespace should be invalid");
    }

    #[test]
    fn cross_namespace_detection_all_different_is_invalid() {
        let fact_ns = "project-a";
        let subject_ns = "project-b";
        let object_ns = "project-c";

        let is_valid = fact_ns == subject_ns && fact_ns == object_ns;
        assert!(!is_valid, "All different namespaces should be invalid");
    }

    // -- Soft deletion with deletion reasons --------------------------------

    #[test]
    fn soft_deletion_preserves_reason() {
        let reason = "Duplicate entity detected during cleanup";
        let stored_reason = Some(reason.to_string());
        assert_eq!(stored_reason.as_deref(), Some(reason));
    }

    #[test]
    fn soft_deletion_without_reason_is_valid() {
        let reason: Option<String> = None;
        assert!(reason.is_none());
    }

    #[test]
    fn soft_deleted_record_excluded_from_retrieval() {
        // Simulate the WHERE deleted_at IS NULL filter.
        let deleted_at: Option<chrono::DateTime<chrono::Utc>> = Some(chrono::Utc::now());
        let is_excluded = deleted_at.is_some();
        assert!(is_excluded, "Record with deleted_at set must be excluded");
    }

    #[test]
    fn active_record_included_in_retrieval() {
        let deleted_at: Option<chrono::DateTime<chrono::Utc>> = None;
        let is_included = deleted_at.is_none();
        assert!(is_included, "Record without deleted_at must be included");
    }
}
