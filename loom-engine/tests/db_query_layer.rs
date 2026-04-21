//! Integration tests for the database query layer.
//!
//! **Validates: Requirements 1.2, 3.1, 6.3, 27.3, 5.2**
//!
//! Tests episode idempotency, entity resolution queries (exact, alias, embedding),
//! fact supersession logic, soft deletion filtering, and predicate candidate
//! occurrence counting. Each test uses a transaction with rollback for isolation.
//!
//! Requires the test database to be running:
//! ```sh
//! docker compose -f docker-compose.test.yml up -d postgres-test
//! ```

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

/// Default test database URL matching docker-compose.test.yml configuration.
const DEFAULT_TEST_DB_URL: &str = "postgres://loom_test:loom_test@localhost:5433/loom_test";

/// Connect to the test database and run all pending migrations.
///
/// Uses `DATABASE_URL` env var if set, otherwise falls back to the default
/// test database URL for docker-compose.test.yml.
async fn setup_test_pool() -> PgPool {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_TEST_DB_URL.to_string());

    let pool = PgPool::connect(&db_url)
        .await
        .expect("Failed to connect to test database");

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

// ---------------------------------------------------------------------------
// Episode idempotency tests
// ---------------------------------------------------------------------------

/// Tests that inserting an episode with the same (source, source_event_id)
/// returns the existing row instead of creating a duplicate.
///
/// **Validates: Requirement 1.2**
mod episode_idempotency {
    use super::*;
    use loom_engine::db::episodes::{insert_episode, NewEpisode};

    #[tokio::test]
    async fn duplicate_source_event_id_returns_existing_row() {
        let pool = setup_test_pool().await;
        let mut tx = pool.begin().await.expect("begin tx");

        let source = format!("test-source-{}", Uuid::new_v4());
        let event_id = format!("evt-{}", Uuid::new_v4());

        let ep1 = NewEpisode {
            source: source.clone(),
            source_id: None,
            source_event_id: Some(event_id.clone()),
            content: "First episode content".to_string(),
            content_hash: format!("hash_{}", Uuid::new_v4()),
            occurred_at: Utc::now(),
            namespace: "test-ns".to_string(),
            metadata: None,
            participants: None,
            ingestion_mode: "live_mcp_capture".to_string(),
            parser_version: None,
            parser_source_schema: None,
        };

        // Use the pool directly (insert_episode takes &PgPool).
        // We'll clean up via unique random values instead of tx rollback
        // since the db functions take &PgPool, not a transaction.
        let first = insert_episode(&pool, &ep1).await.expect("first insert");

        let ep2 = NewEpisode {
            source: source.clone(),
            source_id: None,
            source_event_id: Some(event_id.clone()),
            content: "Different content, same event".to_string(),
            content_hash: format!("hash_{}", Uuid::new_v4()),
            occurred_at: Utc::now(),
            namespace: "test-ns".to_string(),
            metadata: None,
            participants: None,
            ingestion_mode: "live_mcp_capture".to_string(),
            parser_version: None,
            parser_source_schema: None,
        };

        let second = insert_episode(&pool, &ep2).await.expect("second insert");

        assert_eq!(
            first.id, second.id,
            "Duplicate source_event_id should return the same episode ID"
        );

        tx.rollback().await.expect("rollback");
    }

    #[tokio::test]
    async fn different_source_event_ids_create_separate_rows() {
        let pool = setup_test_pool().await;

        let source = format!("test-source-{}", Uuid::new_v4());

        let ep1 = NewEpisode {
            source: source.clone(),
            source_id: None,
            source_event_id: Some(format!("evt-{}", Uuid::new_v4())),
            content: "Episode A".to_string(),
            content_hash: format!("hash_{}", Uuid::new_v4()),
            occurred_at: Utc::now(),
            namespace: "test-ns".to_string(),
            metadata: None,
            participants: None,
            ingestion_mode: "live_mcp_capture".to_string(),
            parser_version: None,
            parser_source_schema: None,
        };

        let ep2 = NewEpisode {
            source: source.clone(),
            source_id: None,
            source_event_id: Some(format!("evt-{}", Uuid::new_v4())),
            content: "Episode B".to_string(),
            content_hash: format!("hash_{}", Uuid::new_v4()),
            occurred_at: Utc::now(),
            namespace: "test-ns".to_string(),
            metadata: None,
            participants: None,
            ingestion_mode: "live_mcp_capture".to_string(),
            parser_version: None,
            parser_source_schema: None,
        };

        let first = insert_episode(&pool, &ep1).await.expect("first insert");
        let second = insert_episode(&pool, &ep2).await.expect("second insert");

        assert_ne!(
            first.id, second.id,
            "Different source_event_ids should create separate episodes"
        );
    }
}

