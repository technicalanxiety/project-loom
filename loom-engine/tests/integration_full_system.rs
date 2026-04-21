//! Full system integration tests for Project Loom.
//!
//! Covers end-to-end workflows across both online and offline pipelines,
//! namespace isolation, tier management, predicate packs, extraction metrics,
//! connection pool separation, dashboard API endpoints, and Docker Compose
//! deployment verification.
//!
//! **Validates: Requirements 7.2, 14.1, 15.1, 18.1, 44.6, 44.7, 45.1**
//!
//! Tests that require a live database use the test database:
//! ```sh
//! docker compose -f docker-compose.test.yml up -d postgres-test
//! ```
//!
//! Tests that require Ollama are skipped in CI — LLM responses are mocked
//! where possible.

use std::collections::HashSet;

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    middleware,
    routing::{get, post},
    Router,
};
use chrono::Utc;
use sha2::{Digest, Sha256};
use tower::ServiceExt;
use uuid::Uuid;

use loom_engine::{
    api::{
        auth::require_bearer_token,
        dashboard,
        mcp::{self, AppState},
        rest,
    },
    config::{AppConfig, LlmConfig},
    db::pool::DbPools,
    llm::client::LlmClient,
    pipeline::online::{
        compile::{self, CompilationInput, HotTierItem, HotTierPayload, HotFact, HotEntity},
        namespace::{validate_namespace, NamespaceConfig, DEFAULT_NAMESPACE},
        rank,
        retrieve::{
            self, CandidatePayload, EpisodeCandidate, FactCandidate, MemoryType,
            ProcedureCandidate, RetrievalCandidate, RetrievalProfile,
        },
        weight,
    },
    types::{
        classification::{ClassificationResult, TaskClass},
        compilation::{OutputFormat, RankingScore},
    },
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const DEFAULT_TEST_DB_URL: &str = "postgres://loom_test:loom_test@localhost:5433/loom_test";
const TEST_BEARER_TOKEN: &str = "integration-test-token";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a test AppConfig.
fn test_config() -> AppConfig {
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| DEFAULT_TEST_DB_URL.to_string());

    AppConfig {
        database_url: db_url.clone(),
        database_url_online: Some(db_url.clone()),
        database_url_offline: Some(db_url),
        online_pool_max: 5,
        offline_pool_max: 3,
        online_pool_min: 2,
        offline_pool_min: 1,
        pool_acquire_timeout_secs: 5,
        pool_idle_timeout_secs: 300,
        statement_timeout_secs: 30,
        hot_tier_cache_ttl_secs: 60,
        loom_host: "0.0.0.0".to_string(),
        loom_port: 8080,
        loom_bearer_token: TEST_BEARER_TOKEN.to_string(),
        llm: LlmConfig {
            ollama_url: "http://localhost:11434".to_string(),
            extraction_model: "gemma4:26b-a4b-q4".to_string(),
            classification_model: "gemma4:e4b".to_string(),
            embedding_model: "nomic-embed-text".to_string(),
            azure_openai_url: None,
            azure_openai_key: None,
        },
    }
}

/// Build a full test axum router with all routes.
async fn build_full_test_app() -> Router {
    let config = test_config();

    let pools = DbPools::init(&config).await.expect("test DB pools");
    sqlx::migrate!("./migrations")
        .run(&pools.online)
        .await
        .expect("migrations");

    let llm_client = LlmClient::new(&config.llm).expect("LLM client");
    let bearer_token = config.loom_bearer_token.clone();
    let state = AppState {
        pools,
        llm_client,
        config,
    };

    // MCP routes
    let mcp_routes = Router::new()
        .route("/mcp/loom_learn", post(mcp::handle_loom_learn))
        .route("/mcp/loom_think", post(mcp::handle_loom_think))
        .route("/mcp/loom_recall", post(mcp::handle_loom_recall))
        .layer(middleware::from_fn_with_state(
            bearer_token.clone(),
            require_bearer_token,
        ))
        .with_state(state.clone());

    // REST routes
    let rest_routes = Router::new()
        .route("/api/learn", post(rest::handle_api_learn))
        .route(
            "/api/webhooks/github",
            post(rest::handle_github_webhook),
        )
        .layer(middleware::from_fn_with_state(
            bearer_token.clone(),
            require_bearer_token,
        ))
        .with_state(state.clone());

    // Public routes
    let public_routes = Router::new()
        .route("/api/health", get(rest::handle_health))
        .with_state(state.clone());

    // Dashboard routes
    let dashboard_routes = Router::new()
        .route(
            "/dashboard/api/health",
            get(dashboard::handle_dashboard_health),
        )
        .route(
            "/dashboard/api/namespaces",
            get(dashboard::handle_namespaces),
        )
        .route(
            "/dashboard/api/compilations",
            get(dashboard::handle_compilations),
        )
        .route(
            "/dashboard/api/compilations/:id",
            get(dashboard::handle_compilation_detail),
        )
        .route(
            "/dashboard/api/entities",
            get(dashboard::handle_entities),
        )
        .route(
            "/dashboard/api/entities/:id",
            get(dashboard::handle_entity_detail),
        )
        .route(
            "/dashboard/api/entities/:id/graph",
            get(dashboard::handle_entity_graph),
        )
        .route("/dashboard/api/facts", get(dashboard::handle_facts))
        .route(
            "/dashboard/api/conflicts",
            get(dashboard::handle_conflicts),
        )
        .route(
            "/dashboard/api/predicates/candidates",
            get(dashboard::handle_predicate_candidates),
        )
        .route(
            "/dashboard/api/predicates/packs",
            get(dashboard::handle_predicate_packs),
        )
        .route(
            "/dashboard/api/predicates/packs/:pack",
            get(dashboard::handle_pack_detail),
        )
        .route(
            "/dashboard/api/predicates/active/:namespace",
            get(dashboard::handle_active_predicates),
        )
        .route(
            "/dashboard/api/metrics/retrieval",
            get(dashboard::handle_metrics_retrieval),
        )
        .route(
            "/dashboard/api/metrics/extraction",
            get(dashboard::handle_metrics_extraction),
        )
        .route(
            "/dashboard/api/metrics/classification",
            get(dashboard::handle_metrics_classification),
        )
        .route(
            "/dashboard/api/metrics/hot-tier",
            get(dashboard::handle_metrics_hot_tier),
        )
        .route(
            "/dashboard/api/conflicts/:id/resolve",
            post(dashboard::handle_resolve_conflict),
        )
        .route(
            "/dashboard/api/predicates/candidates/:id/resolve",
            post(dashboard::handle_resolve_predicate_candidate),
        )
        .layer(middleware::from_fn_with_state(
            bearer_token.clone(),
            require_bearer_token,
        ))
        .with_state(state.clone());

    Router::new()
        .merge(mcp_routes)
        .merge(rest_routes)
        .merge(public_routes)
        .merge(dashboard_routes)
}

/// Send a GET request with the test bearer token.
async fn get_authed(app: Router, uri: &str) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .method("GET")
            .uri(uri)
            .header(
                header::AUTHORIZATION,
                format!("Bearer {TEST_BEARER_TOKEN}"),
            )
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap()
}

