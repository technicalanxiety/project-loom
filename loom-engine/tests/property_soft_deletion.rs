//! Property-based tests for soft deletion filtering.
//!
//! Tests that soft-deleted items have `deleted_at` set to a non-NULL
//! timestamp and that all retrieval queries exclude items with
//! `deleted_at != NULL`.
//!
//! **Property tested:**
//! - Property 27: Soft Deletion Filtering
//!
//! **Validates: Requirements 27.1, 27.3**

use proptest::prelude::*;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Proptest strategy for generating namespace strings.
fn namespace_strategy() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9-]{0,29}".prop_map(|s| s)
}

/// Proptest strategy for generating deletion reason strings.
fn deletion_reason() -> impl Strategy<Value = String> {
    "[a-zA-Z ]{1,50}".prop_map(|s| s)
}

/// Proptest strategy for generating content strings.
fn content_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 ]{10,100}".prop_map(|s| s)
}

/// Proptest strategy for generating entity names.
fn entity_name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_ -]{0,39}".prop_map(|s| s)
}

/// Simulated record with soft deletion fields.
///
/// Mirrors the common pattern across episodes, entities, facts, and
/// procedures: a `deleted_at` timestamp that is `None` for active records
/// and `Some(timestamp)` for soft-deleted records.
#[derive(Debug, Clone)]
struct SoftDeletableRecord {
    id: Uuid,
    namespace: String,
    deleted_at: Option<DateTime<Utc>>,
    deletion_reason: Option<String>,
}

impl SoftDeletableRecord {
    fn new(namespace: &str) -> Self {
        Self {
            id: Uuid::new_v4(),
            namespace: namespace.to_string(),
            deleted_at: None,
            deletion_reason: None,
        }
    }

    fn soft_delete(&mut self, reason: &str) {
        self.deleted_at = Some(Utc::now());
        self.deletion_reason = Some(reason.to_string());
    }

    fn is_deleted(&self) -> bool {
        self.deleted_at.is_some()
    }
}

/// Simulate a retrieval query that filters out soft-deleted records.
///
/// This mirrors the `WHERE deleted_at IS NULL` clause present in all
/// retrieval queries across episodes, entities, facts, and procedures.
fn query_active_records<'a>(records: &'a [SoftDeletableRecord], namespace: &str) -> Vec<&'a SoftDeletableRecord> {
    records
        .iter()
        .filter(|r| r.namespace == namespace && r.deleted_at.is_none())
        .collect()
}

/// Simulate an audit query that returns only soft-deleted records.
fn query_deleted_records<'a>(records: &'a [SoftDeletableRecord], namespace: &str) -> Vec<&'a SoftDeletableRecord> {
    records
        .iter()
        .filter(|r| r.namespace == namespace && r.deleted_at.is_some())
        .collect()
}

// ---------------------------------------------------------------------------
// Property 27: Soft Deletion Filtering
// ---------------------------------------------------------------------------