// ---------------------------------------------------------------------------
// Entity exact match tests
// ---------------------------------------------------------------------------

/// Tests entity exact match resolution including case-insensitive name matching.
///
/// **Validates: Requirement 3.1**
mod entity_exact_match {
    use super::*;
    use loom_engine::db::entities::{get_entity_by_name_type_namespace, insert_entity, NewEntity};

    #[tokio::test]
    async fn exact_match_finds_entity_by_name_type_namespace() {
        let pool = setup_test_pool().await;

        let ns = format!("test-ns-{}", Uuid::new_v4());
        let entity = NewEntity {
            name: "MyProject".to_string(),
            entity_type: "project".to_string(),
            namespace: ns.clone(),
            properties: None,
            source_episodes: None,
        };

        let inserted = insert_entity(&pool, &entity).await.expect("insert entity");

        let found = get_entity_by_name_type_namespace(&pool, "MyProject", "project", &ns)
            .await
            .expect("query");

        assert!(found.is_some(), "Should find entity by exact name");
        assert_eq!(found.unwrap().id, inserted.id);
    }

    #[tokio::test]
    async fn case_insensitive_match_finds_entity() {
        let pool = setup_test_pool().await;

        let ns = format!("test-ns-{}", Uuid::new_v4());
        let entity = NewEntity {
            name: "MyProject".to_string(),
            entity_type: "project".to_string(),
            namespace: ns.clone(),
            properties: None,
            source_episodes: None,
        };

        let inserted = insert_entity(&pool, &entity).await.expect("insert entity");

        // Query with different casing
        let found = get_entity_by_name_type_namespace(&pool, "myproject", "project", &ns)
            .await
            .expect("query");

        assert!(
            found.is_some(),
            "Should find entity with case-insensitive match"
        );
        assert_eq!(found.unwrap().id, inserted.id);
    }

    #[tokio::test]
    async fn insert_entity_idempotency_returns_existing() {
        let pool = setup_test_pool().await;

        let ns = format!("test-ns-{}", Uuid::new_v4());
        let entity = NewEntity {
            name: "DuplicateEntity".to_string(),
            entity_type: "service".to_string(),
            namespace: ns.clone(),
            properties: None,
            source_episodes: None,
        };

        let first = insert_entity(&pool, &entity).await.expect("first insert");
        let second = insert_entity(&pool, &entity).await.expect("second insert");

        assert_eq!(
            first.id, second.id,
            "Duplicate entity insert should return existing row"
        );
    }
}

// ---------------------------------------------------------------------------
// Entity alias match tests
// ---------------------------------------------------------------------------

/// Tests entity alias lookup via JSONB containment on properties->'aliases'.
///
/// **Validates: Requirement 3.1 (alias resolution pass)**
mod entity_alias_match {
    use super::*;
    use loom_engine::db::entities::{insert_entity, query_entities_by_alias, NewEntity};

    #[tokio::test]
    async fn alias_match_finds_entity_by_alias() {
        let pool = setup_test_pool().await;

        let ns = format!("test-ns-{}", Uuid::new_v4());
        let entity = NewEntity {
            name: "Kubernetes".to_string(),
            entity_type: "technology".to_string(),
            namespace: ns.clone(),
            properties: Some(serde_json::json!({
                "aliases": ["k8s", "kube"]
            })),
            source_episodes: None,
        };

        let inserted = insert_entity(&pool, &entity).await.expect("insert entity");

        let results = query_entities_by_alias(&pool, "k8s", "technology", &ns)
            .await
            .expect("alias query");

        assert_eq!(results.len(), 1, "Should find exactly one entity by alias");
        assert_eq!(results[0].id, inserted.id);
    }

    #[tokio::test]
    async fn alias_match_returns_empty_for_unknown_alias() {
        let pool = setup_test_pool().await;

        let ns = format!("test-ns-{}", Uuid::new_v4());
        let entity = NewEntity {
            name: "PostgreSQL".to_string(),
            entity_type: "technology".to_string(),
            namespace: ns.clone(),
            properties: Some(serde_json::json!({
                "aliases": ["postgres", "pg"]
            })),
            source_episodes: None,
        };

        insert_entity(&pool, &entity).await.expect("insert entity");

        let results = query_entities_by_alias(&pool, "mysql", "technology", &ns)
            .await
            .expect("alias query");

        assert!(
            results.is_empty(),
            "Should not find entity with non-matching alias"
        );
    }
}

// ---------------------------------------------------------------------------
// Entity embedding similarity tests
// ---------------------------------------------------------------------------