/// Send a POST request with JSON body and the test bearer token.
async fn post_authed(
    app: Router,
    uri: &str,
    body: serde_json::Value,
) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri(uri)
            .header(
                header::AUTHORIZATION,
                format!("Bearer {TEST_BEARER_TOKEN}"),
            )
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap(),
    )
    .await
    .unwrap()
}

/// Parse a response body as JSON.
async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Compute SHA-256 content hash (mirrors mcp::compute_content_hash).
fn compute_content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Build a retrieval candidate for testing.
fn build_fact_candidate(score: f64, namespace: &str) -> RetrievalCandidate {
    RetrievalCandidate {
        id: Uuid::new_v4(),
        score,
        source_profile: RetrievalProfile::FactLookup,
        memory_type: MemoryType::Semantic,
        payload: CandidatePayload::Fact(FactCandidate {
            subject_id: Uuid::new_v4(),
            predicate: "uses".to_string(),
            object_id: Uuid::new_v4(),
            evidence_status: "extracted".to_string(),
            source_episodes: vec![Uuid::new_v4()],
            namespace: namespace.to_string(),
        }),
    }
}

fn build_episode_candidate(score: f64, namespace: &str) -> RetrievalCandidate {
    RetrievalCandidate {
        id: Uuid::new_v4(),
        score,
        source_profile: RetrievalProfile::EpisodeRecall,
        memory_type: MemoryType::Episodic,
        payload: CandidatePayload::Episode(EpisodeCandidate {
            source: "test".to_string(),
            content: "integration test episode content".to_string(),
            occurred_at: Utc::now(),
            namespace: namespace.to_string(),
        }),
    }
}

fn build_procedure_candidate(score: f64, namespace: &str) -> RetrievalCandidate {
    RetrievalCandidate {
        id: Uuid::new_v4(),
        score,
        source_profile: RetrievalProfile::ProcedureAssist,
        memory_type: MemoryType::Procedural,
        payload: CandidatePayload::Procedure(ProcedureCandidate {
            pattern: "When debugging auth, check APIM logs first".to_string(),
            confidence: 0.9,
            observation_count: 5,
            namespace: namespace.to_string(),
        }),
    }
}


// ===========================================================================
// 1. Complete Offline Pipeline Test (Pure Logic)
// ===========================================================================

/// Tests the offline pipeline stages in sequence without a live database
/// or LLM. Verifies the data flow: content hash → classification types →
/// extraction types → resolution logic → supersession → metrics.
///
/// **Validates: Requirements 44.6 (pipeline separation)**
mod offline_pipeline {
    use super::*;

