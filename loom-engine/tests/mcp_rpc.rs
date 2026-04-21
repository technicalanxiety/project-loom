//! Integration tests for the MCP JSON-RPC 2.0 dispatcher at `POST /mcp`.
//!
//! These tests exercise the router end-to-end so we verify axum extraction,
//! middleware order, and JSON-RPC envelope shapes together. Methods that do
//! not touch the database (`initialize`, `tools/list`, `ping`, error paths)
//! use a lazy pool that never connects; methods that dispatch into real
//! handlers (`tools/call`) require the test DB from `docker-compose.test.yml`.

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    middleware,
    routing::post,
    Router,
};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

use loom_engine::{
    api::{
        auth::require_bearer_token,
        mcp::AppState,
        mcp_rpc::handle_mcp_rpc,
    },
    config::{AppConfig, LlmConfig},
    db::pool::DbPools,
    llm::client::LlmClient,
};

const TEST_BEARER_TOKEN: &str = "test-token";

// ---------------------------------------------------------------------------
// App builders
// ---------------------------------------------------------------------------

/// Build an app with the `/mcp` JSON-RPC route using a lazy pool that never
/// actually connects. Suitable for methods that never hit the DB —
/// `initialize`, `tools/list`, `ping`, and all validation error paths.
fn app_no_db() -> Router {
    let lazy_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_lazy("postgres://invalid:5432/nonexistent")
        .expect("connect_lazy should never fail at construction time");

    let pools = DbPools {
        online: lazy_pool.clone(),
        offline: lazy_pool,
    };

    let config = AppConfig {
        database_url: "postgres://invalid:5432/nonexistent".into(),
        database_url_online: None,
        database_url_offline: None,
        online_pool_max: 1,
        offline_pool_max: 1,
        online_pool_min: 1,
        offline_pool_min: 1,
        pool_acquire_timeout_secs: 5,
        pool_idle_timeout_secs: 300,
        statement_timeout_secs: 30,
        hot_tier_cache_ttl_secs: 60,
        episode_max_attempts: 5,
        episode_backoff_base_secs: 30,
        loom_host: "0.0.0.0".into(),
        loom_port: 8080,
        loom_bearer_token: TEST_BEARER_TOKEN.into(),
        llm: LlmConfig {
            ollama_url: "http://localhost:11434".into(),
            extraction_model: "test".into(),
            classification_model: "test".into(),
            embedding_model: "nomic-embed-text".into(),
            azure_openai_url: None,
            azure_openai_key: None,
        },
    };

    let llm_client = LlmClient::new(&config.llm).expect("LLM client");
    let state = AppState {
        pools,
        llm_client,
        config,
    };

    Router::new()
        .route("/mcp", post(handle_mcp_rpc))
        .layer(middleware::from_fn_with_state(
            TEST_BEARER_TOKEN.to_string(),
            require_bearer_token,
        ))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

async fn post_rpc(app: Router, body: Value) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/mcp")
            .header(header::AUTHORIZATION, format!("Bearer {TEST_BEARER_TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap(),
    )
    .await
    .unwrap()
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

// ===========================================================================
// initialize
// ===========================================================================

#[tokio::test]
async fn initialize_returns_protocol_handshake() {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": { "name": "test-client", "version": "0.0.0" }
        }
    });

    let resp = post_rpc(app_no_db(), body).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert_eq!(json["jsonrpc"], "2.0");
    assert_eq!(json["id"], 0);
    assert!(json["error"].is_null(), "error must be absent on success");

    let result = &json["result"];
    // Client sent 2025-11-25; server echoes.
    assert_eq!(result["protocolVersion"], "2025-11-25");
    assert_eq!(result["serverInfo"]["name"], "loom");
    assert!(result["capabilities"]["tools"].is_object());
}

#[tokio::test]
async fn initialize_uses_default_version_when_client_omits() {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "capabilities": {} }
    });

    let resp = post_rpc(app_no_db(), body).await;
    let json = body_json(resp).await;
    assert!(
        json["result"]["protocolVersion"]
            .as_str()
            .unwrap()
            .starts_with("2025-"),
        "default version should be a 2025-MM-DD string"
    );
}

// ===========================================================================
// tools/list
// ===========================================================================

#[tokio::test]
async fn tools_list_returns_all_three_loom_tools() {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list"
    });

    let resp = post_rpc(app_no_db(), body).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    let tools = json["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 3);

    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"loom_learn"));
    assert!(names.contains(&"loom_think"));
    assert!(names.contains(&"loom_recall"));

    // Every tool must have an inputSchema with type=object.
    for tool in tools {
        assert_eq!(tool["inputSchema"]["type"], "object");
        assert!(tool["inputSchema"]["properties"].is_object());
    }
}