/// Tests entity embedding similarity search via pgvector cosine distance.
///
/// **Validates: Requirement 3.1 (semantic resolution pass)**
mod entity_embedding_similarity {
    use super::*;
    use loom_engine::db::entities::{
        insert_entity, query_entities_by_embedding_similarity, update_entity_state, NewEntity,
    };
    use pgvector::Vector;

    #[tokio::test]
    async fn embedding_similarity_finds_entity_above_threshold() {
        let pool = setup_test_pool().await;

        let ns = format!("test-ns-{}", Uuid::new_v4());
        let entity = NewEntity {
            name: "SimilarEntity".to_string(),
            entity_type: "project".to_string(),
            namespace: ns.clone(),
            properties: None,
            source_episodes: None,
        };

        let inserted = insert_entity(&pool, &entity).await.expect("insert entity");

        // Create a 768-dim embedding with a distinctive pattern
        let mut embedding_vec = vec![0.1_f32; 768];
        embedding_vec[0] = 0.9;
        embedding_vec[1] = 0.8;
        embedding_vec[2] = 0.7;
        let embedding = Vector::from(embedding_vec.clone());

        // Insert entity state with embedding
        update_entity_state(&pool, inserted.id, Some(&embedding), 0.5, "warm", 0, None)
            .await
            .expect("update entity state");

        // Query with the same embedding — should get similarity ~1.0
        let query_embedding = Vector::from(embedding_vec);
        let results = query_entities_by_embedding_similarity(
            &pool,
            &query_embedding,
            "project",
            &ns,
            0.5, // low threshold to ensure we find it
            10,
        )
        .await
        .expect("similarity query");

        assert!(
            !results.is_empty(),
            "Should find entity by embedding similarity"
        );
        assert_eq!(results[0].id, inserted.id);
        assert!(
            results[0].similarity > 0.99,
            "Identical embedding should have similarity ~1.0, got {}",
            results[0].similarity
        );
    }

    #[tokio::test]
    async fn embedding_similarity_excludes_below_threshold() {
        let pool = setup_test_pool().await;

        let ns = format!("test-ns-{}", Uuid::new_v4());
        let entity = NewEntity {
            name: "DistantEntity".to_string(),
            entity_type: "service".to_string(),
            namespace: ns.clone(),
            properties: None,
            source_episodes: None,
        };

        let inserted = insert_entity(&pool, &entity).await.expect("insert entity");

        // Create a specific embedding
        let mut emb1 = vec![0.0_f32; 768];
        emb1[0] = 1.0;
        let embedding = Vector::from(emb1);

        update_entity_state(&pool, inserted.id, Some(&embedding), 0.5, "warm", 0, None)
            .await
            .expect("update entity state");

        // Query with a very different embedding
        let mut emb2 = vec![0.0_f32; 768];
        emb2[767] = 1.0;
        let query_embedding = Vector::from(emb2);

        let results = query_entities_by_embedding_similarity(
            &pool,
            &query_embedding,
            "service",
            &ns,
            0.92, // high threshold
            10,
        )
        .await
        .expect("similarity query");

        assert!(
            results.is_empty(),
            "Orthogonal embedding should not match at 0.92 threshold"
        );
    }
}

// ---------------------------------------------------------------------------
// Fact supersession tests
// ---------------------------------------------------------------------------

/// Tests fact supersession logic: setting valid_until, superseded_by, and
/// evidence_status on the old fact, and filtering superseded facts from
/// current queries.
///
/// **Validates: Requirement 6.3**
mod fact_supersession {
    use super::*;
    use loom_engine::db::entities::{insert_entity, NewEntity};
    use loom_engine::db::facts::{
        insert_fact, query_current_facts_by_namespace, supersede_fact, NewFact,
    };

    /// Helper to create a test entity and return its ID.
    async fn create_test_entity(pool: &PgPool, name: &str, ns: &str) -> Uuid {
        let entity = NewEntity {
            name: name.to_string(),
            entity_type: "project".to_string(),
            namespace: ns.to_string(),
            properties: None,
            source_episodes: None,
        };
        insert_entity(pool, &entity)
            .await
            .expect("insert entity")
            .id
    }