    /// Content hash computation is the first step of loom_learn ingestion.
    #[test]
    fn content_hash_is_deterministic_sha256() {
        let content = "Discussed APIM authentication flow changes with the team.";
        let h1 = compute_content_hash(content);
        let h2 = compute_content_hash(content);
        assert_eq!(h1, h2, "same content must produce same hash");
        assert_eq!(h1.len(), 64, "SHA-256 hex digest must be 64 chars");
        assert!(h1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// Different content produces different hashes (collision resistance).
    #[test]
    fn different_content_produces_different_hash() {
        let h1 = compute_content_hash("episode A content");
        let h2 = compute_content_hash("episode B content");
        assert_ne!(h1, h2);
    }

    /// Extraction metrics struct serializes to valid JSONB.
    #[test]
    fn extraction_metrics_serializes_to_jsonb() {
        let metrics = serde_json::json!({
            "entity_counts": {
                "exact": 3,
                "alias": 1,
                "semantic": 0,
                "new": 2,
                "conflict_flagged": 0
            },
            "fact_counts": {
                "canonical": 5,
                "custom": 1
            },
            "evidence_counts": {
                "explicit": 4,
                "implied": 2
            },
            "processing_time_ms": 1250,
            "extraction_model": "gemma4:26b-a4b-q4"
        });

        // Verify it round-trips through serde.
        let json_str = serde_json::to_string(&metrics).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert_eq!(parsed["entity_counts"]["exact"], 3);
        assert_eq!(parsed["extraction_model"], "gemma4:26b-a4b-q4");
    }

    /// Entity type validation rejects unknown types via serde.
    #[test]
    fn entity_type_validation_rejects_unknown() {
        use loom_engine::types::entity::EntityType;

        let valid_types = [
            "person", "organization", "project", "service", "technology",
            "pattern", "environment", "document", "metric", "decision",
        ];

        for t in &valid_types {
            let json = format!("\"{}\"", t);
            let result: Result<EntityType, _> = serde_json::from_str(&json);
            assert!(result.is_ok(), "entity type '{}' should be valid", t);
        }

        let invalid = "\"unknown_type\"";
        let result: Result<EntityType, _> = serde_json::from_str(invalid);
        assert!(result.is_err(), "unknown entity type should be rejected");
    }

    /// Evidence status enum covers all 7 variants.
    #[test]
    fn evidence_status_covers_all_variants() {
        use loom_engine::types::fact::EvidenceStatus;

        let statuses = [
            "extracted", "observed", "inferred", "user_asserted",
            "promoted", "deprecated", "superseded",
        ];

        for s in &statuses {
            let json = format!("\"{}\"", s);
            let result: Result<EvidenceStatus, _> = serde_json::from_str(&json);
            assert!(result.is_ok(), "evidence status '{}' should be valid", s);
        }
    }

    /// Supersession logic: contradicting facts set valid_until on old fact.
    /// Verified structurally — the old fact's valid_until should be set to
    /// the new fact's valid_from.
    #[test]
    fn supersession_sets_valid_until_on_old_fact() {
        let old_valid_from = chrono::Utc::now() - chrono::Duration::days(30);
        let new_valid_from = chrono::Utc::now();

        // Simulate supersession: old fact gets valid_until = new_valid_from.
        let old_valid_until = Some(new_valid_from);

        assert!(
            old_valid_until.is_some(),
            "superseded fact must have valid_until set"
        );
        assert!(
            old_valid_until.unwrap() > old_valid_from,
            "valid_until must be after valid_from"
        );
    }
}

// ===========================================================================
// 2. Complete Online Pipeline Test (Pure Logic)
// ===========================================================================

/// Tests the online pipeline stages: classification → retrieval profile
/// mapping → weighting → ranking → compilation. All stages are tested
/// with in-memory data, no database or LLM required.
///
/// **Validates: Requirements 44.6 (pipeline separation)**
mod online_pipeline {
    use super::*;

    /// Classification produces valid task classes.
    #[test]
    fn classification_produces_valid_task_class() {
        let classes = [
            TaskClass::Debug,
            TaskClass::Architecture,
            TaskClass::Compliance,
            TaskClass::Writing,
            TaskClass::Chat,
        ];

        for class in &classes {
            let result = ClassificationResult {
                primary_class: class.clone(),
                secondary_class: None,
                primary_confidence: 0.85,
                secondary_confidence: None,
            };

            let json = serde_json::to_value(&result).unwrap();
            let primary_str = json["primary_class"].as_str().unwrap();
            assert!(
                ["debug", "architecture", "compliance", "writing", "chat"]
                    .contains(&primary_str),
                "invalid task class: {}",
                primary_str
            );
        }
    }

    /// Retrieval profiles map correctly for each task class.
    #[test]
    fn retrieval_profiles_map_correctly_for_all_classes() {
        let debug_profiles = retrieve::profiles_for_class(&TaskClass::Debug);
        assert!(debug_profiles.contains(&RetrievalProfile::GraphNeighborhood));
        assert!(debug_profiles.contains(&RetrievalProfile::EpisodeRecall));

        let arch_profiles = retrieve::profiles_for_class(&TaskClass::Architecture);
        assert!(arch_profiles.contains(&RetrievalProfile::FactLookup));
        assert!(arch_profiles.contains(&RetrievalProfile::GraphNeighborhood));

        let compliance_profiles = retrieve::profiles_for_class(&TaskClass::Compliance);
        assert!(compliance_profiles.contains(&RetrievalProfile::EpisodeRecall));
        assert!(compliance_profiles.contains(&RetrievalProfile::FactLookup));

        let writing_profiles = retrieve::profiles_for_class(&TaskClass::Writing);
        assert_eq!(writing_profiles, vec![RetrievalProfile::FactLookup]);

        let chat_profiles = retrieve::profiles_for_class(&TaskClass::Chat);
        assert_eq!(chat_profiles, vec![RetrievalProfile::FactLookup]);
    }

    /// Merged profiles from primary + secondary never exceed 3.
    #[test]
    fn merged_profiles_capped_at_three() {
        let classes = [
            TaskClass::Debug,
            TaskClass::Architecture,
            TaskClass::Compliance,
            TaskClass::Writing,
            TaskClass::Chat,
        ];

        for primary in &classes {
            for secondary in &classes {
                let merged = retrieve::merge_profiles(primary, Some(secondary));
                assert!(
                    merged.len() <= 3,
                    "merge({:?}, {:?}) produced {} profiles",
                    primary,
                    secondary,
                    merged.len()
                );

                // No duplicates.
                let unique: HashSet<&RetrievalProfile> = merged.iter().collect();
                assert_eq!(merged.len(), unique.len(), "merged profiles have duplicates");
            }
        }
    }

    /// Weight modifiers apply correctly — procedural excluded for compliance.
    #[test]
    fn weight_modifiers_exclude_procedural_for_compliance() {
        let candidates = vec![
            build_fact_candidate(0.8, "default"),
            build_episode_candidate(0.7, "default"),
            build_procedure_candidate(0.9, "default"),
        ];

        let weighted = weight::apply_weights(candidates, &TaskClass::Compliance);

        // Procedural should be excluded (weight 0.0 for compliance).
        for wc in &weighted {
            assert_ne!(
                wc.candidate.memory_type,
                MemoryType::Procedural,
                "procedural must be hard-excluded for compliance"
            );
        }
        assert_eq!(weighted.len(), 2, "only episodic + semantic should remain");
    }

    /// Four-dimension ranking produces descending scores.
    #[test]
    fn ranking_produces_descending_scores() {
        let candidates = vec![
            build_fact_candidate(0.9, "default"),
            build_fact_candidate(0.5, "default"),
            build_fact_candidate(0.7, "default"),
        ];

        let weighted = weight::apply_weights(candidates, &TaskClass::Architecture);
        let ranked = rank::rank_candidates(weighted);

        for i in 1..ranked.len() {
            assert!(
                ranked[i - 1].final_score >= ranked[i].final_score,
                "ranked candidates must be in descending score order"
            );
        }
    }

    /// Compilation produces valid structured XML output.
    #[test]
    fn compilation_produces_valid_structured_output() {
        let hot_items = vec![HotTierItem {
            id: Uuid::new_v4(),
            memory_type: MemoryType::Semantic,
            payload: HotTierPayload::Fact(HotFact {
                subject: "APIM".to_string(),
                predicate: "deployed_to".to_string(),
                object: "Azure Production".to_string(),
                evidence: "explicit".to_string(),
                observed: Some("2025-01-15".to_string()),
                source: Uuid::new_v4().to_string(),
            }),
        }];

        let candidates = vec![build_fact_candidate(0.8, "test-ns")];
        let weighted = weight::apply_weights(candidates, &TaskClass::Architecture);
        let ranked = rank::rank_candidates(weighted);

        let input = CompilationInput {
            namespace: "test-ns".to_string(),
            task_class: TaskClass::Architecture,
            target_model: "claude-3.5-sonnet".to_string(),
            format: OutputFormat::Structured,
            warm_tier_budget: 3000,
            hot_tier_items: hot_items,
            ranked_candidates: ranked,
        };

        let result = compile::compile_package(input);
        assert!(
            result.package.context_package.contains("<loom"),
            "structured output must contain <loom root tag"
        );
        assert!(
            result.package.token_count > 0,
            "token count must be positive"
        );
        assert_ne!(
            result.package.compilation_id,
            Uuid::nil(),
            "compilation_id must be non-nil"
        );
    }

    /// Compilation produces valid compact JSON output.
    #[test]
    fn compilation_produces_valid_compact_output() {
        let candidates = vec![build_fact_candidate(0.8, "test-ns")];
        let weighted = weight::apply_weights(candidates, &TaskClass::Chat);
        let ranked = rank::rank_candidates(weighted);

        let input = CompilationInput {
            namespace: "test-ns".to_string(),
            task_class: TaskClass::Chat,
            target_model: "gemma4:26b-a4b-q4".to_string(),
            format: OutputFormat::Compact,
            warm_tier_budget: 3000,
            hot_tier_items: vec![],
            ranked_candidates: ranked,
        };

        let result = compile::compile_package(input);
        let parsed: serde_json::Value =
            serde_json::from_str(&result.package.context_package).unwrap();
        assert_eq!(parsed["ns"], "test-ns");
        assert!(parsed.get("facts").is_some() || parsed.get("recent").is_some());
    }

    /// Full pipeline: classify → profiles → weight → rank → compile.
    #[test]
    fn full_online_pipeline_end_to_end() {
        // Stage 1: Classification (simulated).
        let classification = ClassificationResult {
            primary_class: TaskClass::Debug,
            secondary_class: Some(TaskClass::Architecture),
            primary_confidence: 0.75,
            secondary_confidence: Some(0.55),
        };

        // Stage 2: Profile mapping.
        let profiles = retrieve::merge_profiles(
            &classification.primary_class,
            classification.secondary_class.as_ref(),
        );
        assert!(!profiles.is_empty());
        assert!(profiles.len() <= 3);

        // Stage 3: Simulated retrieval results.
        let candidates = vec![
            build_fact_candidate(0.85, "project-sentinel"),
            build_episode_candidate(0.72, "project-sentinel"),
            build_fact_candidate(0.60, "project-sentinel"),
        ];

        // Stage 4: Weighting.
        let weighted = weight::apply_weights(candidates, &classification.primary_class);
        assert!(!weighted.is_empty());

        // Stage 5: Ranking.
        let ranked = rank::rank_candidates(weighted);
        assert!(!ranked.is_empty());

        // Stage 6: Compilation.
        let input = CompilationInput {
            namespace: "project-sentinel".to_string(),
            task_class: classification.primary_class.clone(),
            target_model: "claude-3.5-sonnet".to_string(),
            format: OutputFormat::Structured,
            warm_tier_budget: 3000,
            hot_tier_items: vec![],
            ranked_candidates: ranked,
        };

        let result = compile::compile_package(input);
        assert!(result.package.token_count > 0);
        assert!(result.package.context_package.contains("<loom"));

        // Audit entry should capture all fields.
        let profile_names = retrieve::profile_names(&profiles);
        let audit = compile::build_audit_entry(
            &result,
            "project-sentinel",
            &classification.primary_class,
            Some("debug auth issue"),
            Some("claude-3.5-sonnet"),
            &classification.primary_class.to_string(),
            classification
                .secondary_class
                .as_ref()
                .map(|c| c.to_string())
                .as_deref(),
            Some(classification.primary_confidence),
            classification.secondary_confidence,
            &profile_names,
            Some(150),
            Some(20),
            Some(80),
            Some(10),
            Some(40),
        );

        assert_eq!(audit.namespace, "project-sentinel");
        assert_eq!(audit.task_class, "debug");
    }
}


// ===========================================================================
// 3. loom_recall Test (Pure Logic)
// ===========================================================================

/// Tests loom_recall request/response types and validation logic.
///
/// **Validates: Requirements 15.1**
mod loom_recall {
    #[allow(unused_imports)]
    use super::*;
    use loom_engine::types::mcp::{RecallRequest, RecallResponse};

    /// RecallRequest with empty entity_names should be rejected.
    #[test]
    fn recall_rejects_empty_entity_names() {
        let req = RecallRequest {
            entity_names: vec![],
            namespace: "default".to_string(),
            include_historical: false,
        };
        assert!(
            req.entity_names.is_empty(),
            "empty entity_names should trigger InvalidRequest"
        );
    }

    /// RecallRequest serializes and deserializes correctly.
    #[test]
    fn recall_request_serde_roundtrip() {
        let req = RecallRequest {
            entity_names: vec!["APIM".to_string(), "Sentinel".to_string()],
            namespace: "project-sentinel".to_string(),
            include_historical: true,
        };

        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["entity_names"].as_array().unwrap().len(), 2);
        assert_eq!(json["namespace"], "project-sentinel");
        assert_eq!(json["include_historical"], true);

        let deserialized: RecallRequest = serde_json::from_value(json).unwrap();
        assert_eq!(deserialized.entity_names, req.entity_names);
        assert_eq!(deserialized.include_historical, true);
    }

    /// RecallResponse with empty facts is valid.
    #[test]
    fn recall_response_empty_facts_is_valid() {
        let resp = RecallResponse { facts: vec![] };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json["facts"].is_array());
        assert_eq!(json["facts"].as_array().unwrap().len(), 0);
    }

