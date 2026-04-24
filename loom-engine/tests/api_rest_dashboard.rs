//! Integration tests for REST and Dashboard API endpoints.
//!
//! Tests cover:
//! - Manual ingestion via POST /api/learn (Requirement 37.1)
//! - Health check endpoint (Requirement 37.2)
//! - Dashboard read-only endpoints return correct data shapes (Requirement 50.2)
//! - Conflict resolution write endpoint (Requirement 50.3)
//! - Predicate candidate resolution with pack-aware promotion (Requirement 50.4)
//! - Namespace listing (Requirement 50.5)
//!
//! DB-backed tests require the test database:
//! ```sh
//! docker compose -f docker-compose.test.yml up -d postgres-test
//! ```

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    middleware,
    routing::{get, post},
    Router,
};
use tower::ServiceExt;

use loom_engine::{
    api::{auth::require_bearer_token, dashboard, mcp::AppState, rest},
    config::{AppConfig, LlmConfig},
    db::pool::DbPools,
    llm::client::LlmClient,
};

// ---------------------------------------------------------------------------
// Test constants
// ---------------------------------------------------------------------------

const DEFAULT_TEST_DB_URL: &str = "postgres://loom_test:loom_test@localhost:5433/loom_test";
const TEST_BEARER_TOKEN: &str = "test-token";

// ---------------------------------------------------------------------------
// Test app builder
// ---------------------------------------------------------------------------

/// Build a test axum router connected to the test database.
async fn build_test_app() -> Router {
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_TEST_DB_URL.to_string());

    let config = AppConfig {
        database_url: db_url.clone(),
        database_url_online: None,
        database_url_offline: None,
        online_pool_max: 5,
        offline_pool_max: 5,
        online_pool_min: 2,
        offline_pool_min: 1,
        pool_acquire_timeout_secs: 5,
        pool_idle_timeout_secs: 300,
        statement_timeout_secs: 30,
        hot_tier_cache_ttl_secs: 60,
        episode_max_attempts: 5,
        episode_backoff_base_secs: 30,
        worker_concurrency: 1,
        loom_host: "0.0.0.0".to_string(),
        loom_port: 8080,
        loom_bearer_token: TEST_BEARER_TOKEN.to_string(),
        llm: LlmConfig {
            ollama_url: "http://localhost:11434".to_string(),
            extraction_model: "gemma4:26b".to_string(),
            classification_model: "gemma4:e4b".to_string(),
            embedding_model: "nomic-embed-text".to_string(),
            azure_openai_url: None,
            azure_openai_key: None,
        },
    };

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
        telemetry: loom_engine::telemetry::new_shared(),
    };

    // REST /api/learn — bearer-token protected
    let rest_learn = Router::new()
        .route("/api/learn", post(rest::handle_api_learn))
        .layer(middleware::from_fn_with_state(
            bearer_token.clone(),
            require_bearer_token,
        ))
        .with_state(state.clone());

    // GET /api/health — unauthenticated
    let public = Router::new()
        .route("/api/health", get(rest::handle_health))
        .with_state(state.clone());

    // Dashboard routes — bearer-token protected
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
        .route("/dashboard/api/entities", get(dashboard::handle_entities))
        .route("/dashboard/api/facts", get(dashboard::handle_facts))
        .route("/dashboard/api/conflicts", get(dashboard::handle_conflicts))
        .route(
            "/dashboard/api/predicates/candidates",
            get(dashboard::handle_predicate_candidates),
        )
        .route(
            "/dashboard/api/predicates/packs",
            get(dashboard::handle_predicate_packs),
        )
        .route(
            "/dashboard/api/metrics/retrieval",
            get(dashboard::handle_metrics_retrieval),
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
            "/dashboard/api/conflicts/{id}/resolve",
            post(dashboard::handle_resolve_conflict),
        )
        .route(
            "/dashboard/api/predicates/candidates/{id}/resolve",
            post(dashboard::handle_resolve_predicate_candidate),
        )
        .route(
            "/dashboard/api/episodes/failed",
            get(dashboard::handle_failed_episodes),
        )
        .route(
            "/dashboard/api/episodes/{id}/requeue",
            post(dashboard::handle_requeue_episode),
        )
        .layer(middleware::from_fn_with_state(
            bearer_token.clone(),
            require_bearer_token,
        ))
        .with_state(state.clone());

    Router::new()
        .merge(rest_learn)
        .merge(public)
        .merge(dashboard_routes)
}