    #[tokio::test]
    async fn supersede_fact_sets_valid_until_and_superseded_by() {
        let pool = setup_test_pool().await;

        let ns = format!("test-ns-{}", Uuid::new_v4());
        let subject_id =
            create_test_entity(&pool, &format!("SubjectA-{}", Uuid::new_v4()), &ns).await;
        let object_id =
            create_test_entity(&pool, &format!("ObjectA-{}", Uuid::new_v4()), &ns).await;
        let new_object_id =
            create_test_entity(&pool, &format!("ObjectB-{}", Uuid::new_v4()), &ns).await;

        // Insert original fact
        let old_fact = insert_fact(
            &pool,
            &NewFact {
                subject_id,
                predicate: "uses".to_string(),
                object_id,
                namespace: ns.clone(),
                source_episodes: vec![Uuid::new_v4()],
                evidence_status: "extracted".to_string(),
                evidence_strength: Some("explicit".to_string()),
                properties: None,
            },
        )
        .await
        .expect("insert old fact");

        assert!(
            old_fact.valid_until.is_none(),
            "New fact should have no valid_until"
        );
        assert!(
            old_fact.superseded_by.is_none(),
            "New fact should have no superseded_by"
        );

        // Insert contradicting fact
        let new_fact = insert_fact(
            &pool,
            &NewFact {
                subject_id,
                predicate: "uses".to_string(),
                object_id: new_object_id,
                namespace: ns.clone(),
                source_episodes: vec![Uuid::new_v4()],
                evidence_status: "extracted".to_string(),
                evidence_strength: Some("explicit".to_string()),
                properties: None,
            },
        )
        .await
        .expect("insert new fact");

        // Supersede the old fact
        let superseded = supersede_fact(&pool, old_fact.id, new_fact.id)
            .await
            .expect("supersede fact");

        assert!(
            superseded.valid_until.is_some(),
            "Superseded fact should have valid_until set"
        );
        assert_eq!(
            superseded.superseded_by,
            Some(new_fact.id),
            "Superseded fact should point to new fact"
        );
        assert_eq!(
            superseded.evidence_status, "superseded",
            "Superseded fact should have evidence_status = 'superseded'"
        );
    }

    #[tokio::test]
    async fn current_facts_query_excludes_superseded() {
        let pool = setup_test_pool().await;

        let ns = format!("test-ns-{}", Uuid::new_v4());
        let subject_id =
            create_test_entity(&pool, &format!("SubjectC-{}", Uuid::new_v4()), &ns).await;
        let object_id =
            create_test_entity(&pool, &format!("ObjectC-{}", Uuid::new_v4()), &ns).await;
        let new_object_id =
            create_test_entity(&pool, &format!("ObjectD-{}", Uuid::new_v4()), &ns).await;

        // Insert and supersede a fact
        let old_fact = insert_fact(
            &pool,
            &NewFact {
                subject_id,
                predicate: "depends_on".to_string(),
                object_id,
                namespace: ns.clone(),
                source_episodes: vec![Uuid::new_v4()],
                evidence_status: "extracted".to_string(),
                evidence_strength: Some("explicit".to_string()),
                properties: None,
            },
        )
        .await
        .expect("insert old fact");

        let new_fact = insert_fact(
            &pool,
            &NewFact {
                subject_id,
                predicate: "depends_on".to_string(),
                object_id: new_object_id,
                namespace: ns.clone(),
                source_episodes: vec![Uuid::new_v4()],
                evidence_status: "extracted".to_string(),
                evidence_strength: Some("explicit".to_string()),
                properties: None,
            },
        )
        .await
        .expect("insert new fact");

        supersede_fact(&pool, old_fact.id, new_fact.id)
            .await
            .expect("supersede");

        // Query current facts — should only return the new fact
        let current = query_current_facts_by_namespace(&pool, &ns, 100)
            .await
            .expect("query current facts");

        let ids: Vec<Uuid> = current.iter().map(|f| f.id).collect();
        assert!(
            !ids.contains(&old_fact.id),
            "Superseded fact should be excluded from current facts"
        );
        assert!(
            ids.contains(&new_fact.id),
            "New (current) fact should be included"
        );
    }
}

// ---------------------------------------------------------------------------
// Soft deletion filtering tests
// ---------------------------------------------------------------------------

/// Tests that soft-deleted records are excluded from standard queries across
/// episodes, entities, and facts.
///
/// **Validates: Requirement 27.3**
mod soft_deletion_filtering {
    use super::*;
    use loom_engine::db::entities::{get_entity_by_name_type_namespace, insert_entity, NewEntity};
    use loom_engine::db::episodes::{
        insert_episode, query_episodes_by_namespace, soft_delete_episode, NewEpisode,
    };
    use loom_engine::db::facts::{
        insert_fact, query_current_facts_by_namespace, query_facts_by_entity, soft_delete_fact,
        NewFact,
    };