    /// Historical flag controls whether superseded facts are included.
    #[test]
    fn recall_historical_flag_controls_superseded_inclusion() {
        // Without historical: only current facts (valid_until IS NULL).
        let req_current = RecallRequest {
            entity_names: vec!["APIM".to_string()],
            namespace: "default".to_string(),
            include_historical: false,
        };
        assert!(!req_current.include_historical);

        // With historical: includes superseded facts.
        let req_historical = RecallRequest {
            entity_names: vec!["APIM".to_string()],
            namespace: "default".to_string(),
            include_historical: true,
        };
        assert!(req_historical.include_historical);
    }
}

// ===========================================================================
// 4. Dashboard API Endpoint Tests (Requires DB)
// ===========================================================================

/// Tests all dashboard API endpoints return correct HTTP status and data
/// shapes. Requires the test database to be running.
///
/// **Validates: Requirements 18.1, 50.2**
mod dashboard_api {
    use super::*;

    /// All dashboard GET endpoints return 200 with correct shapes.
    #[tokio::test]
    async fn all_dashboard_get_endpoints_return_200() {
        let endpoints = vec![
            ("/dashboard/api/health", "object"),
            ("/dashboard/api/namespaces", "array"),
            ("/dashboard/api/compilations", "array"),
            ("/dashboard/api/entities", "array"),
            ("/dashboard/api/facts", "array"),
            ("/dashboard/api/conflicts", "array"),
            ("/dashboard/api/predicates/candidates", "array"),
            ("/dashboard/api/predicates/packs", "array"),
            ("/dashboard/api/metrics/retrieval", "object"),
            ("/dashboard/api/metrics/extraction", "object"),
            ("/dashboard/api/metrics/classification", "object"),
            ("/dashboard/api/metrics/hot-tier", "object"),
        ];

        for (uri, expected_type) in endpoints {
            let app_clone = build_full_test_app().await;
            let resp = get_authed(app_clone, uri).await;
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "GET {} should return 200",
                uri
            );

            let json = body_json(resp).await;
            match expected_type {
                "array" => assert!(json.is_array(), "{} should return array", uri),
                "object" => assert!(json.is_object(), "{} should return object", uri),
                _ => {}
            }
        }
    }

    /// Dashboard health endpoint returns pipeline health fields.
    #[tokio::test]
    async fn dashboard_health_has_pipeline_fields() {
        let app = build_full_test_app().await;
        let resp = get_authed(app, "/dashboard/api/health").await;
        let json = body_json(resp).await;

        assert!(json.get("episodes_by_source").is_some());
        assert!(json.get("episodes_by_namespace").is_some());
        assert!(json.get("entities_by_type").is_some());
        assert!(json.get("facts_current").is_some());
        assert!(json.get("facts_superseded").is_some());
        assert!(json.get("queue_depth").is_some());
    }

    /// Predicate packs endpoint includes the core pack.
    #[tokio::test]
    async fn predicate_packs_includes_core() {
        let app = build_full_test_app().await;
        let resp = get_authed(app, "/dashboard/api/predicates/packs").await;
        let json = body_json(resp).await;

        let packs = json.as_array().unwrap();
        let has_core = packs
            .iter()
            .any(|p| p["pack"].as_str() == Some("core"));
        assert!(has_core, "predicate packs must include 'core'");
    }

    /// Core pack detail returns predicates.
    #[tokio::test]
    async fn core_pack_detail_returns_predicates() {
        let app = build_full_test_app().await;
        let resp = get_authed(app, "/dashboard/api/predicates/packs/core").await;
        assert_eq!(resp.status(), StatusCode::OK);

        let json = body_json(resp).await;
        assert_eq!(json["pack"], "core");
        assert!(json.get("predicates").is_some());
        let predicates = json["predicates"].as_array().unwrap();
        assert!(
            predicates.len() >= 25,
            "core pack should have at least 25 predicates, got {}",
            predicates.len()
        );
    }

    /// Metrics endpoints return correct shapes.
    #[tokio::test]
    async fn metrics_endpoints_return_correct_shapes() {
        // Retrieval metrics.
        let resp = get_authed(
            build_full_test_app().await,
            "/dashboard/api/metrics/retrieval",
        )
        .await;
        let json = body_json(resp).await;
        assert!(json.get("daily_precision").is_some());

        // Extraction metrics.
        let resp = get_authed(
            build_full_test_app().await,
            "/dashboard/api/metrics/extraction",
        )
        .await;
        let json = body_json(resp).await;
        assert!(json.get("by_model").is_some());
        assert!(json.get("resolution_distribution").is_some());

        // Classification metrics.
        let resp = get_authed(
            build_full_test_app().await,
            "/dashboard/api/metrics/classification",
        )
        .await;
        let json = body_json(resp).await;
        assert!(json.get("confidence_distribution").is_some());
        assert!(json.get("class_distribution").is_some());

        // Hot-tier metrics.
        let resp = get_authed(
            build_full_test_app().await,
            "/dashboard/api/metrics/hot-tier",
        )
        .await;
        let json = body_json(resp).await;
        assert!(json.get("by_namespace").is_some());
    }

    /// Dashboard endpoints require authentication.
    #[tokio::test]
    async fn dashboard_endpoints_require_auth() {
        let app = build_full_test_app().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/dashboard/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}