// ===========================================================================
// ping
// ===========================================================================

#[tokio::test]
async fn ping_returns_empty_result() {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "ping"
    });

    let resp = post_rpc(app_no_db(), body).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert_eq!(json["id"], 3);
    assert_eq!(json["result"], json!({}));
}

// ===========================================================================
// notifications
// ===========================================================================

#[tokio::test]
async fn notifications_initialized_returns_204_no_body() {
    let body = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });

    let resp = post_rpc(app_no_db(), body).await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn request_without_id_is_treated_as_notification() {
    // A `tools/list` request with no id is a notification — per JSON-RPC 2.0
    // the server must not respond. Clients that send notifications are
    // typically broken, but we handle it gracefully with 204.
    let body = json!({
        "jsonrpc": "2.0",
        "method": "tools/list"
    });

    let resp = post_rpc(app_no_db(), body).await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

// ===========================================================================
// error paths
// ===========================================================================

#[tokio::test]
async fn unknown_method_returns_method_not_found() {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 99,
        "method": "does/not/exist"
    });

    let resp = post_rpc(app_no_db(), body).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert_eq!(json["id"], 99);
    assert!(json["result"].is_null(), "unknown method must not return result");
    assert_eq!(json["error"]["code"], -32601);
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("does/not/exist"));
}

#[tokio::test]
async fn wrong_jsonrpc_version_returns_invalid_request() {
    let body = json!({
        "jsonrpc": "1.0",
        "id": 5,
        "method": "tools/list"
    });

    let resp = post_rpc(app_no_db(), body).await;
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], -32600);
}

#[tokio::test]
async fn tools_call_without_params_returns_invalid_params() {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 6,
        "method": "tools/call"
    });

    let resp = post_rpc(app_no_db(), body).await;
    let json = body_json(resp).await;
    assert_eq!(json["error"]["code"], -32602);
}

#[tokio::test]
async fn tools_call_unknown_tool_returns_tool_error_not_jsonrpc_error() {
    // Unknown tool name is a tool-level error (is_error: true in result),
    // NOT a JSON-RPC method-not-found error — JSON-RPC found the method
    // `tools/call` correctly, the server just can't execute the requested
    // tool. This is the distinction MCP clients depend on to tell transport
    // failures from tool failures.
    let body = json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "tools/call",
        "params": {
            "name": "nonexistent_tool",
            "arguments": {}
        }
    });

    let resp = post_rpc(app_no_db(), body).await;
    let json = body_json(resp).await;

    // JSON-RPC layer: success.
    assert!(json["error"].is_null());
    assert!(json["result"].is_object());

    // Tool result layer: isError true, content block carrying the message.
    let result = &json["result"];
    assert_eq!(result["isError"], true);
    let content = result["content"].as_array().unwrap();
    assert!(!content.is_empty());
    assert!(content[0]["text"]
        .as_str()
        .unwrap()
        .contains("nonexistent_tool"));
}

#[tokio::test]
async fn tools_call_with_malformed_arguments_returns_tool_error() {
    // Arguments that deserialize into the wrong shape should surface as a
    // tool error with a helpful message naming the failure.
    let body = json!({
        "jsonrpc": "2.0",
        "id": 8,
        "method": "tools/call",
        "params": {
            "name": "loom_learn",
            "arguments": {
                // Missing required content + source + namespace; serde should
                // reject this on deserialization of LearnRequest.
                "foo": "bar"
            }
        }
    });

    let resp = post_rpc(app_no_db(), body).await;
    let json = body_json(resp).await;

    assert_eq!(json["result"]["isError"], true);
    let msg = json["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        msg.contains("invalid arguments") && msg.contains("loom_learn"),
        "expected schema failure message, got: {msg}"
    );
}

// ===========================================================================
// auth
// ===========================================================================

#[tokio::test]
async fn missing_bearer_token_returns_401() {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "initialize"
    });
    let resp = app_no_db()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mcp")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ===========================================================================
// id preservation
// ===========================================================================

#[tokio::test]
async fn string_ids_are_preserved_in_response() {
    // MCP clients frequently use string ids instead of numeric ones. The
    // server must echo them back verbatim.
    let body = json!({
        "jsonrpc": "2.0",
        "id": "req-abc-123",
        "method": "tools/list"
    });

    let resp = post_rpc(app_no_db(), body).await;
    let json = body_json(resp).await;
    assert_eq!(json["id"], "req-abc-123");
}