    #[tokio::test]
    async fn soft_deleted_episode_excluded_from_namespace_query() {
        let pool = setup_test_pool().await;

        let ns = format!("test-ns-{}", Uuid::new_v4());
        let ep = NewEpisode {
            source: format!("test-{}", Uuid::new_v4()),
            source_id: None,
            source_event_id: Some(format!("evt-{}", Uuid::new_v4())),
            content: "Episode to delete".to_string(),
            content_hash: format!("hash_{}", Uuid::new_v4()),
            occurred_at: Utc::now(),
            namespace: ns.clone(),
            metadata: None,
            participants: None,
            ingestion_mode: "live_mcp_capture".to_string(),
            parser_version: None,
            parser_source_schema: None,
        };

        let inserted = insert_episode(&pool, &ep).await.expect("insert episode");

        // Verify it appears in namespace query
        let before = query_episodes_by_namespace(&pool, &ns, None, 100)
            .await
            .expect("query before delete");
        assert!(
            before.iter().any(|e| e.id == inserted.id),
            "Episode should appear before deletion"
        );

        // Soft delete
        soft_delete_episode(&pool, inserted.id, "test cleanup")
            .await
            .expect("soft delete");

        // Verify it's excluded
        let after = query_episodes_by_namespace(&pool, &ns, None, 100)
            .await
            .expect("query after delete");
        assert!(
            !after.iter().any(|e| e.id == inserted.id),
            "Soft-deleted episode should be excluded from namespace query"
        );
    }

    #[tokio::test]
    async fn soft_deleted_entity_excluded_from_exact_match() {
        let pool = setup_test_pool().await;

        let ns = format!("test-ns-{}", Uuid::new_v4());
        let name = format!("DeleteMe-{}", Uuid::new_v4());
        let entity = NewEntity {
            name: name.clone(),
            entity_type: "technology".to_string(),
            namespace: ns.clone(),
            properties: None,
            source_episodes: None,
        };

        let inserted = insert_entity(&pool, &entity).await.expect("insert entity");

        // Soft delete via raw SQL (no soft_delete_entity function exists on entities)
        sqlx::query("UPDATE loom_entities SET deleted_at = now() WHERE id = $1")
            .bind(inserted.id)
            .execute(&pool)
            .await
            .expect("soft delete entity");

        // Exact match should not find it
        let found = get_entity_by_name_type_namespace(&pool, &name, "technology", &ns)
            .await
            .expect("query");

        assert!(
            found.is_none(),
            "Soft-deleted entity should be excluded from exact match query"
        );
    }

    #[tokio::test]
    async fn soft_deleted_fact_excluded_from_queries() {
        let pool = setup_test_pool().await;

        let ns = format!("test-ns-{}", Uuid::new_v4());

        // Create entities for the fact
        let subject = insert_entity(
            &pool,
            &NewEntity {
                name: format!("FactSubject-{}", Uuid::new_v4()),
                entity_type: "service".to_string(),
                namespace: ns.clone(),
                properties: None,
                source_episodes: None,
            },
        )
        .await
        .expect("insert subject");

        let object = insert_entity(
            &pool,
            &NewEntity {
                name: format!("FactObject-{}", Uuid::new_v4()),
                entity_type: "technology".to_string(),
                namespace: ns.clone(),
                properties: None,
                source_episodes: None,
            },
        )
        .await
        .expect("insert object");

        let fact = insert_fact(
            &pool,
            &NewFact {
                subject_id: subject.id,
                predicate: "uses".to_string(),
                object_id: object.id,
                namespace: ns.clone(),
                source_episodes: vec![Uuid::new_v4()],
                evidence_status: "extracted".to_string(),
                evidence_strength: Some("explicit".to_string()),
                properties: None,
            },
        )
        .await
        .expect("insert fact");

        // Verify fact appears in queries
        let before_ns = query_current_facts_by_namespace(&pool, &ns, 100)
            .await
            .expect("query ns before");
        assert!(before_ns.iter().any(|f| f.id == fact.id));

        let before_entity = query_facts_by_entity(&pool, subject.id, &ns)
            .await
            .expect("query entity before");
        assert!(before_entity.iter().any(|f| f.id == fact.id));

        // Soft delete the fact
        soft_delete_fact(&pool, fact.id)
            .await
            .expect("soft delete fact");

        // Verify exclusion from both query types
        let after_ns = query_current_facts_by_namespace(&pool, &ns, 100)
            .await
            .expect("query ns after");
        assert!(
            !after_ns.iter().any(|f| f.id == fact.id),
            "Soft-deleted fact should be excluded from namespace query"
        );

        let after_entity = query_facts_by_entity(&pool, subject.id, &ns)
            .await
            .expect("query entity after");
        assert!(
            !after_entity.iter().any(|f| f.id == fact.id),
            "Soft-deleted fact should be excluded from entity query"
        );
    }
}

// ---------------------------------------------------------------------------
// Predicate candidate occurrence counting tests
// ---------------------------------------------------------------------------

/// Tests predicate candidate upsert logic: occurrence incrementing and
/// threshold-based querying.
///
/// **Validates: Requirement 5.2**
mod predicate_candidate_counting {
    use super::*;
    use loom_engine::db::predicates::{insert_or_update_candidate, query_candidates_by_threshold};