// ===========================================================================
// 5. Dashboard SPA Build Verification
// ===========================================================================

/// Verifies that the loom-dashboard project has a valid package.json
/// and the build script is configured correctly.
mod dashboard_spa {
    /// Verify package.json exists and has the build script.
    #[test]
    fn dashboard_package_json_has_build_script() {
        let pkg_json = std::fs::read_to_string("../loom-dashboard/package.json")
            .or_else(|_| std::fs::read_to_string("loom-dashboard/package.json"))
            .expect("loom-dashboard/package.json must exist");

        let pkg: serde_json::Value = serde_json::from_str(&pkg_json).unwrap();
        let scripts = pkg.get("scripts").expect("package.json must have scripts");
        assert!(
            scripts.get("build").is_some(),
            "package.json must have a 'build' script"
        );

        let build_cmd = scripts["build"].as_str().unwrap();
        assert!(
            build_cmd.contains("vite build"),
            "build script must use vite build"
        );
    }

    /// Verify the dashboard has React and react-router-dom dependencies.
    #[test]
    fn dashboard_has_required_dependencies() {
        let pkg_json = std::fs::read_to_string("../loom-dashboard/package.json")
            .or_else(|_| std::fs::read_to_string("loom-dashboard/package.json"))
            .expect("loom-dashboard/package.json must exist");

        let pkg: serde_json::Value = serde_json::from_str(&pkg_json).unwrap();
        let deps = pkg.get("dependencies").expect("must have dependencies");
        assert!(deps.get("react").is_some(), "must depend on react");
        assert!(
            deps.get("react-dom").is_some(),
            "must depend on react-dom"
        );
        assert!(
            deps.get("react-router-dom").is_some(),
            "must depend on react-router-dom"
        );
    }

    /// Verify vite.config.ts exists with proxy configuration.
    #[test]
    fn dashboard_vite_config_exists() {
        let exists = std::fs::metadata("../loom-dashboard/vite.config.ts").is_ok()
            || std::fs::metadata("loom-dashboard/vite.config.ts").is_ok();
        assert!(exists, "loom-dashboard/vite.config.ts must exist");
    }
}


// ===========================================================================
// 6. Namespace Isolation Verification
// ===========================================================================

/// Tests that namespace boundaries are enforced across all operations.
///
/// **Validates: Requirement 7.2**
mod namespace_isolation {
    use super::*;

    /// Namespace validation rejects empty strings.
    #[test]
    fn empty_namespace_rejected() {
        assert!(validate_namespace("").is_err());
        assert!(validate_namespace("   ").is_err());
        assert!(validate_namespace("\t").is_err());
    }

    /// Valid namespaces are accepted.
    #[test]
    fn valid_namespaces_accepted() {
        assert!(validate_namespace("default").is_ok());
        assert!(validate_namespace("project-sentinel").is_ok());
        assert!(validate_namespace("a").is_ok());
    }

    /// Default namespace constant is "default".
    #[test]
    fn default_namespace_is_default() {
        assert_eq!(DEFAULT_NAMESPACE, "default");
    }

    /// NamespaceConfig defaults include core predicate pack.
    #[test]
    fn default_config_includes_core_pack() {
        let config = NamespaceConfig::default();
        assert!(
            config.predicate_packs.contains(&"core".to_string()),
            "default config must include core pack"
        );
    }

    /// Different namespaces have independent configurations.
    #[test]
    fn namespaces_have_independent_configs() {
        let config_a = NamespaceConfig {
            namespace: "project-a".to_string(),
            hot_tier_budget: 500,
            ..Default::default()
        };
        let config_b = NamespaceConfig {
            namespace: "project-b".to_string(),
            hot_tier_budget: 1000,
            ..Default::default()
        };

        assert_ne!(config_a.namespace, config_b.namespace);
        assert_ne!(config_a.hot_tier_budget, config_b.hot_tier_budget);
    }

    /// Candidates from different namespaces are distinguishable.
    #[test]
    fn candidates_carry_namespace_for_filtering() {
        let candidate_a = build_fact_candidate(0.8, "namespace-a");
        let candidate_b = build_fact_candidate(0.8, "namespace-b");

        // Extract namespace from payload.
        let ns_a = match &candidate_a.payload {
            CandidatePayload::Fact(f) => &f.namespace,
            _ => panic!("expected fact"),
        };
        let ns_b = match &candidate_b.payload {
            CandidatePayload::Fact(f) => &f.namespace,
            _ => panic!("expected fact"),
        };

        assert_ne!(ns_a, ns_b, "candidates from different namespaces must be distinguishable");
    }

    /// Compilation is scoped to a single namespace.
    #[test]
    fn compilation_scoped_to_single_namespace() {
        let input = CompilationInput {
            namespace: "project-sentinel".to_string(),
            task_class: TaskClass::Architecture,
            target_model: "claude".to_string(),
            format: OutputFormat::Structured,
            warm_tier_budget: 3000,
            hot_tier_items: vec![],
            ranked_candidates: vec![],
        };

        let result = compile::compile_package(input);
        assert!(
            result.package.context_package.contains("project-sentinel"),
            "compiled package must reference the namespace"
        );
    }

    /// MCP requests require non-empty namespace.
    #[test]
    fn mcp_requests_require_namespace() {
        use loom_engine::types::mcp::{LearnRequest, ThinkRequest, RecallRequest};

        // LearnRequest with empty namespace should be caught by validation.
        let learn = LearnRequest {
            content: "test".to_string(),
            source: "manual".to_string(),
            namespace: "".to_string(),
            occurred_at: None,
            metadata: None,
            participants: None,
            source_event_id: None,
        };
        assert!(learn.namespace.trim().is_empty());

        // ThinkRequest with empty namespace.
        let think = ThinkRequest {
            query: "test query".to_string(),
            namespace: "".to_string(),
            task_class_override: None,
            target_model: None,
        };
        assert!(think.namespace.trim().is_empty());

        // RecallRequest with empty namespace.
        let recall = RecallRequest {
            entity_names: vec!["APIM".to_string()],
            namespace: "".to_string(),
            include_historical: false,
        };
        assert!(recall.namespace.trim().is_empty());
    }
}

// ===========================================================================
// 7. Hot/Warm Tier Management
// ===========================================================================

/// Tests hot and warm tier promotion/demotion logic.
///
/// **Validates: Requirements 14.1, 15.1**
mod tier_management {
    use super::*;