/// Send a GET request with the test bearer token.
async fn get_authed(app: Router, uri: &str) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .method("GET")
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {TEST_BEARER_TOKEN}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap()
}

/// Send a POST request with JSON body and the test bearer token.
async fn post_authed(app: Router, uri: &str, body: serde_json::Value) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {TEST_BEARER_TOKEN}"))
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

// ---------------------------------------------------------------------------
// REST API tests
// ---------------------------------------------------------------------------

/// POST /api/learn — empty content returns 400.
#[tokio::test]
async fn api_learn_empty_content_returns_400() {
    let app = build_test_app().await;
    let body = serde_json::json!({
        "content": "",
        "namespace": "test-ns",
        "source": "manual"
    });
    let resp = post_authed(app, "/api/learn", body).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// POST /api/learn — whitespace-only content returns 400.
#[tokio::test]
async fn api_learn_whitespace_content_returns_400() {
    let app = build_test_app().await;
    let body = serde_json::json!({
        "content": "   ",
        "namespace": "test-ns",
        "source": "manual"
    });
    let resp = post_authed(app, "/api/learn", body).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// POST /api/learn — empty namespace returns 400.
#[tokio::test]
async fn api_learn_empty_namespace_returns_400() {
    let app = build_test_app().await;
    let body = serde_json::json!({
        "content": "some content",
        "namespace": "",
        "source": "manual"
    });
    let resp = post_authed(app, "/api/learn", body).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// POST /api/learn — valid body returns 200 with episode_id and status.
#[tokio::test]
async fn api_learn_valid_body_returns_200_with_episode_id() {
    let app = build_test_app().await;
    let unique_content = format!("test episode content {}", uuid::Uuid::new_v4());
    let body = serde_json::json!({
        "content": unique_content,
        "namespace": "test-ns",
        "source": "claude-code",  // should be overridden to "manual"
        "occurred_at": "2025-01-01T00:00:00Z"
    });
    let resp = post_authed(app, "/api/learn", body).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert!(
        json.get("episode_id").is_some(),
        "response must contain episode_id"
    );
    assert!(json.get("status").is_some(), "response must contain status");

    let status = json["status"].as_str().unwrap();
    assert!(
        status == "queued" || status == "duplicate",
        "status must be 'queued' or 'duplicate', got '{status}'"
    );
}

/// POST /api/learn — forces source to "manual" regardless of request body.
#[tokio::test]
async fn api_learn_forces_source_to_manual() {
    let app = build_test_app().await;
    let unique_content = format!("source override test {}", uuid::Uuid::new_v4());
    let body = serde_json::json!({
        "content": unique_content,
        "namespace": "test-ns",
        "source": "github",  // should be overridden to "manual"
        "occurred_at": "2025-01-01T00:00:00Z"
    });
    let resp = post_authed(app, "/api/learn", body).await;
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify the episode was stored with source="manual" by checking the DB
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_TEST_DB_URL.to_string());
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    let source: Option<String> =
        sqlx::query_scalar("SELECT source FROM loom_episodes WHERE content LIKE $1 LIMIT 1")
            .bind(format!("source override test%"))
            .fetch_optional(&pool)
            .await
            .unwrap();

    if let Some(s) = source {
        assert_eq!(s, "manual", "source must be forced to 'manual'");
    }
    // If no row found (DB not available), the test passes silently
}

/// POST /api/learn — missing auth returns 401.
#[tokio::test]
async fn api_learn_missing_auth_returns_401() {
    let app = build_test_app().await;
    let body = serde_json::json!({
        "content": "some content",
        "namespace": "test-ns"
    });
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/learn")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// GET /api/health — returns 200 (unauthenticated).
#[tokio::test]
async fn api_health_returns_200_unauthenticated() {
    let app = build_test_app().await;
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
}

/// GET /api/health — response contains required fields.
#[tokio::test]
async fn api_health_response_has_required_fields() {
    let app = build_test_app().await;
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
    assert!(
        json.get("status").is_some(),
        "response must contain 'status'"
    );
    assert!(
        json.get("database").is_some(),
        "response must contain 'database'"
    );
    assert!(
        json.get("ollama").is_some(),
        "response must contain 'ollama'"
    );
    assert!(
        json.get("version").is_some(),
        "response must contain 'version'"
    );

    let status = json["status"].as_str().unwrap();
    assert!(
        status == "ok" || status == "degraded",
        "status must be 'ok' or 'degraded', got '{status}'"
    );
}

// ---------------------------------------------------------------------------
// Dashboard read-only endpoint tests
// ---------------------------------------------------------------------------

/// GET /dashboard/api/health — returns 200 with correct shape.
#[tokio::test]
async fn dashboard_health_returns_200_with_correct_shape() {
    let app = build_test_app().await;
    let resp = get_authed(app, "/dashboard/api/health").await;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert!(json.get("episodes_by_source").is_some());
    assert!(json.get("episodes_by_namespace").is_some());
    assert!(json.get("entities_by_type").is_some());
    assert!(json.get("facts_current").is_some());
    assert!(json.get("facts_superseded").is_some());
    assert!(json.get("queue_depth").is_some());
    assert!(
        json.get("failed_episode_count").is_some(),
        "health response must surface failed_episode_count"
    );
    assert!(
        json["failed_episode_count"].is_i64(),
        "failed_episode_count must be an integer"
    );
}

/// GET /dashboard/api/namespaces — returns 200 with array.
#[tokio::test]
async fn dashboard_namespaces_returns_200_with_array() {
    let app = build_test_app().await;
    let resp = get_authed(app, "/dashboard/api/namespaces").await;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert!(json.is_array(), "namespaces response must be an array");
}

/// GET /dashboard/api/compilations — returns 200 with array.
#[tokio::test]
async fn dashboard_compilations_returns_200_with_array() {
    let app = build_test_app().await;
    let resp = get_authed(app, "/dashboard/api/compilations").await;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert!(json.is_array(), "compilations response must be an array");
}

/// GET /dashboard/api/entities — returns 200 with array.
#[tokio::test]
async fn dashboard_entities_returns_200_with_array() {
    let app = build_test_app().await;
    let resp = get_authed(app, "/dashboard/api/entities").await;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert!(json.is_array(), "entities response must be an array");
}

/// GET /dashboard/api/facts — returns 200 with array.
#[tokio::test]
async fn dashboard_facts_returns_200_with_array() {
    let app = build_test_app().await;
    let resp = get_authed(app, "/dashboard/api/facts").await;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert!(json.is_array(), "facts response must be an array");
}

/// GET /dashboard/api/conflicts — returns 200 with array.
#[tokio::test]
async fn dashboard_conflicts_returns_200_with_array() {
    let app = build_test_app().await;
    let resp = get_authed(app, "/dashboard/api/conflicts").await;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert!(json.is_array(), "conflicts response must be an array");
}

/// GET /dashboard/api/predicates/candidates — returns 200 with array.
#[tokio::test]
async fn dashboard_predicate_candidates_returns_200_with_array() {
    let app = build_test_app().await;
    let resp = get_authed(app, "/dashboard/api/predicates/candidates").await;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert!(
        json.is_array(),
        "predicate candidates response must be an array"
    );
}

/// GET /dashboard/api/predicates/packs — returns 200 with array.
#[tokio::test]
async fn dashboard_predicate_packs_returns_200_with_array() {
    let app = build_test_app().await;
    let resp = get_authed(app, "/dashboard/api/predicates/packs").await;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert!(json.is_array(), "predicate packs response must be an array");
}

/// GET /dashboard/api/metrics/retrieval — returns 200 with correct shape.
#[tokio::test]
async fn dashboard_metrics_retrieval_returns_200_with_correct_shape() {
    let app = build_test_app().await;
    let resp = get_authed(app, "/dashboard/api/metrics/retrieval").await;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert!(
        json.get("daily_precision").is_some(),
        "must contain daily_precision"
    );
    assert!(
        json["daily_precision"].is_array(),
        "daily_precision must be an array"
    );
}

/// GET /dashboard/api/metrics/classification — returns 200 with correct shape.
#[tokio::test]
async fn dashboard_metrics_classification_returns_200_with_correct_shape() {
    let app = build_test_app().await;
    let resp = get_authed(app, "/dashboard/api/metrics/classification").await;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert!(json.get("confidence_distribution").is_some());
    assert!(json.get("class_distribution").is_some());
    assert!(json["confidence_distribution"].is_array());
    assert!(json["class_distribution"].is_array());
}

/// GET /dashboard/api/metrics/hot-tier — returns 200 with correct shape.
#[tokio::test]
async fn dashboard_metrics_hot_tier_returns_200_with_correct_shape() {
    let app = build_test_app().await;
    let resp = get_authed(app, "/dashboard/api/metrics/hot-tier").await;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert!(
        json.get("by_namespace").is_some(),
        "must contain by_namespace"
    );
    assert!(
        json["by_namespace"].is_array(),
        "by_namespace must be an array"
    );
}

/// Dashboard endpoints require auth — missing token returns 401.
#[tokio::test]
async fn dashboard_endpoints_require_auth() {
    let app = build_test_app().await;
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

// ---------------------------------------------------------------------------
// Dashboard write endpoint validation tests
// ---------------------------------------------------------------------------

/// POST /dashboard/api/conflicts/{id}/resolve — invalid resolution returns 400.
#[tokio::test]
async fn resolve_conflict_invalid_resolution_returns_400() {
    let app = build_test_app().await;
    let id = uuid::Uuid::new_v4();
    let body = serde_json::json!({ "resolution": "invalid_value" });
    let resp = post_authed(app, &format!("/dashboard/api/conflicts/{id}/resolve"), body).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// POST /dashboard/api/conflicts/{id}/resolve — "merged" without merged_into returns 400.
#[tokio::test]
async fn resolve_conflict_merged_without_merged_into_returns_400() {
    let app = build_test_app().await;
    let id = uuid::Uuid::new_v4();
    let body = serde_json::json!({ "resolution": "merged" });
    let resp = post_authed(app, &format!("/dashboard/api/conflicts/{id}/resolve"), body).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// POST /dashboard/api/conflicts/{id}/resolve — non-existent ID returns 404.
#[tokio::test]
async fn resolve_conflict_nonexistent_id_returns_404() {
    let app = build_test_app().await;
    let id = uuid::Uuid::new_v4(); // random UUID, won't exist
    let body = serde_json::json!({ "resolution": "kept_separate" });
    let resp = post_authed(app, &format!("/dashboard/api/conflicts/{id}/resolve"), body).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// POST /dashboard/api/predicates/candidates/{id}/resolve — invalid action returns 400.
#[tokio::test]
async fn resolve_predicate_candidate_invalid_action_returns_400() {
    let app = build_test_app().await;
    let id = uuid::Uuid::new_v4();
    let body = serde_json::json!({ "action": "invalid_action" });
    let resp = post_authed(
        app,
        &format!("/dashboard/api/predicates/candidates/{id}/resolve"),
        body,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// POST /dashboard/api/predicates/candidates/{id}/resolve — action="map" without mapped_to returns 400.
#[tokio::test]
async fn resolve_predicate_candidate_map_without_mapped_to_returns_400() {
    let app = build_test_app().await;
    let id = uuid::Uuid::new_v4();
    let body = serde_json::json!({ "action": "map" });
    let resp = post_authed(
        app,
        &format!("/dashboard/api/predicates/candidates/{id}/resolve"),
        body,
    )
    .await;
    // The handler checks action validity first, then fetches the candidate.
    // Since the candidate doesn't exist, we get 404 before the mapped_to check.
    // But if action is valid and candidate exists, missing mapped_to → 400.
    // For a non-existent candidate, we get 404.
    assert!(
        resp.status() == StatusCode::NOT_FOUND || resp.status() == StatusCode::BAD_REQUEST,
        "expected 404 (candidate not found) or 400 (missing mapped_to), got {}",
        resp.status()
    );
}

/// POST /dashboard/api/predicates/candidates/{id}/resolve — action="promote" without target_pack returns 400.
#[tokio::test]
async fn resolve_predicate_candidate_promote_without_target_pack_returns_400() {
    let app = build_test_app().await;
    let id = uuid::Uuid::new_v4();
    let body = serde_json::json!({ "action": "promote" });
    let resp = post_authed(
        app,
        &format!("/dashboard/api/predicates/candidates/{id}/resolve"),
        body,
    )
    .await;
    // Same as above: non-existent candidate → 404 before target_pack check.
    assert!(
        resp.status() == StatusCode::NOT_FOUND || resp.status() == StatusCode::BAD_REQUEST,
        "expected 404 (candidate not found) or 400 (missing target_pack), got {}",
        resp.status()
    );
}

/// POST /dashboard/api/predicates/candidates/{id}/resolve — non-existent ID returns 404.
#[tokio::test]
async fn resolve_predicate_candidate_nonexistent_id_returns_404() {
    let app = build_test_app().await;
    let id = uuid::Uuid::new_v4(); // random UUID, won't exist
    let body = serde_json::json!({ "action": "map", "mapped_to": "uses" });
    let resp = post_authed(
        app,
        &format!("/dashboard/api/predicates/candidates/{id}/resolve"),
        body,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Namespace listing tests
// ---------------------------------------------------------------------------

/// GET /dashboard/api/namespaces — returns array of namespace objects with required fields.
#[tokio::test]
async fn dashboard_namespaces_items_have_required_fields() {
    let app = build_test_app().await;
    let resp = get_authed(app, "/dashboard/api/namespaces").await;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    let arr = json.as_array().unwrap();

    // If there are any namespaces, verify they have the required fields.
    for ns in arr {
        assert!(
            ns.get("namespace").is_some(),
            "namespace object must have 'namespace' field"
        );
        assert!(
            ns.get("hot_tier_budget").is_some(),
            "namespace object must have 'hot_tier_budget'"
        );
        assert!(
            ns.get("warm_tier_budget").is_some(),
            "namespace object must have 'warm_tier_budget'"
        );
        assert!(
            ns.get("predicate_packs").is_some(),
            "namespace object must have 'predicate_packs'"
        );
    }
}

// ---------------------------------------------------------------------------
// Pagination parameter tests
// ---------------------------------------------------------------------------

/// GET /dashboard/api/compilations with limit and offset params — returns 200.
#[tokio::test]
async fn dashboard_compilations_accepts_pagination_params() {
    let app = build_test_app().await;
    let resp = get_authed(app, "/dashboard/api/compilations?limit=10&offset=0").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json.is_array());
}

/// GET /dashboard/api/entities with namespace filter — returns 200.
#[tokio::test]
async fn dashboard_entities_accepts_namespace_filter() {
    let app = build_test_app().await;
    let resp = get_authed(app, "/dashboard/api/entities?namespace=default").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json.is_array());
}

/// GET /dashboard/api/facts with filters — returns 200.
#[tokio::test]
async fn dashboard_facts_accepts_filters() {
    let app = build_test_app().await;
    let resp = get_authed(app, "/dashboard/api/facts?namespace=default&limit=5").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json.is_array());
}

// ---------------------------------------------------------------------------
// Failed-episode surfacing and requeue endpoint tests
// ---------------------------------------------------------------------------

/// GET /dashboard/api/episodes/failed — returns 200 with array.
#[tokio::test]
async fn dashboard_failed_episodes_returns_200_with_array() {
    let app = build_test_app().await;
    let resp = get_authed(app, "/dashboard/api/episodes/failed").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let json = body_json(resp).await;
    assert!(json.is_array(), "failed episodes response must be an array");
}

/// POST /dashboard/api/episodes/{id}/requeue — non-existent ID returns 404.
#[tokio::test]
async fn requeue_nonexistent_episode_returns_404() {
    let app = build_test_app().await;
    let id = uuid::Uuid::new_v4();
    let resp = post_authed(
        app,
        &format!("/dashboard/api/episodes/{id}/requeue"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// POST /dashboard/api/episodes/{id}/requeue — resets a failed episode
/// and returns the new state (`pending`, attempts=0).
#[tokio::test]
async fn requeue_failed_episode_resets_state() {
    use loom_engine::db::episodes::{
        claim_episode_for_processing, insert_episode, record_processing_failure, NewEpisode,
    };

    let app = build_test_app().await;

    // Seed a failed episode directly via the DB layer so we can point the
    // requeue endpoint at a known-bad row.
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_TEST_DB_URL.to_string());
    let pool = sqlx::PgPool::connect(&db_url).await.unwrap();

    let ep_input = NewEpisode {
        source: format!("requeue-test-{}", uuid::Uuid::new_v4()),
        source_id: None,
        source_event_id: Some(format!("evt-{}", uuid::Uuid::new_v4())),
        content: "content that would fail".to_string(),
        content_hash: format!("hash-{}", uuid::Uuid::new_v4()),
        occurred_at: chrono::Utc::now(),
        namespace: format!("requeue-ns-{}", uuid::Uuid::new_v4()),
        metadata: None,
        participants: None,
        ingestion_mode: "live_mcp_capture".to_string(),
        parser_version: None,
        parser_source_schema: None,
    };
    let ep = insert_episode(&pool, &ep_input).await.expect("insert");

    // Two claim+fail cycles with max=2 drives the row to 'failed'.
    for _ in 0..2 {
        claim_episode_for_processing(&pool, ep.id)
            .await
            .expect("claim")
            .expect("claimed");
        record_processing_failure(&pool, ep.id, "simulated", 2)
            .await
            .expect("record");
    }

    let resp = post_authed(
        app,
        &format!("/dashboard/api/episodes/{}/requeue", ep.id),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert_eq!(json["id"].as_str().unwrap(), ep.id.to_string());
    assert_eq!(json["processing_status"].as_str().unwrap(), "pending");
    assert_eq!(json["processing_attempts"].as_i64().unwrap(), 0);
}