    #[tokio::test]
    async fn insert_or_update_increments_occurrences() {
        let pool = setup_test_pool().await;

        let predicate = format!("custom_pred_{}", Uuid::new_v4());
        let fact_id_1 = Uuid::new_v4();
        let fact_id_2 = Uuid::new_v4();
        let fact_id_3 = Uuid::new_v4();

        // First call creates a new candidate with occurrences = 1
        insert_or_update_candidate(&pool, &predicate, fact_id_1)
            .await
            .expect("first insert");

        // Second and third calls increment occurrences
        insert_or_update_candidate(&pool, &predicate, fact_id_2)
            .await
            .expect("second insert");
        insert_or_update_candidate(&pool, &predicate, fact_id_3)
            .await
            .expect("third insert");

        // Query with threshold 1 — should find our candidate
        let candidates = query_candidates_by_threshold(&pool, 1)
            .await
            .expect("query candidates");

        let ours = candidates
            .iter()
            .find(|c| c.predicate == predicate)
            .expect("should find our candidate");

        assert_eq!(
            ours.occurrences,
            Some(3),
            "Occurrences should be 3 after three inserts"
        );

        // Verify example_facts contains all three fact IDs
        let example_facts = ours
            .example_facts
            .as_ref()
            .expect("should have example_facts");
        assert_eq!(example_facts.len(), 3, "Should have 3 example facts");
        assert!(example_facts.contains(&fact_id_1));
        assert!(example_facts.contains(&fact_id_2));
        assert!(example_facts.contains(&fact_id_3));
    }

    #[tokio::test]
    async fn threshold_query_filters_by_minimum_occurrences() {
        let pool = setup_test_pool().await;

        let pred_low = format!("low_pred_{}", Uuid::new_v4());
        let pred_high = format!("high_pred_{}", Uuid::new_v4());

        // Insert low-occurrence candidate (1 occurrence)
        insert_or_update_candidate(&pool, &pred_low, Uuid::new_v4())
            .await
            .expect("insert low");

        // Insert high-occurrence candidate (5 occurrences)
        for _ in 0..5 {
            insert_or_update_candidate(&pool, &pred_high, Uuid::new_v4())
                .await
                .expect("insert high");
        }

        // Query with threshold 5 — should only find the high-occurrence candidate
        let candidates = query_candidates_by_threshold(&pool, 5)
            .await
            .expect("query threshold 5");

        let predicates: Vec<&str> = candidates.iter().map(|c| c.predicate.as_str()).collect();
        assert!(
            predicates.contains(&pred_high.as_str()),
            "High-occurrence candidate should be returned at threshold 5"
        );
        assert!(
            !predicates.contains(&pred_low.as_str()),
            "Low-occurrence candidate should be excluded at threshold 5"
        );
    }
}

// ---------------------------------------------------------------------------
// Episode processing lifecycle (retry / backoff / failed / requeue)
// ---------------------------------------------------------------------------

/// Tests for the processing-state machine introduced in migration 016.
/// These guard against the poison-pill-retry-forever regression: an
/// episode that fails deterministically must eventually transition to
/// `failed` and stop consuming worker cycles.
mod episode_processing_lifecycle {
    use super::*;
    use loom_engine::db::episodes::{
        claim_episode_for_processing, count_failed_episodes, insert_episode, list_failed_episodes,
        list_unprocessed_episodes, mark_episode_processed, record_processing_failure,
        requeue_episode, NewEpisode,
    };

    /// Build a minimal NewEpisode with unique source_event_id so each test
    /// owns its own row even when they run concurrently against a shared DB.
    fn fresh_episode(content: &str) -> NewEpisode {
        NewEpisode {
            source: format!("test-{}", Uuid::new_v4()),
            source_id: None,
            source_event_id: Some(format!("evt-{}", Uuid::new_v4())),
            content: content.to_string(),
            content_hash: format!("hash-{}", Uuid::new_v4()),
            occurred_at: Utc::now(),
            namespace: format!("test-ns-{}", Uuid::new_v4()),
            metadata: None,
            participants: None,
            ingestion_mode: "live_mcp_capture".to_string(),
            parser_version: None,
            parser_source_schema: None,
        }
    }

    #[tokio::test]
    async fn new_episode_defaults_to_pending_with_zero_attempts() {
        let pool = setup_test_pool().await;
        let ep = insert_episode(&pool, &fresh_episode("pending defaults"))
            .await
            .expect("insert");

        assert_eq!(ep.processing_status, "pending");
        assert_eq!(ep.processing_attempts, 0);
        assert!(ep.processing_last_attempt.is_none());
        assert!(ep.processing_last_error.is_none());
    }

