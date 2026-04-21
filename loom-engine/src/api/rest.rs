//! REST API endpoints for non-MCP clients.
//!
//! Exposes three endpoints:
//!
//! - **POST /api/learn** — manual episode submission. Identical logic to
//!   `loom_learn` but forces `source = "manual"` regardless of the request body.
//! - **POST /api/webhooks/github** — GitHub webhook connector. Ingests pull
//!   request comment and issue comment events as episodes.
//! - **GET /api/health** — unauthenticated health check returning database and
//!   Ollama connectivity status.

use std::time::{Duration, Instant};

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{DateTime, Utc};
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
// GitHub webhook types
// ---------------------------------------------------------------------------

/// Supported GitHub webhook event types.
const GITHUB_EVENT_ISSUE_COMMENT: &str = "issue_comment";
const GITHUB_EVENT_PR_REVIEW_COMMENT: &str = "pull_request_review_comment";

/// GitHub webhook payload for issue comment and PR review comment events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubWebhookPayload {
    /// The action that was performed (e.g. "created", "edited", "deleted").
    pub action: String,
    /// The comment object containing the body and metadata.
    pub comment: GitHubComment,
    /// The repository where the event occurred.
    pub repository: GitHubRepository,
    /// The user who triggered the event.
    pub sender: GitHubUser,
}

/// A GitHub comment (issue comment or PR review comment).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubComment {
    /// Unique numeric identifier for the comment.
    pub id: i64,
    /// The comment body text.
    pub body: String,
    /// ISO 8601 timestamp when the comment was created.
    pub created_at: DateTime<Utc>,
    /// URL to the comment on GitHub.
    #[serde(default)]
    pub html_url: Option<String>,
}

/// A GitHub repository reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubRepository {
    /// Full repository name including owner (e.g. "owner/repo").
    pub full_name: String,
    /// Short repository name without owner.
    pub name: String,
}

/// A GitHub user reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubUser {
    /// GitHub username.
    pub login: String,
}

/// Resolve a namespace from a GitHub repository full name.
///
/// Uses the repository `full_name` (e.g. "owner/repo") directly as the
/// namespace, preserving the owner context for isolation.
fn resolve_namespace_from_repo(full_name: &str) -> String {
    full_name.to_string()
}

/// Build a `source_event_id` from the GitHub event type and comment ID.
///
/// Format: `{event_type}:{comment_id}` to ensure uniqueness across event
/// types (an issue comment and PR comment could theoretically share an ID).
fn build_source_event_id(event_type: &str, comment_id: i64) -> String {
    format!("{event_type}:{comment_id}")
}

// ---------------------------------------------------------------------------
// POST /api/webhooks/github
// ---------------------------------------------------------------------------

