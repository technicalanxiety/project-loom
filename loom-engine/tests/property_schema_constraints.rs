//! Property-based tests for database schema uniqueness constraints.
//!
//! **Validates: Requirements 20.6, 20.7**
//!
//! Tests that PostgreSQL UNIQUE constraints on `loom_episodes` and `loom_entities`
//! correctly reject duplicate insertions. Uses proptest to generate random valid
//! field values and verifies the constraint holds across many inputs.
//!
//! Requires the test database to be running:
//! ```sh
//! docker compose -f docker-compose.test.yml up -d postgres-test
//! ```

use proptest::prelude::*;
use sqlx::PgPool;
use uuid::Uuid;

/// Default test database URL matching docker-compose.test.yml configuration.
const DEFAULT_TEST_DB_URL: &str = "postgres://loom_test:loom_test@localhost:5433/loom_test";

/// The 10 valid entity types defined by the CHECK constraint on loom_entities.
const VALID_ENTITY_TYPES: &[&str] = &[
    "person",
    "organization",
    "project",
    "service",
    "technology",
    "pattern",
    "environment",
    "document",
    "metric",
    "decision",
];

/// Connect to the test database and run all pending migrations.
///
/// Uses `DATABASE_URL` env var if set, otherwise falls back to the default
/// test database URL for docker-compose.test.yml.
async fn setup_test_pool() -> PgPool {
    let db_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_TEST_DB_URL.to_string());

    let pool = PgPool::connect(&db_url)
        .await
        .expect("Failed to connect to test database");

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

/// Proptest strategy for generating non-empty alphanumeric strings (1..=50 chars).
///
/// These are safe for use in TEXT columns and avoid edge cases with empty strings
/// or special characters that could complicate constraint testing.
fn non_empty_text() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9_]{1,50}".prop_map(|s| s)
}

/// Proptest strategy for selecting a valid entity type from the 10 allowed values.
fn valid_entity_type() -> impl Strategy<Value = String> {
    prop::sample::select(VALID_ENTITY_TYPES).prop_map(|s| s.to_string())
}

/// Check whether a sqlx error is a PostgreSQL unique violation (error code 23505).
fn is_unique_violation(err: &sqlx::Error) -> bool {
    match err {
        sqlx::Error::Database(db_err) => db_err.code().as_deref() == Some("23505"),
        _ => false,
    }
}

/// **Property 25: Uniqueness Constraints — Episode Deduplication**
///
/// **Validates: Requirements 20.6**
///
/// Test that duplicate (source, source_event_id) insertions into loom_episodes
/// fail with a unique violation error from PostgreSQL.
///
/// Each iteration generates random source and source_event_id values, inserts
/// a row, then attempts a duplicate insertion and verifies it is rejected.
/// All work is done inside a transaction that is rolled back to keep the
/// database clean.
mod episode_uniqueness {
    use super::*;

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        #[test]
        fn duplicate_source_event_id_fails(
            source in non_empty_text(),
            source_event_id in non_empty_text(),
            content1 in non_empty_text(),
            content2 in non_empty_text(),
            namespace in non_empty_text(),
        ) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let pool = setup_test_pool().await;
                let mut tx = pool.begin().await.expect("Failed to begin transaction");

                let content_hash1 = format!("hash_{}", Uuid::new_v4());
                let content_hash2 = format!("hash_{}", Uuid::new_v4());
                let occurred_at = chrono::Utc::now();

                // First insertion should succeed
                let first_result = sqlx::query(
                    r#"
                    INSERT INTO loom_episodes (source, source_event_id, content, content_hash, occurred_at, namespace)
                    VALUES ($1, $2, $3, $4, $5, $6)
                    RETURNING id
                    "#,
                )
                .bind(&source)
                .bind(&source_event_id)
                .bind(&content1)
                .bind(&content_hash1)
                .bind(occurred_at)
                .bind(&namespace)
                .fetch_one(&mut *tx)
                .await;

                prop_assert!(
                    first_result.is_ok(),
                    "First episode insertion should succeed, got: {:?}",
                    first_result.err()
                );

                // Duplicate insertion with same (source, source_event_id) should fail
                let duplicate_result = sqlx::query(
                    r#"
                    INSERT INTO loom_episodes (source, source_event_id, content, content_hash, occurred_at, namespace)
                    VALUES ($1, $2, $3, $4, $5, $6)
                    RETURNING id
                    "#,
                )
                .bind(&source)
                .bind(&source_event_id)
                .bind(&content2)
                .bind(&content_hash2)
                .bind(occurred_at)
                .bind(&namespace)
                .fetch_one(&mut *tx)
                .await;

                prop_assert!(
                    duplicate_result.is_err(),
                    "Duplicate (source, source_event_id) insertion should fail"
                );

                let err = duplicate_result.unwrap_err();
                prop_assert!(
                    is_unique_violation(&err),
                    "Error should be a unique violation (23505), got: {:?}",
                    err
                );

                // Rollback to keep the database clean
                tx.rollback().await.expect("Failed to rollback transaction");

                Ok(())
            })?;
        }
    }
}

/// **Property 25: Uniqueness Constraints — Entity Deduplication**
///
/// **Validates: Requirements 20.7**
///
/// Test that duplicate (name, entity_type, namespace) insertions into loom_entities
/// fail with a unique violation error from PostgreSQL.
///
/// Each iteration generates random name, entity_type (from the 10 valid types),
/// and namespace values, inserts a row, then attempts a duplicate insertion and
/// verifies it is rejected. All work is done inside a transaction that is rolled
/// back to keep the database clean.
mod entity_uniqueness {
    use super::*;

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        #[test]
        fn duplicate_name_type_namespace_fails(
            name in non_empty_text(),
            entity_type in valid_entity_type(),
            namespace in non_empty_text(),
        ) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let pool = setup_test_pool().await;
                let mut tx = pool.begin().await.expect("Failed to begin transaction");

                // First insertion should succeed
                let first_result = sqlx::query(
                    r#"
                    INSERT INTO loom_entities (name, entity_type, namespace)
                    VALUES ($1, $2, $3)
                    RETURNING id
                    "#,
                )
                .bind(&name)
                .bind(&entity_type)
                .bind(&namespace)
                .fetch_one(&mut *tx)
                .await;

                prop_assert!(
                    first_result.is_ok(),
                    "First entity insertion should succeed, got: {:?}",
                    first_result.err()
                );

                // Duplicate insertion with same (name, entity_type, namespace) should fail
                let duplicate_result = sqlx::query(
                    r#"
                    INSERT INTO loom_entities (name, entity_type, namespace)
                    VALUES ($1, $2, $3)
                    RETURNING id
                    "#,
                )
                .bind(&name)
                .bind(&entity_type)
                .bind(&namespace)
                .fetch_one(&mut *tx)
                .await;

                prop_assert!(
                    duplicate_result.is_err(),
                    "Duplicate (name, entity_type, namespace) insertion should fail"
                );

                let err = duplicate_result.unwrap_err();
                prop_assert!(
                    is_unique_violation(&err),
                    "Error should be a unique violation (23505), got: {:?}",
                    err
                );

                // Rollback to keep the database clean
                tx.rollback().await.expect("Failed to rollback transaction");

                Ok(())
            })?;
        }
    }
}