    #[tokio::test]
    async fn claim_transitions_pending_to_processing_and_increments_attempts() {
        let pool = setup_test_pool().await;
        let ep = insert_episode(&pool, &fresh_episode("claim"))
            .await
            .expect("insert");

        let claimed = claim_episode_for_processing(&pool, ep.id)
            .await
            .expect("claim")
            .expect("should be claimable");

        assert_eq!(claimed.processing_status, "processing");
        assert_eq!(claimed.processing_attempts, 1);
        assert!(claimed.processing_last_attempt.is_some());
    }

    #[tokio::test]
    async fn claim_is_atomic_second_caller_gets_none() {
        let pool = setup_test_pool().await;
        let ep = insert_episode(&pool, &fresh_episode("atomic claim"))
            .await
            .expect("insert");

        let first = claim_episode_for_processing(&pool, ep.id)
            .await
            .expect("first claim");
        let second = claim_episode_for_processing(&pool, ep.id)
            .await
            .expect("second claim");

        assert!(first.is_some(), "first claim should succeed");
        assert!(
            second.is_none(),
            "second claim must return None — row is no longer pending"
        );
    }

    #[tokio::test]
    async fn record_failure_below_max_returns_to_pending() {
        let pool = setup_test_pool().await;
        let ep = insert_episode(&pool, &fresh_episode("retry"))
            .await
            .expect("insert");

        // Claim pushes attempts to 1.
        claim_episode_for_processing(&pool, ep.id)
            .await
            .expect("claim")
            .expect("claimed");

        // attempts=1 < max_attempts=5 → pending.
        let after = record_processing_failure(&pool, ep.id, "boom", 5)
            .await
            .expect("record failure");

        assert_eq!(after.processing_status, "pending");
        assert_eq!(after.processing_attempts, 1);
        assert_eq!(after.processing_last_error.as_deref(), Some("boom"));
    }

    #[tokio::test]
    async fn record_failure_at_max_transitions_to_failed() {
        let pool = setup_test_pool().await;
        let ep = insert_episode(&pool, &fresh_episode("exhaust"))
            .await
            .expect("insert");

        // Simulate hitting the max by claiming repeatedly: each claim
        // requires returning to pending first, so loop claim → fail.
        for _ in 0..3 {
            claim_episode_for_processing(&pool, ep.id)
                .await
                .expect("claim")
                .expect("claimed");
            record_processing_failure(&pool, ep.id, "fail", 3)
                .await
                .expect("record");
        }

        let terminal = sqlx::query_as::<_, loom_engine::types::episode::Episode>(
            "SELECT * FROM loom_episodes WHERE id = $1",
        )
        .bind(ep.id)
        .fetch_one(&pool)
        .await
        .expect("fetch");

        assert_eq!(terminal.processing_status, "failed");
        assert_eq!(terminal.processing_attempts, 3);
    }

    #[tokio::test]
    async fn failed_episode_not_returned_by_poll_query() {
        let pool = setup_test_pool().await;
        let ep = insert_episode(&pool, &fresh_episode("poll-exclude"))
            .await
            .expect("insert");

        // Burn through retries to land in 'failed'.
        for _ in 0..2 {
            claim_episode_for_processing(&pool, ep.id)
                .await
                .expect("claim")
                .expect("claimed");
            record_processing_failure(&pool, ep.id, "deterministic", 2)
                .await
                .expect("record");
        }

        // Poll with a generous batch; the failed episode must be excluded.
        let pending = list_unprocessed_episodes(&pool, 100, 1)
            .await
            .expect("list");
        assert!(
            !pending.iter().any(|e| e.id == ep.id),
            "failed episode must not appear in the pending poll result"
        );
    }

    #[tokio::test]
    async fn backoff_skips_recently_attempted_episode() {
        let pool = setup_test_pool().await;
        let input = fresh_episode("backoff");
        // Capture the namespace so we can scope assertions to just the
        // episodes this test owns — the test DB is shared across tests.
        let namespace = input.namespace.clone();
        let ep = insert_episode(&pool, &input).await.expect("insert");

        // Claim once (attempts=1, last_attempt=now) and record failure so
        // status returns to 'pending'.
        claim_episode_for_processing(&pool, ep.id)
            .await
            .expect("claim")
            .expect("claimed");
        record_processing_failure(&pool, ep.id, "transient", 5)
            .await
            .expect("record");

        // Helper that applies the same backoff predicate as
        // list_unprocessed_episodes but scoped to our unique namespace so
        // cross-test data can't affect the result.
        async fn is_ready_in_ns(pool: &PgPool, namespace: &str, base: i64) -> bool {
            let count: i64 = sqlx::query_scalar(
                r#"
                SELECT COUNT(*)
                FROM loom_episodes
                WHERE namespace = $1
                  AND processing_status = 'pending'
                  AND deleted_at IS NULL
                  AND (
                    processing_last_attempt IS NULL
                    OR processing_last_attempt
                       + ($2 * (1::bigint << LEAST(processing_attempts, 20))) * interval '1 second'
                       < NOW()
                  )
                "#,
            )
            .bind(namespace)
            .bind(base)
            .fetch_one(pool)
            .await
            .expect("scoped count");
            count > 0
        }

        // With base=60s and attempts=1, backoff window is 120s. The
        // episode should be skipped by the predicate.
        assert!(
            !is_ready_in_ns(&pool, &namespace, 60).await,
            "episode under backoff must not be considered ready"
        );

        // With base=0s, the predicate becomes last_attempt < NOW() which
        // holds for any past attempt, so the episode is ready.
        assert!(
            is_ready_in_ns(&pool, &namespace, 0).await,
            "episode outside backoff window must be considered ready"
        );
    }