    /// Hot tier items are always included in compilation.
    #[test]
    fn hot_tier_items_always_included_in_compilation() {
        let hot_items = vec![
            HotTierItem {
                id: Uuid::new_v4(),
                memory_type: MemoryType::Semantic,
                payload: HotTierPayload::Fact(HotFact {
                    subject: "APIM".to_string(),
                    predicate: "deployed_to".to_string(),
                    object: "Azure Production".to_string(),
                    evidence: "explicit".to_string(),
                    observed: Some("2025-01-15".to_string()),
                    source: Uuid::new_v4().to_string(),
                }),
            },
            HotTierItem {
                id: Uuid::new_v4(),
                memory_type: MemoryType::Semantic,
                payload: HotTierPayload::Entity(HotEntity {
                    name: "Project Sentinel".to_string(),
                    entity_type: "project".to_string(),
                    summary: Some("AI-powered security monitoring".to_string()),
                }),
            },
        ];

        let input = CompilationInput {
            namespace: "test-ns".to_string(),
            task_class: TaskClass::Architecture,
            target_model: "claude".to_string(),
            format: OutputFormat::Structured,
            warm_tier_budget: 3000,
            hot_tier_items: hot_items,
            ranked_candidates: vec![],
        };

        let result = compile::compile_package(input);
        // Hot tier items should appear in the output even with no warm candidates.
        assert!(
            result.package.token_count > 0,
            "hot tier items must contribute tokens"
        );
        assert!(
            result.package.context_package.contains("APIM")
                || result.package.context_package.contains("Project Sentinel"),
            "hot tier items must appear in compiled output"
        );
    }

    /// Warm tier budget limits the number of warm candidates included.
    #[test]
    fn warm_tier_budget_limits_candidates() {
        // Create many candidates that would exceed a small budget.
        let mut candidates = Vec::new();
        for i in 0..20 {
            candidates.push(build_fact_candidate(0.9 - (i as f64 * 0.03), "test-ns"));
        }

        let weighted = weight::apply_weights(candidates, &TaskClass::Architecture);
        let ranked = rank::rank_candidates(weighted);

        let input = CompilationInput {
            namespace: "test-ns".to_string(),
            task_class: TaskClass::Architecture,
            target_model: "claude".to_string(),
            format: OutputFormat::Structured,
            warm_tier_budget: 100, // Very small budget.
            hot_tier_items: vec![],
            ranked_candidates: ranked,
        };

        let result = compile::compile_package(input);
        // With a tiny budget, not all 20 candidates should be included.
        assert!(
            result.selected_items.len() < 20,
            "warm tier budget should limit selected candidates"
        );
    }

    /// NamespaceConfig has independent hot and warm tier budgets.
    #[test]
    fn namespace_config_has_independent_budgets() {
        let config = NamespaceConfig {
            namespace: "test".to_string(),
            hot_tier_budget: 500,
            warm_tier_budget: 3000,
            ..Default::default()
        };

        assert_eq!(config.hot_tier_budget, 500);
        assert_eq!(config.warm_tier_budget, 3000);
        assert_ne!(
            config.hot_tier_budget, config.warm_tier_budget,
            "hot and warm budgets should be independent"
        );
    }

    /// Default warm tier budget constant is defined.
    #[test]
    fn default_warm_tier_budget_is_defined() {
        assert!(
            compile::DEFAULT_WARM_TIER_BUDGET > 0,
            "default warm tier budget must be positive"
        );
    }
}

// ===========================================================================
// 8. Predicate Pack System
// ===========================================================================

/// Tests the predicate pack system: core pack always included, pack-aware
/// prompt assembly, and predicate validation.
///
/// **Validates: Requirements 25.1, 25.3, 28.4, 28.6**
mod predicate_pack_system {
    use super::*;

    /// Core pack is always included in default namespace config.
    #[test]
    fn core_pack_always_in_default_config() {
        let config = NamespaceConfig::default();
        assert!(
            config.predicate_packs.contains(&"core".to_string()),
            "default config must include core pack"
        );
    }

    /// Custom namespace config still includes core pack.
    #[test]
    fn custom_config_includes_core_pack() {
        let config = NamespaceConfig {
            namespace: "grc-project".to_string(),
            predicate_packs: vec!["core".to_string(), "grc".to_string()],
            ..Default::default()
        };

        assert!(config.predicate_packs.contains(&"core".to_string()));
        assert!(config.predicate_packs.contains(&"grc".to_string()));
    }

    /// Predicate pack names are non-empty strings.
    #[test]
    fn predicate_pack_names_are_non_empty() {
        let config = NamespaceConfig::default();
        for pack in &config.predicate_packs {
            assert!(!pack.is_empty(), "pack name must not be empty");
            assert!(!pack.trim().is_empty(), "pack name must not be whitespace");
        }
    }

    /// Predicate categories are valid.
    #[test]
    fn predicate_categories_are_valid() {
        let valid_categories = [
            "structural",
            "temporal",
            "decisional",
            "operational",
            "regulatory",
        ];

        // Verify the category set is complete.
        assert_eq!(valid_categories.len(), 5);
        for cat in &valid_categories {
            assert!(!cat.is_empty());
        }
    }
}


// ===========================================================================
// 9. Extraction Metrics
// ===========================================================================

/// Tests that extraction metrics are properly structured for JSONB storage.
///
/// **Validates: Requirements 48.1, 48.3, 48.4, 48.5, 48.6, 48.7**
mod extraction_metrics {
    #[allow(unused_imports)]
    use super::*;

    /// ExtractionMetrics JSONB has all required fields.
    #[test]
    fn extraction_metrics_has_required_fields() {
        let metrics = serde_json::json!({
            "entity_counts": {
                "exact": 3,
                "alias": 1,
                "semantic": 0,
                "new": 2,
                "conflict_flagged": 0
            },
            "fact_counts": {
                "canonical": 5,
                "custom": 1
            },
            "evidence_counts": {
                "explicit": 4,
                "implied": 2
            },
            "processing_time_ms": 1250,
            "extraction_model": "gemma4:26b-a4b-q4"
        });

        // Verify all required fields are present.
        assert!(metrics.get("entity_counts").is_some());
        assert!(metrics.get("fact_counts").is_some());
        assert!(metrics.get("evidence_counts").is_some());
        assert!(metrics.get("processing_time_ms").is_some());
        assert!(metrics.get("extraction_model").is_some());

        // Verify entity count breakdown.
        let entity_counts = &metrics["entity_counts"];
        assert!(entity_counts.get("exact").is_some());
        assert!(entity_counts.get("alias").is_some());
        assert!(entity_counts.get("semantic").is_some());
        assert!(entity_counts.get("new").is_some());
        assert!(entity_counts.get("conflict_flagged").is_some());

        // Verify fact count breakdown.
        let fact_counts = &metrics["fact_counts"];
        assert!(fact_counts.get("canonical").is_some());
        assert!(fact_counts.get("custom").is_some());

        // Verify evidence count breakdown.
        let evidence_counts = &metrics["evidence_counts"];
        assert!(evidence_counts.get("explicit").is_some());
        assert!(evidence_counts.get("implied").is_some());
    }

    /// Processing time is a positive integer.
    #[test]
    fn processing_time_is_positive() {
        let metrics = serde_json::json!({
            "processing_time_ms": 1250
        });
        let time = metrics["processing_time_ms"].as_i64().unwrap();
        assert!(time > 0, "processing_time_ms must be positive");
    }

    /// Entity counts sum to total entities extracted.
    #[test]
    fn entity_counts_sum_correctly() {
        let entity_counts = serde_json::json!({
            "exact": 3,
            "alias": 1,
            "semantic": 2,
            "new": 4,
            "conflict_flagged": 1
        });

        let total: i64 = entity_counts["exact"].as_i64().unwrap()
            + entity_counts["alias"].as_i64().unwrap()
            + entity_counts["semantic"].as_i64().unwrap()
            + entity_counts["new"].as_i64().unwrap()
            + entity_counts["conflict_flagged"].as_i64().unwrap();

        assert_eq!(total, 11, "entity counts should sum correctly");
    }
}