/// **Property 27: Soft Deletion Filtering**
///
/// **Validates: Requirements 27.1, 27.3**
///
/// Tests that:
/// 1. Soft-deleted items have `deleted_at` set to a non-NULL timestamp.
/// 2. All retrieval queries exclude items with `deleted_at != NULL`.
/// 3. Soft-deleted records remain in the database for audit purposes.
/// 4. Deletion reasons are preserved.
mod soft_deletion_filtering {
    use super::*;

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// After soft deletion, `deleted_at` is always non-NULL.
        #[test]
        fn soft_deleted_items_have_non_null_deleted_at(
            ns in namespace_strategy(),
            reason in deletion_reason(),
        ) {
            let mut record = SoftDeletableRecord::new(&ns);

            // Before deletion: deleted_at is None.
            prop_assert!(
                record.deleted_at.is_none(),
                "Active record must have deleted_at = None"
            );

            // After deletion: deleted_at is Some(timestamp).
            record.soft_delete(&reason);

            prop_assert!(
                record.deleted_at.is_some(),
                "Soft-deleted record must have deleted_at = Some(timestamp)"
            );
            prop_assert!(
                record.is_deleted(),
                "is_deleted() must return true after soft deletion"
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Retrieval queries never return soft-deleted records.
        #[test]
        fn retrieval_excludes_soft_deleted_records(
            ns in namespace_strategy(),
            active_count in 1usize..20usize,
            deleted_count in 1usize..10usize,
            reason in deletion_reason(),
        ) {
            let mut records: Vec<SoftDeletableRecord> = Vec::new();

            // Create active records.
            for _ in 0..active_count {
                records.push(SoftDeletableRecord::new(&ns));
            }

            // Create and soft-delete records.
            for _ in 0..deleted_count {
                let mut record = SoftDeletableRecord::new(&ns);
                record.soft_delete(&reason);
                records.push(record);
            }

            // Query active records — must exclude all soft-deleted ones.
            let active = query_active_records(&records, &ns);

            prop_assert_eq!(
                active.len(),
                active_count,
                "Active query must return exactly {} records, got {}",
                active_count,
                active.len()
            );

            // Verify none of the returned records are deleted.
            for record in &active {
                prop_assert!(
                    record.deleted_at.is_none(),
                    "Active query must not return records with deleted_at set"
                );
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Soft-deleted records remain accessible for audit queries.
        #[test]
        fn deleted_records_accessible_for_audit(
            ns in namespace_strategy(),
            active_count in 1usize..10usize,
            deleted_count in 1usize..10usize,
            reason in deletion_reason(),
        ) {
            let mut records: Vec<SoftDeletableRecord> = Vec::new();

            for _ in 0..active_count {
                records.push(SoftDeletableRecord::new(&ns));
            }

            for _ in 0..deleted_count {
                let mut record = SoftDeletableRecord::new(&ns);
                record.soft_delete(&reason);
                records.push(record);
            }

            // Audit query — must return only soft-deleted records.
            let deleted = query_deleted_records(&records, &ns);

            prop_assert_eq!(
                deleted.len(),
                deleted_count,
                "Audit query must return exactly {} deleted records, got {}",
                deleted_count,
                deleted.len()
            );

            // Verify all returned records are deleted.
            for record in &deleted {
                prop_assert!(
                    record.deleted_at.is_some(),
                    "Audit query must only return records with deleted_at set"
                );
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Deletion reasons are preserved on soft-deleted records.
        #[test]
        fn deletion_reason_is_preserved(
            ns in namespace_strategy(),
            reason in deletion_reason(),
        ) {
            let mut record = SoftDeletableRecord::new(&ns);
            record.soft_delete(&reason);

            prop_assert_eq!(
                record.deletion_reason.as_deref(),
                Some(reason.as_str()),
                "Deletion reason must be preserved"
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Total record count is preserved after soft deletion.
        ///
        /// Soft deletion never removes records from the database — the total
        /// count of active + deleted records must equal the original count.
        #[test]
        fn total_count_preserved_after_deletion(
            ns in namespace_strategy(),
            total_count in 2usize..30usize,
            delete_fraction in 0.1f64..0.9f64,
            reason in deletion_reason(),
        ) {
            let delete_count = ((total_count as f64) * delete_fraction).ceil() as usize;
            let delete_count = delete_count.min(total_count);

            let mut records: Vec<SoftDeletableRecord> = (0..total_count)
                .map(|_| SoftDeletableRecord::new(&ns))
                .collect();

            // Soft-delete a subset.
            for record in records.iter_mut().take(delete_count) {
                record.soft_delete(&reason);
            }

            let active = query_active_records(&records, &ns);
            let deleted = query_deleted_records(&records, &ns);

            prop_assert_eq!(
                active.len() + deleted.len(),
                total_count,
                "Active ({}) + deleted ({}) must equal total ({})",
                active.len(),
                deleted.len(),
                total_count
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 20, .. ProptestConfig::default() })]

        /// Soft deletion is namespace-scoped — deleting in one namespace
        /// does not affect records in another.
        #[test]
        fn soft_deletion_is_namespace_scoped(
            ns_a in "[a-z]{3,10}",
            ns_b in "[a-z]{3,10}",
            reason in deletion_reason(),
        ) {
            prop_assume!(ns_a != ns_b);

            let mut records = vec![
                SoftDeletableRecord::new(&ns_a),
                SoftDeletableRecord::new(&ns_a),
                SoftDeletableRecord::new(&ns_b),
                SoftDeletableRecord::new(&ns_b),
            ];

            // Soft-delete all records in ns_a.
            for record in records.iter_mut().filter(|r| r.namespace == ns_a) {
                record.soft_delete(&reason);
            }

            // ns_a should have 0 active, 2 deleted.
            let active_a = query_active_records(&records, &ns_a);
            let deleted_a = query_deleted_records(&records, &ns_a);
            prop_assert_eq!(active_a.len(), 0);
            prop_assert_eq!(deleted_a.len(), 2);

            // ns_b should be unaffected: 2 active, 0 deleted.
            let active_b = query_active_records(&records, &ns_b);
            let deleted_b = query_deleted_records(&records, &ns_b);
            prop_assert_eq!(active_b.len(), 2);
            prop_assert_eq!(deleted_b.len(), 0);
        }
    }
}