    #[tokio::test]
    async fn mark_processed_sets_completed_and_clears_error() {
        let pool = setup_test_pool().await;
        let ep = insert_episode(&pool, &fresh_episode("complete"))
            .await
            .expect("insert");

        // Simulate a prior failed attempt so there's an error on file.
        claim_episode_for_processing(&pool, ep.id)
            .await
            .expect("claim")
            .expect("claimed");
        record_processing_failure(&pool, ep.id, "earlier error", 5)
            .await
            .expect("record");

        // Then succeed on the next attempt.
        claim_episode_for_processing(&pool, ep.id)
            .await
            .expect("claim")
            .expect("claimed again");
        let completed = mark_episode_processed(&pool, ep.id, &serde_json::json!({"ok": true}))
            .await
            .expect("mark processed");

        assert_eq!(completed.processing_status, "completed");
        assert_eq!(completed.processed, Some(true));
        assert!(
            completed.processing_last_error.is_none(),
            "last_error should be cleared on success"
        );
    }

    #[tokio::test]
    async fn requeue_resets_status_attempts_and_error() {
        let pool = setup_test_pool().await;
        let ep = insert_episode(&pool, &fresh_episode("requeue"))
            .await
            .expect("insert");

        // Drive to 'failed'.
        for _ in 0..2 {
            claim_episode_for_processing(&pool, ep.id)
                .await
                .expect("claim")
                .expect("claimed");
            record_processing_failure(&pool, ep.id, "boom", 2)
                .await
                .expect("record");
        }

        let reset = requeue_episode(&pool, ep.id)
            .await
            .expect("requeue")
            .expect("episode exists");

        assert_eq!(reset.processing_status, "pending");
        assert_eq!(reset.processing_attempts, 0);
        assert!(reset.processing_last_attempt.is_none());
        assert!(reset.processing_last_error.is_none());
        assert_eq!(reset.processed, Some(false));
    }

    #[tokio::test]
    async fn requeue_returns_none_for_unknown_id() {
        let pool = setup_test_pool().await;
        let missing = requeue_episode(&pool, Uuid::new_v4())
            .await
            .expect("requeue");
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn failed_episodes_appear_in_list_and_count() {
        let pool = setup_test_pool().await;
        let ep = insert_episode(&pool, &fresh_episode("surface"))
            .await
            .expect("insert");

        // Exhaust retries.
        for _ in 0..2 {
            claim_episode_for_processing(&pool, ep.id)
                .await
                .expect("claim")
                .expect("claimed");
            record_processing_failure(&pool, ep.id, "reason", 2)
                .await
                .expect("record");
        }

        let count = count_failed_episodes(&pool).await.expect("count");
        assert!(count >= 1, "expected at least one failed episode");

        let listed = list_failed_episodes(&pool, 100).await.expect("list");
        assert!(
            listed.iter().any(|e| e.id == ep.id),
            "failed episode must be surfaced in the failed-list query"
        );
    }

    #[tokio::test]
    async fn record_failure_truncates_oversized_error_message() {
        let pool = setup_test_pool().await;
        let ep = insert_episode(&pool, &fresh_episode("truncate"))
            .await
            .expect("insert");
        claim_episode_for_processing(&pool, ep.id)
            .await
            .expect("claim")
            .expect("claimed");

        // 10 KiB of 'x' — well beyond the 2000-char cap.
        let huge = "x".repeat(10_000);
        let after = record_processing_failure(&pool, ep.id, &huge, 5)
            .await
            .expect("record");

        let stored = after
            .processing_last_error
            .expect("error should be populated");
        assert!(
            stored.len() <= 2000,
            "stored error should be truncated to <= 2000 chars, got {}",
            stored.len()
        );
    }
}