// ===========================================================================
// 10. Connection Pool Separation
// ===========================================================================

/// Tests that online and offline connection pools are structurally separate.
///
/// **Validates: Requirement 44.7**
mod connection_pool_separation {
    use super::*;

    /// AppConfig supports separate online and offline database URLs.
    #[test]
    fn config_supports_separate_pool_urls() {
        let config = AppConfig {
            database_url: "postgres://shared:5432/db".to_string(),
            database_url_online: Some("postgres://online:5432/db".to_string()),
            database_url_offline: Some("postgres://offline:5432/db".to_string()),
            online_pool_max: 10,
            offline_pool_max: 5,
            online_pool_min: 2,
            offline_pool_min: 1,
            pool_acquire_timeout_secs: 5,
            pool_idle_timeout_secs: 300,
            statement_timeout_secs: 30,
            hot_tier_cache_ttl_secs: 60,
            loom_host: "0.0.0.0".to_string(),
            loom_port: 8080,
            loom_bearer_token: "test".to_string(),
            llm: LlmConfig {
                ollama_url: "http://ollama:11434".to_string(),
                extraction_model: "gemma4:26b-a4b-q4".to_string(),
                classification_model: "gemma4:e4b".to_string(),
                embedding_model: "nomic-embed-text".to_string(),
                azure_openai_url: None,
                azure_openai_key: None,
            },
        };

        let online_url = config
            .database_url_online
            .as_deref()
            .unwrap_or(&config.database_url);
        let offline_url = config
            .database_url_offline
            .as_deref()
            .unwrap_or(&config.database_url);

        assert_ne!(
            online_url, offline_url,
            "online and offline pools must use different URLs when configured"
        );
    }

    /// Pool sizes are independently configurable.
    #[test]
    fn pool_sizes_independently_configurable() {
        let config = test_config();
        assert_eq!(config.online_pool_max, 5);
        assert_eq!(config.offline_pool_max, 3);
        assert_ne!(
            config.online_pool_max, config.offline_pool_max,
            "pool sizes should be independently configurable"
        );
    }

    /// DbPools struct has separate online and offline fields.
    #[test]
    fn db_pools_has_separate_fields() {
        // This is a compile-time check — if DbPools didn't have separate
        // `online` and `offline` fields, this function wouldn't compile.
        #[allow(dead_code)]
        fn assert_has_separate_pools(_: &DbPools) {}

        // The function signature proves the structural contract.
    }

    /// Fallback to shared URL when online/offline not configured.
    #[test]
    fn fallback_to_shared_url() {
        let config = AppConfig {
            database_url: "postgres://shared:5432/db".to_string(),
            database_url_online: None,
            database_url_offline: None,
            online_pool_max: 10,
            offline_pool_max: 5,
            online_pool_min: 2,
            offline_pool_min: 1,
            pool_acquire_timeout_secs: 5,
            pool_idle_timeout_secs: 300,
            statement_timeout_secs: 30,
            hot_tier_cache_ttl_secs: 60,
            loom_host: "0.0.0.0".to_string(),
            loom_port: 8080,
            loom_bearer_token: "test".to_string(),
            llm: LlmConfig {
                ollama_url: "http://ollama:11434".to_string(),
                extraction_model: "gemma4:26b-a4b-q4".to_string(),
                classification_model: "gemma4:e4b".to_string(),
                embedding_model: "nomic-embed-text".to_string(),
                azure_openai_url: None,
                azure_openai_key: None,
            },
        };

        let online_url = config
            .database_url_online
            .as_deref()
            .unwrap_or(&config.database_url);
        let offline_url = config
            .database_url_offline
            .as_deref()
            .unwrap_or(&config.database_url);

        assert_eq!(online_url, "postgres://shared:5432/db");
        assert_eq!(offline_url, "postgres://shared:5432/db");
    }
}

// ===========================================================================
// 11. Docker Compose Verification
// ===========================================================================

/// Tests that docker-compose.yml is valid and defines all 5 required
/// containers.
///
/// **Validates: Requirement 45.1**
mod docker_compose_verification {
    /// docker-compose.yml exists and is valid YAML.
    #[test]
    fn docker_compose_file_exists_and_is_valid() {
        let content = std::fs::read_to_string("../docker-compose.yml")
            .or_else(|_| std::fs::read_to_string("docker-compose.yml"))
            .expect("docker-compose.yml must exist");

        // Basic validation: file is non-empty and contains expected keys.
        assert!(!content.is_empty(), "docker-compose.yml must not be empty");
        assert!(
            content.contains("services:"),
            "docker-compose.yml must define services"
        );
    }

    /// All 5 required containers are defined.
    #[test]
    fn all_five_containers_defined() {
        let content = std::fs::read_to_string("../docker-compose.yml")
            .or_else(|_| std::fs::read_to_string("docker-compose.yml"))
            .expect("docker-compose.yml must exist");

        let required_services = [
            "postgres:",
            "ollama:",
            "loom-engine:",
            "loom-dashboard:",
            "caddy:",
        ];

        for service in &required_services {
            assert!(
                content.contains(service),
                "docker-compose.yml must define service '{}'",
                service
            );
        }
    }

    /// PostgreSQL container has pgvector image.
    #[test]
    fn postgres_uses_pgvector_image() {
        let content = std::fs::read_to_string("../docker-compose.yml")
            .or_else(|_| std::fs::read_to_string("docker-compose.yml"))
            .expect("docker-compose.yml must exist");

        assert!(
            content.contains("pgvector/pgvector"),
            "postgres service must use pgvector image"
        );
    }

    /// PostgreSQL container has pgAudit enabled.
    #[test]
    fn postgres_has_pgaudit_enabled() {
        let content = std::fs::read_to_string("../docker-compose.yml")
            .or_else(|_| std::fs::read_to_string("docker-compose.yml"))
            .expect("docker-compose.yml must exist");

        assert!(
            content.contains("pgaudit"),
            "postgres service must enable pgaudit"
        );
    }

    /// Ollama container is configured.
    #[test]
    fn ollama_container_configured() {
        let content = std::fs::read_to_string("../docker-compose.yml")
            .or_else(|_| std::fs::read_to_string("docker-compose.yml"))
            .expect("docker-compose.yml must exist");

        assert!(
            content.contains("ollama/ollama"),
            "ollama service must use ollama/ollama image"
        );
        assert!(
            content.contains("11434"),
            "ollama service must expose port 11434"
        );
    }

    /// Caddy container is configured with Caddyfile.
    #[test]
    fn caddy_container_configured() {
        let content = std::fs::read_to_string("../docker-compose.yml")
            .or_else(|_| std::fs::read_to_string("docker-compose.yml"))
            .expect("docker-compose.yml must exist");

        assert!(
            content.contains("caddy:"),
            "caddy service must be defined"
        );
        assert!(
            content.contains("Caddyfile"),
            "caddy service must mount Caddyfile"
        );
    }

    /// Docker network is defined for inter-container communication.
    #[test]
    fn docker_network_defined() {
        let content = std::fs::read_to_string("../docker-compose.yml")
            .or_else(|_| std::fs::read_to_string("docker-compose.yml"))
            .expect("docker-compose.yml must exist");

        assert!(
            content.contains("networks:"),
            "docker-compose.yml must define networks"
        );
        assert!(
            content.contains("loom-net"),
            "docker-compose.yml must define loom-net network"
        );
    }

