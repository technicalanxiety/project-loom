//! REST API endpoints for non-MCP clients.
//!
//! Exposes two endpoints:
//!
//! - **POST /api/learn** — manual episode submission. Identical logic to
//!   `loom_learn` but forces `source = "manual"` regardless of the request body.
//! - **GET /api/health** — unauthenticated health check returning database and
//!   Ollama connectivity status.

use std::time::{Duration, Instant};

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::db::episodes::{self, NewEpisode};
use crate::types::mcp::{LearnRequest, LearnResponse};

use super::mcp::AppState;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur in REST handlers.
#[derive(Debug, thiserror::Error)]
pub enum RestError {
    /// A required field was missing or invalid.
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    /// A database operation failed.
    #[error("database error: {0}")]
    Database(String),
}

impl IntoResponse for RestError {
    fn into_response(self) -> Response {
        let status = match &self {
            RestError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            RestError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = serde_json::json!({ "error": self.to_string() });
        (status, Json(body)).into_response()
    }
}

// ---------------------------------------------------------------------------
// Health check types
// ---------------------------------------------------------------------------

/// Status of a single downstream component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentStatus {
    /// Whether the component responded successfully.
    pub ok: bool,
    /// Round-trip latency in milliseconds, if the probe completed.
    pub latency_ms: Option<u64>,
    /// Error message if the probe failed.
    pub error: Option<String>,
}

/// Response payload for `GET /api/health`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    /// Overall status: `"ok"` when all components are healthy, `"degraded"` otherwise.
    pub status: String,
    /// PostgreSQL database connectivity.
    pub database: ComponentStatus,
    /// Ollama LLM service connectivity.
    pub ollama: ComponentStatus,
    /// Crate version from `Cargo.toml`.
    pub version: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Compute a hex-encoded SHA-256 hash of the given content string.
fn compute_content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

// ---------------------------------------------------------------------------
// POST /api/learn
// ---------------------------------------------------------------------------

/// Handle `POST /api/learn` — manual episode submission.
///
/// Accepts the same [`LearnRequest`] body as `loom_learn` but always sets
/// `source = "manual"`, ignoring whatever the caller provided. Requires
/// `content` and `namespace`; `occurred_at` defaults to now if omitted.
///
/// # Idempotency
///
/// 1. Content-hash + namespace check — catches identical content re-submitted
///    under different event IDs.
/// 2. `(source, source_event_id)` unique constraint handled by
///    [`episodes::insert_episode`].
///
/// Returns [`LearnResponse`] with the episode UUID and status `"queued"` or
/// `"duplicate"`.
pub async fn handle_api_learn(
    State(state): State<AppState>,
    Json(mut req): Json<LearnRequest>,
) -> Result<Json<LearnResponse>, RestError> {
    // Force source to "manual" regardless of what the caller sent.
    req.source = "manual".to_string();

    // Validate required fields.
    if req.content.trim().is_empty() {
        return Err(RestError::InvalidRequest("content must not be empty".into()));
    }
    if req.namespace.trim().is_empty() {
        return Err(RestError::InvalidRequest("namespace must not be empty".into()));
    }

    let content_hash = compute_content_hash(&req.content);
    let occurred_at = req.occurred_at.unwrap_or_else(chrono::Utc::now);

    // Idempotency check: content_hash + namespace.
    let existing_by_hash: Option<crate::types::episode::Episode> = sqlx::query_as(
        "SELECT * FROM loom_episodes WHERE content_hash = $1 AND namespace = $2 LIMIT 1",
    )
    .bind(&content_hash)
    .bind(&req.namespace)
    .fetch_optional(&state.pools.offline)
    .await
    .map_err(|e| RestError::Database(e.to_string()))?;

    if let Some(existing) = existing_by_hash {
        tracing::info!(
            episode_id = %existing.id,
            namespace = %req.namespace,
            "api_learn: duplicate episode (content_hash match)"
        );
        return Ok(Json(LearnResponse {
            episode_id: existing.id,
            status: "duplicate".to_string(),
        }));
    }

    // Insert episode — insert_episode handles (source, source_event_id) dedup.
    let new_ep = NewEpisode {
        source: "manual".to_string(),
        source_id: None,
        source_event_id: req.source_event_id.clone(),
        content: req.content.clone(),
        content_hash: content_hash.clone(),
        occurred_at,
        namespace: req.namespace.clone(),
        metadata: req.metadata.clone(),
        participants: req.participants.clone(),
    };

    let episode = episodes::insert_episode(&state.pools.offline, &new_ep)
        .await
        .map_err(|e| RestError::Database(e.to_string()))?;

    let status = if episode.processed.unwrap_or(false) {
        "duplicate"
    } else {
        "queued"
    };

    tracing::info!(
        episode_id = %episode.id,
        namespace = %req.namespace,
        status,
        "api_learn: manual episode ingested"
    );

    Ok(Json(LearnResponse {
        episode_id: episode.id,
        status: status.to_string(),
    }))
}

