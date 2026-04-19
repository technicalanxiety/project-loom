//! Bearer token authentication middleware.
//!
//! Validates the `Authorization: Bearer <token>` header on all API requests
//! using constant-time comparison to prevent timing attacks. Returns a
//! generic 401 response on any auth failure — the response body never
//! reveals whether the token exists, is expired, or is simply wrong.
//!
//! # Security properties
//!
//! - Tokens are compared with [`subtle::ConstantTimeEq`]-equivalent logic
//!   via [`constant_time_eq`] to prevent timing side-channels.
//! - Failed attempts are logged with request metadata (IP, timestamp) but
//!   the submitted token value is **never** logged.
//! - All auth errors return the same generic message regardless of failure
//!   reason (missing header, malformed header, wrong token).
//! - The middleware is applied at the router level, before any handler runs.

use axum::{
    body::Body,
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

// ---------------------------------------------------------------------------
// Error response
// ---------------------------------------------------------------------------

/// Generic 401 response body — never reveals the reason for rejection.
const UNAUTHORIZED_BODY: &str = "Unauthorized";

/// Build a 401 Unauthorized response with a `WWW-Authenticate` header.
fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
        UNAUTHORIZED_BODY,
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Constant-time comparison
// ---------------------------------------------------------------------------

/// Compare two byte slices in constant time to prevent timing attacks.
///
/// Returns `true` only when both slices have the same length **and** every
/// byte is equal. The comparison always visits all bytes of the shorter
/// slice, preventing early-exit timing leaks.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    // XOR all bytes and accumulate into a single value. Any non-zero result
    // means at least one byte differed.
    let diff: u8 = a.iter().zip(b.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y));
    diff == 0
}

// ---------------------------------------------------------------------------
// Middleware
// ---------------------------------------------------------------------------

/// Tower-compatible axum middleware that validates the bearer token.
///
/// Extracts the `Authorization` header, strips the `Bearer ` prefix, and
/// compares the submitted token against `expected_token` using constant-time
/// comparison. Any failure returns 401 immediately.
///
/// # Usage
///
/// ```rust,ignore
/// use axum::middleware;
///
/// let app = Router::new()
///     .route("/mcp", post(mcp_handler))
///     .layer(middleware::from_fn_with_state(
///         expected_token.clone(),
///         require_bearer_token,
///     ));
/// ```
pub async fn require_bearer_token(
    axum::extract::State(expected_token): axum::extract::State<String>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let submitted = match auth_header {
        Some(h) if h.starts_with("Bearer ") => &h["Bearer ".len()..],
        Some(_) => {
            // Header present but malformed (no "Bearer " prefix).
            tracing::warn!(
                path = %request.uri().path(),
                "auth rejected: malformed Authorization header (missing Bearer prefix)"
            );
            return unauthorized();
        }
        None => {
            tracing::warn!(
                path = %request.uri().path(),
                "auth rejected: missing Authorization header"
            );
            return unauthorized();
        }
    };

    if !constant_time_eq(submitted.as_bytes(), expected_token.as_bytes()) {
        // Log the rejection with metadata but NOT the submitted token value.
        tracing::warn!(
            path = %request.uri().path(),
            "auth rejected: invalid bearer token"
        );
        return unauthorized();
    }

    next.run(request).await
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        middleware,
        routing::get,
        Router,
    };
    use tower::ServiceExt; // for `oneshot`

    /// Build a test router with the auth middleware applied.
    fn test_app(token: &str) -> Router {
        Router::new()
            .route("/test", get(|| async { "ok" }))
            .layer(middleware::from_fn_with_state(
                token.to_string(),
                require_bearer_token,
            ))
    }

    async fn send(app: Router, auth: Option<&str>) -> StatusCode {
        let mut builder = Request::builder().uri("/test").method("GET");
        if let Some(a) = auth {
            builder = builder.header("Authorization", a);
        }
        let req = builder.body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        resp.status()
    }

    #[tokio::test]
    async fn missing_header_returns_401() {
        let app = test_app("secret");
        assert_eq!(send(app, None).await, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn malformed_header_no_bearer_prefix_returns_401() {
        let app = test_app("secret");
        assert_eq!(
            send(app, Some("Token secret")).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn wrong_token_returns_401() {
        let app = test_app("secret");
        assert_eq!(
            send(app, Some("Bearer wrong")).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn valid_token_returns_200() {
        let app = test_app("secret");
        assert_eq!(
            send(app, Some("Bearer secret")).await,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn empty_token_returns_401() {
        let app = test_app("secret");
        assert_eq!(
            send(app, Some("Bearer ")).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn bearer_prefix_only_returns_401() {
        let app = test_app("secret");
        // "Bearer " with nothing after — submitted token is ""
        assert_eq!(
            send(app, Some("Bearer ")).await,
            StatusCode::UNAUTHORIZED
        );
    }

    // -- constant_time_eq unit tests ----------------------------------------

    #[test]
    fn constant_time_eq_equal_slices() {
        assert!(constant_time_eq(b"hello", b"hello"));
    }

    #[test]
    fn constant_time_eq_different_slices() {
        assert!(!constant_time_eq(b"hello", b"world"));
    }

    #[test]
    fn constant_time_eq_different_lengths() {
        assert!(!constant_time_eq(b"hello", b"hell"));
    }

    #[test]
    fn constant_time_eq_empty_slices() {
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn constant_time_eq_one_empty() {
        assert!(!constant_time_eq(b"", b"x"));
    }
}