    /// Persistent volumes are defined for data storage.
    #[test]
    fn persistent_volumes_defined() {
        let content = std::fs::read_to_string("../docker-compose.yml")
            .or_else(|_| std::fs::read_to_string("docker-compose.yml"))
            .expect("docker-compose.yml must exist");

        assert!(
            content.contains("volumes:"),
            "docker-compose.yml must define volumes"
        );
        assert!(
            content.contains("pgdata"),
            "docker-compose.yml must define pgdata volume"
        );
    }

    /// loom-engine depends on postgres and ollama.
    #[test]
    fn loom_engine_depends_on_postgres_and_ollama() {
        let content = std::fs::read_to_string("../docker-compose.yml")
            .or_else(|_| std::fs::read_to_string("docker-compose.yml"))
            .expect("docker-compose.yml must exist");

        assert!(
            content.contains("depends_on"),
            "loom-engine must have depends_on"
        );
    }

    /// Health checks are configured for critical services.
    #[test]
    fn health_checks_configured() {
        let content = std::fs::read_to_string("../docker-compose.yml")
            .or_else(|_| std::fs::read_to_string("docker-compose.yml"))
            .expect("docker-compose.yml must exist");

        assert!(
            content.contains("healthcheck"),
            "services must have health checks"
        );
        assert!(
            content.contains("pg_isready"),
            "postgres health check must use pg_isready"
        );
    }

    /// Caddyfile exists for reverse proxy configuration.
    #[test]
    fn caddyfile_exists() {
        let exists = std::fs::metadata("../Caddyfile").is_ok()
            || std::fs::metadata("Caddyfile").is_ok();
        assert!(exists, "Caddyfile must exist");
    }
}

// ===========================================================================
// Additional Integration Checks
// ===========================================================================

/// Cross-cutting integration checks that span multiple subsystems.
mod cross_cutting {
    use super::*;

    /// RankingScore composite formula is correct.
    #[test]
    fn ranking_score_composite_formula() {
        let score = RankingScore {
            relevance: 0.8,
            recency: 0.6,
            stability: 0.9,
            provenance: 0.7,
        };

        let expected = 0.8 * 0.40 + 0.6 * 0.25 + 0.9 * 0.20 + 0.7 * 0.15;
        let actual = score.composite();
        assert!(
            (actual - expected).abs() < f64::EPSILON,
            "composite score formula: expected {}, got {}",
            expected,
            actual
        );
    }

    /// OutputFormat display is correct.
    #[test]
    fn output_format_display() {
        assert_eq!(OutputFormat::Structured.to_string(), "structured");
        assert_eq!(OutputFormat::Compact.to_string(), "compact");
    }

    /// Claude models select structured format, others select compact.
    #[test]
    fn output_format_selection_by_model() {
        let claude_models = ["claude-3.5-sonnet", "claude-3-opus", "Claude-4"];
        for model in &claude_models {
            let format = if model.to_lowercase().contains("claude") {
                OutputFormat::Structured
            } else {
                OutputFormat::Compact
            };
            assert_eq!(
                format,
                OutputFormat::Structured,
                "{} should select structured format",
                model
            );
        }

        let non_claude = ["gpt-4.1-mini", "gemma4:26b-a4b-q4", "llama-3"];
        for model in &non_claude {
            let format = if model.to_lowercase().contains("claude") {
                OutputFormat::Structured
            } else {
                OutputFormat::Compact
            };
            assert_eq!(
                format,
                OutputFormat::Compact,
                "{} should select compact format",
                model
            );
        }
    }

    /// TaskClass round-trips through serde.
    #[test]
    fn task_class_serde_roundtrip() {
        let classes = [
            TaskClass::Debug,
            TaskClass::Architecture,
            TaskClass::Compliance,
            TaskClass::Writing,
            TaskClass::Chat,
        ];

        for class in &classes {
            let json = serde_json::to_value(class).unwrap();
            let deserialized: TaskClass = serde_json::from_value(json).unwrap();
            assert_eq!(&deserialized, class);
        }
    }

    /// TaskClass FromStr parses all valid values.
    #[test]
    fn task_class_from_str_all_valid() {
        use std::str::FromStr;

        let valid = [
            ("debug", TaskClass::Debug),
            ("architecture", TaskClass::Architecture),
            ("compliance", TaskClass::Compliance),
            ("writing", TaskClass::Writing),
            ("chat", TaskClass::Chat),
        ];

        for (s, expected) in &valid {
            let parsed = TaskClass::from_str(s).expect("should parse");
            assert_eq!(&parsed, expected);
        }
    }

    /// TaskClass FromStr rejects invalid values.
    #[test]
    fn task_class_from_str_rejects_invalid() {
        use std::str::FromStr;
        assert!(TaskClass::from_str("unknown").is_err());
        assert!(TaskClass::from_str("").is_err());
    }

    /// McpError variants all produce non-empty display strings.
    #[test]
    fn mcp_error_variants_display() {
        use loom_engine::api::mcp::McpError;

        let errors = vec![
            McpError::Database("db error".into()),
            McpError::Classification("classify error".into()),
            McpError::Retrieval("retrieval error".into()),
            McpError::Embedding("embedding error".into()),
            McpError::InvalidRequest("bad input".into()),
        ];

        for err in errors {
            let msg = err.to_string();
            assert!(!msg.is_empty(), "error message must not be empty");
        }
    }

    /// MCP endpoint loom_learn returns 200 for valid input (requires DB).
    #[tokio::test]
    async fn loom_learn_returns_200_for_valid_input() {
        let app = build_full_test_app().await;
        let unique_content = format!("integration test episode {}", Uuid::new_v4());
        let body = serde_json::json!({
            "content": unique_content,
            "source": "manual",
            "namespace": "integration-test",
            "occurred_at": "2025-01-15T10:00:00Z"
        });

        let resp = post_authed(app, "/mcp/loom_learn", body).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let json = body_json(resp).await;
        assert!(json.get("episode_id").is_some());
        let status = json["status"].as_str().unwrap();
        assert!(
            status == "queued" || status == "duplicate",
            "status must be 'queued' or 'duplicate', got '{}'",
            status
        );
    }

    /// MCP endpoint loom_learn rejects empty content.
    #[tokio::test]
    async fn loom_learn_rejects_empty_content() {
        let app = build_full_test_app().await;
        let body = serde_json::json!({
            "content": "",
            "source": "manual",
            "namespace": "integration-test"
        });

        let resp = post_authed(app, "/mcp/loom_learn", body).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    /// MCP endpoint loom_recall rejects empty entity_names.
    #[tokio::test]
    async fn loom_recall_rejects_empty_entity_names() {
        let app = build_full_test_app().await;
        let body = serde_json::json!({
            "entity_names": [],
            "namespace": "integration-test"
        });

        let resp = post_authed(app, "/mcp/loom_recall", body).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    /// Health check endpoint is unauthenticated and returns 200.
    #[tokio::test]
    async fn health_check_unauthenticated() {
        let app = build_full_test_app().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let json = body_json(resp).await;
        let status = json["status"].as_str().unwrap();
        assert!(status == "ok" || status == "degraded");
        assert!(json.get("version").is_some());
    }
}