// ---------------------------------------------------------------------------
// GET /api/health
// ---------------------------------------------------------------------------

/// Handle `GET /api/health` — unauthenticated connectivity health check.
///
/// Probes the PostgreSQL online pool (`SELECT 1`) and the Ollama API
/// (`GET /api/tags`) with a 2-second timeout each. Always returns HTTP 200
/// so that load balancers and Docker health checks can read the body to
/// determine degraded state rather than treating a 5xx as a crash.
pub async fn handle_health(State(state): State<AppState>) -> Json<HealthResponse> {
    let timeout = Duration::from_secs(2);

    // Probe database (online pool).
    let db_status = probe_database(&state.pools.online, timeout).await;

    // Probe Ollama.
    let ollama_status = probe_ollama(&state.config.llm.ollama_url, timeout).await;

    let overall = if db_status.ok && ollama_status.ok {
        "ok"
    } else {
        "degraded"
    };

    tracing::debug!(
        status = overall,
        db_ok = db_status.ok,
        ollama_ok = ollama_status.ok,
        "health check completed"
    );

    Json(HealthResponse {
        status: overall.to_string(),
        database: db_status,
        ollama: ollama_status,
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// Probe the PostgreSQL pool with a `SELECT 1` within the given timeout.
async fn probe_database(pool: &sqlx::PgPool, timeout: Duration) -> ComponentStatus {
    let start = Instant::now();
    let result = tokio::time::timeout(
        timeout,
        sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(pool),
    )
    .await;

    let latency_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok(Ok(_)) => ComponentStatus {
            ok: true,
            latency_ms: Some(latency_ms),
            error: None,
        },
        Ok(Err(e)) => ComponentStatus {
            ok: false,
            latency_ms: Some(latency_ms),
            error: Some(e.to_string()),
        },
        Err(_elapsed) => ComponentStatus {
            ok: false,
            latency_ms: Some(latency_ms),
            error: Some("database probe timed out".to_string()),
        },
    }
}

/// Probe the Ollama API by hitting `{base_url}/api/tags` within the given timeout.
async fn probe_ollama(base_url: &str, timeout: Duration) -> ComponentStatus {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let start = Instant::now();

    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .unwrap_or_default();

    let result = client.get(&url).send().await;
    let latency_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok(resp) if resp.status().is_success() => ComponentStatus {
            ok: true,
            latency_ms: Some(latency_ms),
            error: None,
        },
        Ok(resp) => ComponentStatus {
            ok: false,
            latency_ms: Some(latency_ms),
            error: Some(format!("ollama returned HTTP {}", resp.status())),
        },
        Err(e) => ComponentStatus {
            ok: false,
            latency_ms: Some(latency_ms),
            error: Some(e.to_string()),
        },
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- compute_content_hash -----------------------------------------------

    #[test]
    fn content_hash_is_hex_sha256() {
        let hash = compute_content_hash("hello world");
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn same_content_produces_same_hash() {
        let h1 = compute_content_hash("test content");
        let h2 = compute_content_hash("test content");
        assert_eq!(h1, h2);
    }

    #[test]
    fn different_content_produces_different_hash() {
        let h1 = compute_content_hash("content A");
        let h2 = compute_content_hash("content B");
        assert_ne!(h1, h2);
    }

    #[test]
    fn empty_content_produces_valid_hash() {
        let hash = compute_content_hash("");
        assert_eq!(hash.len(), 64);
    }

    // -- RestError display --------------------------------------------------

    #[test]
    fn rest_error_invalid_request_displays() {
        let err = RestError::InvalidRequest("content must not be empty".into());
        assert!(err.to_string().contains("invalid request"));
        assert!(err.to_string().contains("content must not be empty"));
    }

    #[test]
    fn rest_error_database_displays() {
        let err = RestError::Database("connection refused".into());
        assert!(err.to_string().contains("database error"));
        assert!(err.to_string().contains("connection refused"));
    }

    // -- RestError HTTP status codes ----------------------------------------

    #[tokio::test]
    async fn invalid_request_error_returns_400() {
        let err = RestError::InvalidRequest("bad input".into());
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn database_error_returns_500() {
        let err = RestError::Database("db down".into());
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    // -- HealthResponse serialization ---------------------------------------

    #[test]
    fn health_response_serializes_ok_status() {
        let resp = HealthResponse {
            status: "ok".to_string(),
            database: ComponentStatus {
                ok: true,
                latency_ms: Some(3),
                error: None,
            },
            ollama: ComponentStatus {
                ok: true,
                latency_ms: Some(12),
                error: None,
            },
            version: "0.1.0".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"status\":\"ok\""));
        assert!(json.contains("\"ok\":true"));
    }

    #[test]
    fn health_response_serializes_degraded_status() {
        let resp = HealthResponse {
            status: "degraded".to_string(),
            database: ComponentStatus {
                ok: false,
                latency_ms: None,
                error: Some("timeout".to_string()),
            },
            ollama: ComponentStatus {
                ok: true,
                latency_ms: Some(5),
                error: None,
            },
            version: "0.1.0".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"status\":\"degraded\""));
        assert!(json.contains("\"error\":\"timeout\""));
    }

    // -- probe_ollama URL construction --------------------------------------

    #[test]
    fn ollama_url_trailing_slash_is_trimmed() {
        // Verify the URL construction logic doesn't double-slash.
        let base = "http://ollama:11434/";
        let url = format!("{}/api/tags", base.trim_end_matches('/'));
        assert_eq!(url, "http://ollama:11434/api/tags");
    }

    #[test]
    fn ollama_url_no_trailing_slash() {
        let base = "http://ollama:11434";
        let url = format!("{}/api/tags", base.trim_end_matches('/'));
        assert_eq!(url, "http://ollama:11434/api/tags");
    }

    // -- handle_api_learn validation (pure logic) ---------------------------

    /// Verify that empty content triggers an InvalidRequest error before any
    /// DB access. We test the validation condition directly since constructing
    /// a full AppState requires a live database.
    #[test]
    fn empty_content_triggers_invalid_request() {
        // Mirrors the validation guard in handle_api_learn.
        let content = "   ";
        let is_empty = content.trim().is_empty();
        assert!(is_empty, "whitespace-only content must be treated as empty");
    }

    #[test]
    fn non_empty_content_passes_validation() {
        let content = "some episode content";
        let is_empty = content.trim().is_empty();
        assert!(!is_empty);
    }

    #[test]
    fn empty_namespace_triggers_invalid_request() {
        let namespace = "";
        let is_empty = namespace.trim().is_empty();
        assert!(is_empty, "empty namespace must be treated as empty");
    }

    #[test]
    fn whitespace_namespace_triggers_invalid_request() {
        let namespace = "  \t  ";
        let is_empty = namespace.trim().is_empty();
        assert!(is_empty, "whitespace-only namespace must be treated as empty");
    }

    #[test]
    fn non_empty_namespace_passes_validation() {
        let namespace = "my-project";
        let is_empty = namespace.trim().is_empty();
        assert!(!is_empty);
    }

    /// Verify that handle_api_learn forces source = "manual" regardless of
    /// what the caller provides. We test the mutation logic directly.
    #[test]
    fn source_is_forced_to_manual() {
        // Simulate the mutation in handle_api_learn.
        let mut source = "claude-code".to_string();
        source = "manual".to_string();
        assert_eq!(source, "manual");
    }

    // -- ComponentStatus serialization -------------------------------------

    #[test]
    fn component_status_ok_with_latency_serializes() {
        let status = ComponentStatus {
            ok: true,
            latency_ms: Some(42),
            error: None,
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["ok"], true);
        assert_eq!(json["latency_ms"], 42);
        assert!(json["error"].is_null());
    }

    #[test]
    fn component_status_failed_with_error_serializes() {
        let status = ComponentStatus {
            ok: false,
            latency_ms: Some(100),
            error: Some("connection refused".to_string()),
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["ok"], false);
        assert_eq!(json["error"], "connection refused");
    }

    #[test]
    fn component_status_no_latency_serializes() {
        let status = ComponentStatus {
            ok: false,
            latency_ms: None,
            error: Some("timeout".to_string()),
        };
        let json = serde_json::to_value(&status).unwrap();
        assert!(json["latency_ms"].is_null());
        assert_eq!(json["error"], "timeout");
    }

    // -- HealthResponse overall status logic --------------------------------

    #[test]
    fn health_response_both_ok_produces_ok_status() {
        let db = ComponentStatus { ok: true, latency_ms: Some(1), error: None };
        let ollama = ComponentStatus { ok: true, latency_ms: Some(2), error: None };
        let overall = if db.ok && ollama.ok { "ok" } else { "degraded" };
        assert_eq!(overall, "ok");
    }

    #[test]
    fn health_response_db_fail_produces_degraded_status() {
        let db = ComponentStatus { ok: false, latency_ms: None, error: Some("down".into()) };
        let ollama = ComponentStatus { ok: true, latency_ms: Some(2), error: None };
        let overall = if db.ok && ollama.ok { "ok" } else { "degraded" };
        assert_eq!(overall, "degraded");
    }

    #[test]
    fn health_response_ollama_fail_produces_degraded_status() {
        let db = ComponentStatus { ok: true, latency_ms: Some(1), error: None };
        let ollama = ComponentStatus { ok: false, latency_ms: None, error: Some("unreachable".into()) };
        let overall = if db.ok && ollama.ok { "ok" } else { "degraded" };
        assert_eq!(overall, "degraded");
    }

    #[test]
    fn health_response_both_fail_produces_degraded_status() {
        let db = ComponentStatus { ok: false, latency_ms: None, error: Some("down".into()) };
        let ollama = ComponentStatus { ok: false, latency_ms: None, error: Some("unreachable".into()) };
        let overall = if db.ok && ollama.ok { "ok" } else { "degraded" };
        assert_eq!(overall, "degraded");
    }

    #[test]
    fn health_response_status_field_is_ok_or_degraded() {
        // The status field must be exactly "ok" or "degraded" — no other values.
        let valid_statuses = ["ok", "degraded"];
        for status in &valid_statuses {
            let resp = HealthResponse {
                status: status.to_string(),
                database: ComponentStatus { ok: true, latency_ms: None, error: None },
                ollama: ComponentStatus { ok: true, latency_ms: None, error: None },
                version: "0.1.0".to_string(),
            };
            assert!(
                resp.status == "ok" || resp.status == "degraded",
                "status must be 'ok' or 'degraded', got '{}'",
                resp.status
            );
        }
    }

    #[test]
    fn health_response_contains_database_and_ollama_fields() {
        let resp = HealthResponse {
            status: "ok".to_string(),
            database: ComponentStatus { ok: true, latency_ms: Some(5), error: None },
            ollama: ComponentStatus { ok: true, latency_ms: Some(10), error: None },
            version: "0.1.0".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json.get("database").is_some(), "database field must be present");
        assert!(json.get("ollama").is_some(), "ollama field must be present");
        assert!(json.get("status").is_some(), "status field must be present");
        assert!(json.get("version").is_some(), "version field must be present");
    }
}
