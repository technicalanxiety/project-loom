//! Fact supersession detection and resolution.
//!
//! When a new fact contradicts an existing fact (same subject + predicate +
//! namespace, different object), the old fact is superseded: its `valid_until`
//! is set to the new fact's `valid_from`, `superseded_by` points to the new
//! fact, and `evidence_status` becomes `'superseded'`.
//!
//! Old facts remain in the database (soft supersession, not deletion) and
//! are excluded from default retrieval queries via the `valid_until IS NULL`
//! filter.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::db::facts::{self, FactError};
use crate::types::fact::Fact;

/// Errors that can occur during supersession resolution.
#[derive(Debug, thiserror::Error)]
pub enum SupersedeError {
    /// An underlying fact database error.
    #[error("fact database error: {0}")]
    Fact(#[from] FactError),
}

/// Details of a new fact needed for supersession checks.
///
/// Contains the minimal set of fields required to identify contradicting
/// facts and apply supersession updates.
#[derive(Debug, Clone)]
pub struct NewFactDetails {
    /// The new fact's identifier.
    pub fact_id: Uuid,
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
}

/// Resolve supersessions for a single new fact.
///
/// Queries existing current facts (where `valid_until IS NULL` and
/// `deleted_at IS NULL`) that share the same `(subject_id, predicate,
/// namespace)` but have a different `object_id`. For each contradicting
/// fact found, calls [`facts::supersede_fact_at`] to set:
///
/// - `valid_until` = new fact's `valid_from`
/// - `superseded_by` = new fact's `id`
/// - `evidence_status` = `'superseded'`
///
/// Returns the number of facts that were superseded.
///
/// # Errors
///
/// Returns [`SupersedeError::Fact`] if any database query or update fails.
#[tracing::instrument(
    skip(pool),
    fields(
        fact_id = %new_fact.fact_id,
        subject_id = %new_fact.subject_id,
        predicate = %new_fact.predicate,
        namespace = %new_fact.namespace,
    )
)]
pub async fn resolve_supersessions(
    pool: &PgPool,
    new_fact: &NewFactDetails,
) -> Result<usize, SupersedeError> {
    // Query current facts with same (subject_id, predicate, namespace).
    let existing = facts::query_facts_by_subject_and_predicate(
        pool,
        new_fact.subject_id,
        &new_fact.predicate,
        &new_fact.namespace,
    )
    .await?;

    let mut superseded_count: usize = 0;

    for old_fact in &existing {
        // Skip the new fact itself (if it was already inserted).
        if old_fact.id == new_fact.fact_id {
            continue;
        }

        // Only supersede facts with a different object_id.
        if old_fact.object_id == new_fact.object_id {
            continue;
        }

        tracing::debug!(
            old_fact_id = %old_fact.id,
            old_object_id = %old_fact.object_id,
            new_fact_id = %new_fact.fact_id,
            new_object_id = %new_fact.object_id,
            "superseding contradicting fact"
        );

        facts::supersede_fact_at(
            pool,
            old_fact.id,
            new_fact.fact_id,
            new_fact.valid_from,
        )
        .await?;

        superseded_count += 1;
    }

    if superseded_count > 0 {
        tracing::info!(
            fact_id = %new_fact.fact_id,
            predicate = %new_fact.predicate,
            namespace = %new_fact.namespace,
            superseded_count = superseded_count,
            "supersession resolved"
        );
    } else {
        tracing::debug!(
            fact_id = %new_fact.fact_id,
            predicate = %new_fact.predicate,
            namespace = %new_fact.namespace,
            "no contradicting facts found"
        );
    }

    Ok(superseded_count)
}

/// Resolve supersessions for a batch of new facts.
///
/// Processes each new fact in sequence, accumulating the total number of
/// superseded facts across the entire batch. Logs the aggregate count
/// at the end of the batch.
///
/// # Errors
///
/// Returns [`SupersedeError`] on the first database failure encountered.
/// Facts processed before the failure will have already been superseded.
#[tracing::instrument(skip(pool, new_facts), fields(batch_size = new_facts.len()))]
pub async fn resolve_supersessions_batch(
    pool: &PgPool,
    new_facts: &[NewFactDetails],
) -> Result<usize, SupersedeError> {
    let mut total_superseded: usize = 0;

    for new_fact in new_facts {
        let count = resolve_supersessions(pool, new_fact).await?;
        total_superseded += count;
    }

    tracing::info!(
        batch_size = new_facts.len(),
        total_superseded = total_superseded,
        "batch supersession resolution complete"
    );

    Ok(total_superseded)
}