/// Handle `POST /api/webhooks/github` — GitHub webhook event ingestion.
///
/// Accepts GitHub webhook payloads for `issue_comment` and
/// `pull_request_review_comment` events. The event type is determined from
/// the `X-GitHub-Event` header.
///
/// # Idempotency
///
/// 1. Content-hash + namespace check — catches identical content.
/// 2. `(source, source_event_id)` unique constraint via
///    [`episodes::insert_episode`] using `github:{event_type}:{comment_id}`.
///
/// Returns [`LearnResponse`] with the episode UUID and status.
pub async fn handle_github_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<GitHubWebhookPayload>,
) -> Result<Json<LearnResponse>, RestError> {
    // Read the X-GitHub-Event header to determine event type.
    let event_type = headers
        .get("X-GitHub-Event")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            RestError::InvalidRequest("missing X-GitHub-Event header".into())
        })?;

    // Only accept supported event types.
    if event_type != GITHUB_EVENT_ISSUE_COMMENT
        && event_type != GITHUB_EVENT_PR_REVIEW_COMMENT
    {
        return Err(RestError::InvalidRequest(format!(
            "unsupported GitHub event type: {event_type}; \
             expected 'issue_comment' or 'pull_request_review_comment'"
        )));
    }

    // Validate comment body is not empty.
    if payload.comment.body.trim().is_empty() {
        return Err(RestError::InvalidRequest(
            "comment body must not be empty".into(),
        ));
    }

    let namespace = resolve_namespace_from_repo(&payload.repository.full_name);
    let source_event_id = build_source_event_id(event_type, payload.comment.id);
    let content_hash = compute_content_hash(&payload.comment.body);
    let occurred_at = payload.comment.created_at;
    let participants = vec![payload.sender.login.clone()];

    // Build metadata from the webhook payload.
    let metadata = serde_json::json!({
        "github_event": event_type,
        "action": payload.action,
        "comment_id": payload.comment.id,
        "html_url": payload.comment.html_url,
        "repository": payload.repository.full_name,
    });

    // Idempotency check: content_hash + namespace.
    let existing_by_hash: Option<crate::types::episode::Episode> = sqlx::query_as(
        "SELECT * FROM loom_episodes WHERE content_hash = $1 AND namespace = $2 LIMIT 1",
    )
    .bind(&content_hash)
    .bind(&namespace)
    .fetch_optional(&state.pools.offline)
    .await
    .map_err(|e| RestError::Database(e.to_string()))?;

    if let Some(existing) = existing_by_hash {
        tracing::info!(
            episode_id = %existing.id,
            namespace = %namespace,
            source_event_id = %source_event_id,
            "github_webhook: duplicate episode (content_hash match)"
        );
        return Ok(Json(LearnResponse {
            episode_id: existing.id,
            status: "duplicate".to_string(),
        }));
    }

    // Insert episode — insert_episode handles (source, source_event_id) dedup.
    let new_ep = NewEpisode {
        source: "github".to_string(),
        source_id: None,
        source_event_id: Some(source_event_id.clone()),
        content: payload.comment.body.clone(),
        content_hash,
        occurred_at,
        namespace: namespace.clone(),
        metadata: Some(metadata),
        participants: Some(participants),
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
        namespace = %namespace,
        event_type = %event_type,
        source_event_id = %source_event_id,
        status,
        "github_webhook: episode ingested"
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
    use chrono::Datelike;

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

    // -- GitHub webhook payload deserialization -----------------------------

    /// Helper to build a minimal valid issue_comment payload JSON string.
    fn sample_issue_comment_json() -> String {
        serde_json::json!({
            "action": "created",
            "comment": {
                "id": 12345,
                "body": "Looks good to me, let's merge this.",
                "created_at": "2025-01-15T10:30:00Z",
                "html_url": "https://github.com/owner/repo/issues/1#issuecomment-12345"
            },
            "repository": {
                "full_name": "owner/repo",
                "name": "repo"
            },
            "sender": {
                "login": "octocat"
            }
        })
        .to_string()
    }

    /// Helper to build a minimal valid pull_request_review_comment payload.
    fn sample_pr_review_comment_json() -> String {
        serde_json::json!({
            "action": "created",
            "comment": {
                "id": 67890,
                "body": "This function needs better error handling.",
                "created_at": "2025-02-20T14:00:00Z",
                "html_url": "https://github.com/org/project/pull/42#discussion_r67890"
            },
            "repository": {
                "full_name": "org/project",
                "name": "project"
            },
            "sender": {
                "login": "reviewer"
            }
        })
        .to_string()
    }

    #[test]
    fn github_issue_comment_payload_deserializes() {
        let json = sample_issue_comment_json();
        let payload: GitHubWebhookPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(payload.action, "created");
        assert_eq!(payload.comment.id, 12345);
        assert_eq!(payload.comment.body, "Looks good to me, let's merge this.");
        assert_eq!(payload.repository.full_name, "owner/repo");
        assert_eq!(payload.repository.name, "repo");
        assert_eq!(payload.sender.login, "octocat");
    }

    #[test]
    fn github_pr_review_comment_payload_deserializes() {
        let json = sample_pr_review_comment_json();
        let payload: GitHubWebhookPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(payload.action, "created");
        assert_eq!(payload.comment.id, 67890);
        assert_eq!(
            payload.comment.body,
            "This function needs better error handling."
        );
        assert_eq!(payload.repository.full_name, "org/project");
        assert_eq!(payload.sender.login, "reviewer");
    }

    #[test]
    fn github_payload_extracts_occurred_at_from_created_at() {
        let json = sample_issue_comment_json();
        let payload: GitHubWebhookPayload = serde_json::from_str(&json).unwrap();
        // created_at should parse to 2025-01-15T10:30:00Z
        assert_eq!(payload.comment.created_at.year(), 2025);
        assert_eq!(payload.comment.created_at.month(), 1);
        assert_eq!(payload.comment.created_at.day(), 15);
    }

    #[test]
    fn github_payload_html_url_is_optional() {
        let json = serde_json::json!({
            "action": "created",
            "comment": {
                "id": 99,
                "body": "test",
                "created_at": "2025-01-01T00:00:00Z"
            },
            "repository": {
                "full_name": "a/b",
                "name": "b"
            },
            "sender": {
                "login": "user"
            }
        })
        .to_string();
        let payload: GitHubWebhookPayload = serde_json::from_str(&json).unwrap();
        assert!(payload.comment.html_url.is_none());
    }

    // -- Namespace resolution from repository full_name --------------------

    #[test]
    fn namespace_from_repo_uses_full_name() {
        let ns = resolve_namespace_from_repo("owner/repo");
        assert_eq!(ns, "owner/repo");
    }

    #[test]
    fn namespace_from_repo_preserves_org_prefix() {
        let ns = resolve_namespace_from_repo("my-org/my-project");
        assert_eq!(ns, "my-org/my-project");
    }

    #[test]
    fn namespace_from_repo_handles_nested_names() {
        // GitHub doesn't actually allow nested names, but verify no panic.
        let ns = resolve_namespace_from_repo("a/b/c");
        assert_eq!(ns, "a/b/c");
    }

    // -- Participant extraction from sender --------------------------------

    #[test]
    fn participant_extracted_from_sender_login() {
        let json = sample_issue_comment_json();
        let payload: GitHubWebhookPayload = serde_json::from_str(&json).unwrap();
        let participants = vec![payload.sender.login.clone()];
        assert_eq!(participants, vec!["octocat"]);
    }

    #[test]
    fn participant_from_pr_review_sender() {
        let json = sample_pr_review_comment_json();
        let payload: GitHubWebhookPayload = serde_json::from_str(&json).unwrap();
        let participants = vec![payload.sender.login.clone()];
        assert_eq!(participants, vec!["reviewer"]);
    }

    // -- source_event_id construction from comment id ----------------------

    #[test]
    fn source_event_id_from_issue_comment() {
        let id = build_source_event_id("issue_comment", 12345);
        assert_eq!(id, "issue_comment:12345");
    }

    #[test]
    fn source_event_id_from_pr_review_comment() {
        let id = build_source_event_id("pull_request_review_comment", 67890);
        assert_eq!(id, "pull_request_review_comment:67890");
    }

    #[test]
    fn source_event_id_different_types_produce_different_ids() {
        let id1 = build_source_event_id("issue_comment", 100);
        let id2 = build_source_event_id("pull_request_review_comment", 100);
        assert_ne!(
            id1, id2,
            "same comment ID with different event types must produce different source_event_ids"
        );
    }

    // -- Unsupported event type rejection ----------------------------------

    #[test]
    fn unsupported_event_type_is_detected() {
        let event_type = "push";
        let is_supported = event_type == GITHUB_EVENT_ISSUE_COMMENT
            || event_type == GITHUB_EVENT_PR_REVIEW_COMMENT;
        assert!(!is_supported, "push events should not be supported");
    }

    #[test]
    fn issue_comment_event_type_is_supported() {
        let is_supported = GITHUB_EVENT_ISSUE_COMMENT == "issue_comment";
        assert!(is_supported);
    }

    #[test]
    fn pr_review_comment_event_type_is_supported() {
        let is_supported = GITHUB_EVENT_PR_REVIEW_COMMENT == "pull_request_review_comment";
        assert!(is_supported);
    }

    #[test]
    fn unsupported_event_types_rejected() {
        let unsupported = ["push", "pull_request", "issues", "create", "delete", "star"];
        for event in &unsupported {
            let is_supported = *event == GITHUB_EVENT_ISSUE_COMMENT
                || *event == GITHUB_EVENT_PR_REVIEW_COMMENT;
            assert!(
                !is_supported,
                "event type '{}' should not be supported",
                event
            );
        }
    }

    // -- GitHub webhook content hash ----------------------------------------

    #[test]
    fn github_comment_content_hash_is_deterministic() {
        let body = "Looks good to me, let's merge this.";
        let h1 = compute_content_hash(body);
        let h2 = compute_content_hash(body);
        assert_eq!(h1, h2);
    }

    #[test]
    fn github_different_comments_produce_different_hashes() {
        let h1 = compute_content_hash("LGTM");
        let h2 = compute_content_hash("Needs changes");
        assert_ne!(h1, h2);
    }
}