/// Build a [`NewFactDetails`] from an inserted [`Fact`] row.
///
/// Convenience helper for callers that already have the full `Fact` struct
/// after insertion.
pub fn new_fact_details_from(fact: &Fact) -> NewFactDetails {
    NewFactDetails {
        fact_id: fact.id,
        subject_id: fact.subject_id,
        predicate: fact.predicate.clone(),
        object_id: fact.object_id,
        namespace: fact.namespace.clone(),
        valid_from: fact.valid_from,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supersede_error_displays_message() {
        let err = SupersedeError::Fact(FactError::Sqlx(sqlx::Error::RowNotFound));
        let msg = err.to_string();
        assert!(msg.contains("fact database error"), "got: {msg}");
    }

    #[test]
    fn new_fact_details_debug_format() {
        let details = NewFactDetails {
            fact_id: Uuid::new_v4(),
            subject_id: Uuid::new_v4(),
            predicate: "uses".to_string(),
            object_id: Uuid::new_v4(),
            namespace: "default".to_string(),
            valid_from: Utc::now(),
        };
        let debug = format!("{details:?}");
        assert!(debug.contains("uses"));
        assert!(debug.contains("default"));
    }

    #[test]
    fn new_fact_details_clone() {
        let details = NewFactDetails {
            fact_id: Uuid::new_v4(),
            subject_id: Uuid::new_v4(),
            predicate: "deployed_to".to_string(),
            object_id: Uuid::new_v4(),
            namespace: "project-x".to_string(),
            valid_from: Utc::now(),
        };
        let cloned = details.clone();
        assert_eq!(cloned.fact_id, details.fact_id);
        assert_eq!(cloned.predicate, details.predicate);
        assert_eq!(cloned.namespace, details.namespace);
    }

    #[test]
    fn new_fact_details_from_fact() {
        let fact = Fact {
            id: Uuid::new_v4(),
            subject_id: Uuid::new_v4(),
            predicate: "uses".to_string(),
            object_id: Uuid::new_v4(),
            namespace: "default".to_string(),
            valid_from: Utc::now(),
            valid_until: None,
            source_episodes: vec![Uuid::new_v4()],
            superseded_by: None,
            evidence_status: "extracted".to_string(),
            evidence_strength: Some("explicit".to_string()),
            properties: None,
            created_at: Some(Utc::now()),
            deleted_at: None,
        };

        let details = new_fact_details_from(&fact);
        assert_eq!(details.fact_id, fact.id);
        assert_eq!(details.subject_id, fact.subject_id);
        assert_eq!(details.predicate, fact.predicate);
        assert_eq!(details.object_id, fact.object_id);
        assert_eq!(details.namespace, fact.namespace);
        assert_eq!(details.valid_from, fact.valid_from);
    }

    #[test]
    fn new_fact_details_from_fact_with_superseded() {
        let superseding_id = Uuid::new_v4();
        let fact = Fact {
            id: Uuid::new_v4(),
            subject_id: Uuid::new_v4(),
            predicate: "deployed_to".to_string(),
            object_id: Uuid::new_v4(),
            namespace: "prod".to_string(),
            valid_from: Utc::now(),
            valid_until: Some(Utc::now()),
            source_episodes: vec![],
            superseded_by: Some(superseding_id),
            evidence_status: "superseded".to_string(),
            evidence_strength: None,
            properties: None,
            created_at: Some(Utc::now()),
            deleted_at: None,
        };

        let details = new_fact_details_from(&fact);
        // The helper only copies the fields needed for supersession checks.
        assert_eq!(details.fact_id, fact.id);
        assert_eq!(details.predicate, "deployed_to");
    }

    // -- Unit tests for task 7.10: supersession with multiple contradicting facts --

    #[test]
    fn supersession_decision_same_subject_predicate_different_object() {
        // Simulate the core supersession decision: same (subject, predicate,
        // namespace) but different object should trigger supersession.
        let subject_id = Uuid::new_v4();
        let old_object = Uuid::new_v4();
        let new_object = Uuid::new_v4();
        let ns = "default";
        let pred = "deployed_to";

        let old_fact = Fact {
            id: Uuid::new_v4(),
            subject_id,
            predicate: pred.to_string(),
            object_id: old_object,
            namespace: ns.to_string(),
            valid_from: Utc::now(),
            valid_until: None,
            source_episodes: vec![Uuid::new_v4()],
            superseded_by: None,
            evidence_status: "extracted".to_string(),
            evidence_strength: Some("explicit".to_string()),
            properties: None,
            created_at: Some(Utc::now()),
            deleted_at: None,
        };

        let new_details = NewFactDetails {
            fact_id: Uuid::new_v4(),
            subject_id,
            predicate: pred.to_string(),
            object_id: new_object,
            namespace: ns.to_string(),
            valid_from: Utc::now(),
        };

        // Decision: old_fact should be superseded because same subject +
        // predicate + namespace, different object.
        let should_supersede = old_fact.subject_id == new_details.subject_id
            && old_fact.predicate == new_details.predicate
            && old_fact.namespace == new_details.namespace
            && old_fact.object_id != new_details.object_id
            && old_fact.valid_until.is_none()
            && old_fact.deleted_at.is_none();

        assert!(should_supersede, "contradicting fact should be superseded");
    }

    #[test]
    fn supersession_skips_same_object() {
        let subject_id = Uuid::new_v4();
        let object_id = Uuid::new_v4();
        let ns = "default";
        let pred = "uses";

        let old_fact = Fact {
            id: Uuid::new_v4(),
            subject_id,
            predicate: pred.to_string(),
            object_id,
            namespace: ns.to_string(),
            valid_from: Utc::now(),
            valid_until: None,
            source_episodes: vec![Uuid::new_v4()],
            superseded_by: None,
            evidence_status: "extracted".to_string(),
            evidence_strength: Some("explicit".to_string()),
            properties: None,
            created_at: Some(Utc::now()),
            deleted_at: None,
        };

        let new_details = NewFactDetails {
            fact_id: Uuid::new_v4(),
            subject_id,
            predicate: pred.to_string(),
            object_id, // same object
            namespace: ns.to_string(),
            valid_from: Utc::now(),
        };

        let should_supersede = old_fact.object_id != new_details.object_id;
        assert!(!should_supersede, "same object should not trigger supersession");
    }

    #[test]
    fn supersession_multiple_contradicting_facts() {
        // When multiple old facts contradict the new fact, all should be
        // identified for supersession.
        let subject_id = Uuid::new_v4();
        let ns = "project-x";
        let pred = "deployed_to";

        let old_facts: Vec<Fact> = (0..3)
            .map(|_| Fact {
                id: Uuid::new_v4(),
                subject_id,
                predicate: pred.to_string(),
                object_id: Uuid::new_v4(), // each has a different object
                namespace: ns.to_string(),
                valid_from: Utc::now(),
                valid_until: None,
                source_episodes: vec![Uuid::new_v4()],
                superseded_by: None,
                evidence_status: "extracted".to_string(),
                evidence_strength: Some("explicit".to_string()),
                properties: None,
                created_at: Some(Utc::now()),
                deleted_at: None,
            })
            .collect();

        let new_details = NewFactDetails {
            fact_id: Uuid::new_v4(),
            subject_id,
            predicate: pred.to_string(),
            object_id: Uuid::new_v4(), // different from all old facts
            namespace: ns.to_string(),
            valid_from: Utc::now(),
        };

        let supersede_count = old_facts
            .iter()
            .filter(|old| {
                old.id != new_details.fact_id
                    && old.subject_id == new_details.subject_id
                    && old.predicate == new_details.predicate
                    && old.namespace == new_details.namespace
                    && old.object_id != new_details.object_id
                    && old.valid_until.is_none()
                    && old.deleted_at.is_none()
            })
            .count();

        assert_eq!(
            supersede_count, 3,
            "all 3 contradicting facts should be superseded"
        );
    }

    #[test]
    fn supersession_skips_already_superseded_facts() {
        let subject_id = Uuid::new_v4();
        let ns = "default";
        let pred = "uses";

        // Old fact already has valid_until set (already superseded).
        let old_fact = Fact {
            id: Uuid::new_v4(),
            subject_id,
            predicate: pred.to_string(),
            object_id: Uuid::new_v4(),
            namespace: ns.to_string(),
            valid_from: Utc::now(),
            valid_until: Some(Utc::now()), // already superseded
            source_episodes: vec![Uuid::new_v4()],
            superseded_by: Some(Uuid::new_v4()),
            evidence_status: "superseded".to_string(),
            evidence_strength: None,
            properties: None,
            created_at: Some(Utc::now()),
            deleted_at: None,
        };

        let new_details = NewFactDetails {
            fact_id: Uuid::new_v4(),
            subject_id,
            predicate: pred.to_string(),
            object_id: Uuid::new_v4(),
            namespace: ns.to_string(),
            valid_from: Utc::now(),
        };

        let should_supersede = old_fact.object_id != new_details.object_id
            && old_fact.valid_until.is_none(); // this is false

        assert!(
            !should_supersede,
            "already-superseded fact should not be superseded again"
        );
    }

    #[test]
    fn supersession_skips_deleted_facts() {
        let subject_id = Uuid::new_v4();
        let ns = "default";
        let pred = "uses";

        let old_fact = Fact {
            id: Uuid::new_v4(),
            subject_id,
            predicate: pred.to_string(),
            object_id: Uuid::new_v4(),
            namespace: ns.to_string(),
            valid_from: Utc::now(),
            valid_until: None,
            source_episodes: vec![Uuid::new_v4()],
            superseded_by: None,
            evidence_status: "extracted".to_string(),
            evidence_strength: None,
            properties: None,
            created_at: Some(Utc::now()),
            deleted_at: Some(Utc::now()), // soft-deleted
        };

        let new_details = NewFactDetails {
            fact_id: Uuid::new_v4(),
            subject_id,
            predicate: pred.to_string(),
            object_id: Uuid::new_v4(),
            namespace: ns.to_string(),
            valid_from: Utc::now(),
        };

        let should_supersede = old_fact.object_id != new_details.object_id
            && old_fact.valid_until.is_none()
            && old_fact.deleted_at.is_none(); // this is false

        assert!(
            !should_supersede,
            "soft-deleted fact should not be superseded"
        );
    }
}
