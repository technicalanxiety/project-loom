//! Ollama HTTP client with Azure OpenAI fallback.
//!
//! Provides a unified [`LlmClient`] that talks to Ollama's OpenAI-compatible
//! API (`/v1/chat/completions`, `/v1/embeddings`) and falls back to Azure
//! OpenAI when Ollama is unreachable (connection error or timeout).
//!
//! All calls include exponential-backoff retry (3 attempts) and structured
//! logging via [`tracing`].

use std::time::Duration;

use reqwest::StatusCode;
use serde_json::json;
use thiserror::Error;

use crate::config::LlmConfig;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors produced by the LLM client layer.
#[derive(Debug, Error)]
pub enum LlmError {
    /// An HTTP-level error from reqwest (connection refused, DNS, TLS, etc.).
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    /// The upstream API returned a non-2xx status code.
    #[error("API returned status {status}: {body}")]
    ApiError {
        /// HTTP status code.
        status: u16,
        /// Response body (truncated to 512 chars).
        body: String,
    },

    /// Failed to parse the upstream JSON response.
    #[error("failed to parse response JSON: {0}")]
    Parse(String),

    /// All retry attempts (including fallback) were exhausted.
    #[error("all retries exhausted: {0}")]
    RetriesExhausted(String),

    /// Azure OpenAI fallback is not configured.
    #[error("Azure OpenAI fallback not configured")]
    NoFallback,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// Maximum number of retry attempts per provider (Ollama or Azure).
const MAX_RETRIES: u32 = 3;

/// Maximum number of retry attempts for Azure OpenAI rate limits.
/// Uses escalating backoff: 1s, 2s, 4s, 8s, 16s.
const AZURE_RATE_LIMIT_MAX_RETRIES: u32 = 5;

/// Per-request timeout.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Base delay for exponential backoff (doubles each retry).
const BACKOFF_BASE: Duration = Duration::from_millis(500);

/// Base delay for Azure rate limit backoff (1 second, doubles each retry).
const AZURE_RATE_LIMIT_BACKOFF_BASE: Duration = Duration::from_secs(1);

/// Unified LLM client wrapping reqwest with Ollama primary and Azure OpenAI
/// fallback.
#[derive(Debug, Clone)]
pub struct LlmClient {
    /// Underlying HTTP client with timeout configuration.
    http: reqwest::Client,
    /// Ollama base URL (e.g. `http://ollama:11434`).
    ollama_url: String,
    /// Optional Azure OpenAI endpoint URL.
    azure_url: Option<String>,
    /// Optional Azure OpenAI API key.
    azure_key: Option<String>,
}

impl LlmClient {
    /// Create a new [`LlmClient`] from the application's [`LlmConfig`].
    ///
    /// The underlying reqwest client is configured with a 30-second timeout.
    pub fn new(config: &LlmConfig) -> Result<Self, LlmError> {
        // Ensure the rustls crypto provider is installed before reqwest
        // attempts any TLS handshake. reqwest is compiled with
        // `rustls-no-provider` so that aws-lc-sys is not pulled in; callers
        // must install a provider themselves. Idempotent — safe to call from
        // tests that construct multiple LlmClients and from main() which
        // also calls it. `crate::crypto` resolves both when compiled as
        // part of the library and as part of the binary (both crate roots
        // declare `mod crypto`).
        crate::crypto::ensure_crypto_provider();

        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()?;

        Ok(Self {
            http,
            ollama_url: config.ollama_url.trim_end_matches('/').to_string(),
            azure_url: config
                .azure_openai_url
                .as_deref()
                .map(|u| u.trim_end_matches('/').to_string()),
            azure_key: config.azure_openai_key.clone(),
        })
    }

    // -- public API ---------------------------------------------------------

    /// Send a chat completion request and return the assistant's response as
    /// a [`serde_json::Value`].
    ///
    /// Tries Ollama first with retries. On connection error or timeout the
    /// client falls back to Azure OpenAI (if configured), using
    /// `gpt-4.1-mini` as the model.
    pub async fn call_llm(
        &self,
        model: &str,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<serde_json::Value, LlmError> {
        let body = json!({
            "model": model,
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user",   "content": user_prompt },
            ],
            "temperature": 0.1,
        });

        // Attempt Ollama
        let ollama_endpoint = format!("{}/v1/chat/completions", self.ollama_url);
        match self
            .post_with_retry(&ollama_endpoint, &body, None)
            .await
        {
            Ok(resp) => parse_chat_response(&resp),
            Err(e) if is_connection_error(&e) => {
                tracing::warn!(
                    error = %e,
                    "Ollama unreachable, attempting Azure OpenAI fallback"
                );
                self.call_llm_azure(system_prompt, user_prompt).await
            }
            Err(e) => Err(e),
        }
    }

    /// Generate an embedding vector for the given `input` text.
    ///
    /// Tries Ollama first with retries. Falls back to Azure OpenAI on
    /// connection errors.
    pub async fn call_embeddings(
        &self,
        model: &str,
        input: &str,
    ) -> Result<Vec<f32>, LlmError> {
        let body = json!({
            "model": model,
            "input": input,
        });

        let ollama_endpoint = format!("{}/v1/embeddings", self.ollama_url);
        match self
            .post_with_retry(&ollama_endpoint, &body, None)
            .await
        {
            Ok(resp) => parse_embedding_response(&resp),
            Err(e) if is_connection_error(&e) => {
                tracing::warn!(
                    error = %e,
                    "Ollama unreachable for embeddings, attempting Azure OpenAI fallback"
                );
                self.call_embeddings_azure(input).await
            }
            Err(e) => Err(e),
        }
    }

    // -- Azure fallback -----------------------------------------------------

    /// Chat completion via Azure OpenAI (fallback path).
    ///
    /// Uses extended retry logic for rate limits (429): up to 5 retries
    /// with exponential backoff (1s, 2s, 4s, 8s, 16s).
    async fn call_llm_azure(
        &self,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<serde_json::Value, LlmError> {
        let (url, key) = self.azure_config()?;

        let body = json!({
            "model": "gpt-4.1-mini",
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user",   "content": user_prompt },
            ],
            "temperature": 0.1,
        });

        let endpoint = format!("{url}/v1/chat/completions");
        let resp = self
            .post_with_retry_azure(&endpoint, &body, key)
            .await?;
        parse_chat_response(&resp)
    }

    /// Embedding generation via Azure OpenAI (fallback path).
    ///
    /// Uses extended retry logic for rate limits (429).
    async fn call_embeddings_azure(
        &self,
        input: &str,
    ) -> Result<Vec<f32>, LlmError> {
        let (url, key) = self.azure_config()?;

        let body = json!({
            "model": "text-embedding-3-small",
            "input": input,
        });

        let endpoint = format!("{url}/v1/embeddings");
        let resp = self
            .post_with_retry_azure(&endpoint, &body, key)
            .await?;
        parse_embedding_response(&resp)
    }

    // -- helpers ------------------------------------------------------------

    /// Return Azure config or [`LlmError::NoFallback`].
    fn azure_config(&self) -> Result<(&str, &str), LlmError> {
        match (&self.azure_url, &self.azure_key) {
            (Some(url), Some(key)) => Ok((url.as_str(), key.as_str())),
            _ => Err(LlmError::NoFallback),
        }
    }

    /// POST `body` to `url` with exponential-backoff retry.
    ///
    /// If `api_key` is `Some`, it is sent as the `api-key` header (Azure
    /// convention).
    async fn post_with_retry(
        &self,
        url: &str,
        body: &serde_json::Value,
        api_key: Option<&str>,
    ) -> Result<serde_json::Value, LlmError> {
        let mut last_err: Option<LlmError> = None;

        for attempt in 0..MAX_RETRIES {
            if attempt > 0 {
                let delay = BACKOFF_BASE * 2u32.pow(attempt - 1);
                tracing::info!(attempt, delay_ms = delay.as_millis() as u64, url, "retrying LLM request");
                tokio::time::sleep(delay).await;
            }

            let mut req = self
                .http
                .post(url)
                .header("Content-Type", "application/json");

            if let Some(key) = api_key {
                req = req.header("api-key", key);
            }

            match req.json(body).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        let json: serde_json::Value = resp
                            .json()
                            .await
                            .map_err(|e| LlmError::Parse(e.to_string()))?;
                        tracing::info!(url, status = %status, "LLM request succeeded");
                        return Ok(json);
                    }

                    // Retry on server errors (5xx) and rate limits (429).
                    let body_text = resp
                        .text()
                        .await
                        .unwrap_or_default();
                    let body_preview = truncate(&body_text, 512);

                    if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
                        tracing::warn!(
                            url,
                            status = status.as_u16(),
                            attempt,
                            body = %body_preview,
                            "retryable API error"
                        );
                        last_err = Some(LlmError::ApiError {
                            status: status.as_u16(),
                            body: body_preview,
                        });
                        continue;
                    }

                    // Non-retryable client error — bail immediately.
                    return Err(LlmError::ApiError {
                        status: status.as_u16(),
                        body: body_preview,
                    });
                }
                Err(e) => {
                    tracing::warn!(url, attempt, error = %e, "HTTP request error");
                    last_err = Some(LlmError::Http(e));
                }
            }
        }

        Err(LlmError::RetriesExhausted(
            last_err
                .map(|e| e.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
        ))
    }

    /// POST with Azure-specific retry logic for rate limits.
    ///
    /// Uses extended backoff for 429 responses: 1s, 2s, 4s, 8s, 16s
    /// (up to [`AZURE_RATE_LIMIT_MAX_RETRIES`] attempts).
    async fn post_with_retry_azure(
        &self,
        url: &str,
        body: &serde_json::Value,
        api_key: &str,
    ) -> Result<serde_json::Value, LlmError> {
        let mut last_err: Option<LlmError> = None;

        for attempt in 0..AZURE_RATE_LIMIT_MAX_RETRIES {
            if attempt > 0 {
                // Use escalating backoff: 1s, 2s, 4s, 8s, 16s.
                let delay = AZURE_RATE_LIMIT_BACKOFF_BASE * 2u32.pow(attempt - 1);
                tracing::info!(
                    attempt,
                    delay_ms = delay.as_millis() as u64,
                    url,
                    "retrying Azure OpenAI request (rate limit backoff)"
                );
                tokio::time::sleep(delay).await;
            }

            let req = self
                .http
                .post(url)
                .header("Content-Type", "application/json")
                .header("api-key", api_key);

            match req.json(body).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        let json: serde_json::Value = resp
                            .json()
                            .await
                            .map_err(|e| LlmError::Parse(e.to_string()))?;
                        tracing::info!(url, status = %status, "Azure OpenAI request succeeded");
                        return Ok(json);
                    }

                    let body_text = resp.text().await.unwrap_or_default();
                    let body_preview = truncate(&body_text, 512);

                    // Retry on 429 (rate limit) and 5xx (server errors).
                    if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                        tracing::warn!(
                            url,
                            status = status.as_u16(),
                            attempt,
                            body = %body_preview,
                            "Azure OpenAI retryable error"
                        );
                        last_err = Some(LlmError::ApiError {
                            status: status.as_u16(),
                            body: body_preview,
                        });
                        continue;
                    }

                    return Err(LlmError::ApiError {
                        status: status.as_u16(),
                        body: body_preview,
                    });
                }
                Err(e) => {
                    tracing::warn!(url, attempt, error = %e, "Azure OpenAI HTTP request error");
                    last_err = Some(LlmError::Http(e));
                }
            }
        }

        Err(LlmError::RetriesExhausted(
            last_err
                .map(|e| e.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
        ))
    }
}

// ---------------------------------------------------------------------------
// Response parsers
// ---------------------------------------------------------------------------

/// Extract the assistant message content from an OpenAI-compatible chat
/// completion response.
///
/// Expected shape: `{"choices": [{"message": {"content": "..."}}]}`
///
/// The content string is parsed as JSON so callers receive structured data.
/// If the content is not valid JSON it is returned as a
/// [`serde_json::Value::String`].
fn parse_chat_response(resp: &serde_json::Value) -> Result<serde_json::Value, LlmError> {
    let content = resp
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .ok_or_else(|| {
            LlmError::Parse(format!(
                "unexpected chat response structure: {}",
                truncate(&resp.to_string(), 256)
            ))
        })?;

    // Try to parse the content as JSON; fall back to a plain string value.
    let value = serde_json::from_str(content)
        .unwrap_or_else(|_| serde_json::Value::String(content.to_string()));
    Ok(value)
}

/// Extract the embedding vector from an OpenAI-compatible embeddings
/// response.
///
/// Expected shape: `{"data": [{"embedding": [f32, ...]}]}`
fn parse_embedding_response(resp: &serde_json::Value) -> Result<Vec<f32>, LlmError> {
    let arr = resp
        .get("data")
        .and_then(|d| d.get(0))
        .and_then(|d| d.get("embedding"))
        .and_then(|e| e.as_array())
        .ok_or_else(|| {
            LlmError::Parse(format!(
                "unexpected embedding response structure: {}",
                truncate(&resp.to_string(), 256)
            ))
        })?;

    arr.iter()
        .map(|v| {
            v.as_f64()
                .map(|f| f as f32)
                .ok_or_else(|| LlmError::Parse("embedding element is not a number".to_string()))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/// Check whether an [`LlmError`] represents a connection-level failure that
/// warrants falling back to Azure OpenAI.
///
/// This covers connection refused, DNS failures, timeouts, and the
/// `RetriesExhausted` wrapper that captures these after multiple attempts.
fn is_connection_error(err: &LlmError) -> bool {
    match err {
        LlmError::Http(e) => is_reqwest_connection_error(e),
        LlmError::RetriesExhausted(msg) => {
            // The message is the Display of the last inner error.
            let lower = msg.to_lowercase();
            lower.contains("connect")
                || lower.contains("timeout")
                || lower.contains("timed out")
                || lower.contains("error sending request")
                || lower.contains("dns")
                || lower.contains("connection refused")
        }
        _ => false,
    }
}

/// Inspect a [`reqwest::Error`] (including its source chain) for
/// connection-level failures.
fn is_reqwest_connection_error(e: &reqwest::Error) -> bool {
    if e.is_connect() || e.is_timeout() {
        return true;
    }
    // reqwest wraps hyper errors; walk the source chain.
    let msg = e.to_string().to_lowercase();
    msg.contains("connect")
        || msg.contains("connection refused")
        || msg.contains("dns")
        || msg.contains("error sending request")
}

/// Truncate a string to at most `max` characters.
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- parse_chat_response ------------------------------------------------

    #[test]
    fn parse_chat_response_extracts_json_content() {
        let resp = json!({
            "choices": [{
                "message": {
                    "content": "{\"entities\": [{\"name\": \"Rust\"}]}"
                }
            }]
        });

        let result = parse_chat_response(&resp).expect("should parse");
        assert!(result.get("entities").is_some());
    }

    #[test]
    fn parse_chat_response_returns_string_for_non_json_content() {
        let resp = json!({
            "choices": [{
                "message": {
                    "content": "Hello, world!"
                }
            }]
        });

        let result = parse_chat_response(&resp).expect("should parse");
        assert_eq!(result, serde_json::Value::String("Hello, world!".into()));
    }

    #[test]
    fn parse_chat_response_errors_on_missing_choices() {
        let resp = json!({ "error": "bad request" });
        let err = parse_chat_response(&resp).unwrap_err();
        assert!(matches!(err, LlmError::Parse(_)));
    }

    #[test]
    fn parse_chat_response_errors_on_empty_choices() {
        let resp = json!({ "choices": [] });
        let err = parse_chat_response(&resp).unwrap_err();
        assert!(matches!(err, LlmError::Parse(_)));
    }

    // -- parse_embedding_response -------------------------------------------

    #[test]
    fn parse_embedding_response_extracts_vector() {
        let resp = json!({
            "data": [{
                "embedding": [0.1, 0.2, 0.3, 0.4]
            }]
        });

        let vec = parse_embedding_response(&resp).expect("should parse");
        assert_eq!(vec.len(), 4);
        assert!((vec[0] - 0.1).abs() < f32::EPSILON);
        assert!((vec[3] - 0.4).abs() < f32::EPSILON);
    }

    #[test]
    fn parse_embedding_response_errors_on_missing_data() {
        let resp = json!({ "error": "bad" });
        let err = parse_embedding_response(&resp).unwrap_err();
        assert!(matches!(err, LlmError::Parse(_)));
    }

    #[test]
    fn parse_embedding_response_errors_on_non_numeric_element() {
        let resp = json!({
            "data": [{
                "embedding": [0.1, "not_a_number", 0.3]
            }]
        });

        let err = parse_embedding_response(&resp).unwrap_err();
        assert!(matches!(err, LlmError::Parse(_)));
    }

    // -- is_connection_error ------------------------------------------------

    #[test]
    fn is_connection_error_detects_retries_exhausted_with_connect() {
        let err = LlmError::RetriesExhausted("connect error: refused".into());
        assert!(is_connection_error(&err));
    }

    #[test]
    fn is_connection_error_detects_retries_exhausted_with_timeout() {
        let err = LlmError::RetriesExhausted("request timed out".into());
        assert!(is_connection_error(&err));
    }

    #[test]
    fn is_connection_error_false_for_api_error() {
        let err = LlmError::ApiError {
            status: 400,
            body: "bad request".into(),
        };
        assert!(!is_connection_error(&err));
    }

    #[test]
    fn is_connection_error_false_for_parse_error() {
        let err = LlmError::Parse("bad json".into());
        assert!(!is_connection_error(&err));
    }

    // -- truncate -----------------------------------------------------------

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_string_trimmed() {
        let result = truncate("hello world", 5);
        assert!(result.starts_with("hello"));
        assert!(result.ends_with('…'));
    }

    // -- LlmError Display ---------------------------------------------------

    #[test]
    fn llm_error_display_messages() {
        let err = LlmError::NoFallback;
        assert_eq!(err.to_string(), "Azure OpenAI fallback not configured");

        let err = LlmError::ApiError {
            status: 500,
            body: "internal".into(),
        };
        assert!(err.to_string().contains("500"));

        let err = LlmError::Parse("bad".into());
        assert!(err.to_string().contains("bad"));

        let err = LlmError::RetriesExhausted("timeout".into());
        assert!(err.to_string().contains("timeout"));
    }

    // -- LlmClient construction ---------------------------------------------

    #[test]
    fn llm_client_trims_trailing_slash() {
        let config = LlmConfig {
            ollama_url: "http://localhost:11434/".to_string(),
            extraction_model: "gemma4:26b".to_string(),
            classification_model: "gemma4:e4b".to_string(),
            embedding_model: "nomic-embed-text".to_string(),
            azure_openai_url: Some("https://my-azure.openai.azure.com/".to_string()),
            azure_openai_key: Some("test-key".to_string()),
        };

        let client = LlmClient::new(&config).expect("should build");
        assert_eq!(client.ollama_url, "http://localhost:11434");
        assert_eq!(
            client.azure_url.as_deref(),
            Some("https://my-azure.openai.azure.com")
        );
    }

    #[test]
    fn azure_config_returns_no_fallback_when_missing() {
        let config = LlmConfig {
            ollama_url: "http://localhost:11434".to_string(),
            extraction_model: "gemma4:26b".to_string(),
            classification_model: "gemma4:e4b".to_string(),
            embedding_model: "nomic-embed-text".to_string(),
            azure_openai_url: None,
            azure_openai_key: None,
        };

        let client = LlmClient::new(&config).expect("should build");
        let err = client.azure_config().unwrap_err();
        assert!(matches!(err, LlmError::NoFallback));
    }

    // -- Retry logic (integration-style with wiremock) ----------------------

    #[tokio::test]
    async fn retry_succeeds_after_transient_failure() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        // First two calls return 500, third succeeds.
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(500).set_body_string("server error"),
            )
            .up_to_n_times(2)
            .expect(2)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{
                    "message": { "content": "{\"ok\": true}" }
                }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let config = LlmConfig {
            ollama_url: server.uri(),
            extraction_model: "test".to_string(),
            classification_model: "test".to_string(),
            embedding_model: "test".to_string(),
            azure_openai_url: None,
            azure_openai_key: None,
        };

        let client = LlmClient::new(&config).expect("should build");
        let result = client
            .call_llm("test-model", "system", "user")
            .await
            .expect("should succeed after retries");

        assert_eq!(result.get("ok").and_then(|v| v.as_bool()), Some(true));
    }

    #[tokio::test]
    async fn retries_exhausted_returns_error() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        // All calls return 500.
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(500).set_body_string("always failing"),
            )
            .expect(3)
            .mount(&server)
            .await;

        let config = LlmConfig {
            ollama_url: server.uri(),
            extraction_model: "test".to_string(),
            classification_model: "test".to_string(),
            embedding_model: "test".to_string(),
            azure_openai_url: None,
            azure_openai_key: None,
        };

        let client = LlmClient::new(&config).expect("should build");
        let err = client
            .call_llm("test-model", "system", "user")
            .await
            .unwrap_err();

        assert!(matches!(err, LlmError::RetriesExhausted(_)));
    }

    #[tokio::test]
    async fn non_retryable_error_fails_immediately() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        // 400 is not retryable — should fail on first attempt.
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(400).set_body_string("bad request"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let config = LlmConfig {
            ollama_url: server.uri(),
            extraction_model: "test".to_string(),
            classification_model: "test".to_string(),
            embedding_model: "test".to_string(),
            azure_openai_url: None,
            azure_openai_key: None,
        };

        let client = LlmClient::new(&config).expect("should build");
        let err = client
            .call_llm("test-model", "system", "user")
            .await
            .unwrap_err();

        assert!(matches!(err, LlmError::ApiError { status: 400, .. }));
    }

    #[tokio::test]
    async fn embedding_call_parses_response() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{
                    "embedding": [0.1, 0.2, 0.3]
                }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let config = LlmConfig {
            ollama_url: server.uri(),
            extraction_model: "test".to_string(),
            classification_model: "test".to_string(),
            embedding_model: "test".to_string(),
            azure_openai_url: None,
            azure_openai_key: None,
        };

        let client = LlmClient::new(&config).expect("should build");
        let vec = client
            .call_embeddings("nomic-embed-text", "hello world")
            .await
            .expect("should parse");

        assert_eq!(vec.len(), 3);
        assert!((vec[0] - 0.1).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn azure_fallback_on_ollama_connection_error() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        // Azure mock server
        let azure_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{
                    "message": { "content": "{\"fallback\": true}" }
                }]
            })))
            .expect(1)
            .mount(&azure_server)
            .await;

        let config = LlmConfig {
            // Point Ollama to a port that is not listening.
            ollama_url: "http://127.0.0.1:1".to_string(),
            extraction_model: "test".to_string(),
            classification_model: "test".to_string(),
            embedding_model: "test".to_string(),
            azure_openai_url: Some(azure_server.uri()),
            azure_openai_key: Some("test-key".to_string()),
        };

        let client = LlmClient::new(&config).expect("should build");
        let result = client
            .call_llm("test-model", "system", "user")
            .await
            .expect("should fall back to Azure");

        assert_eq!(
            result.get("fallback").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[tokio::test]
    async fn azure_fallback_returns_no_fallback_when_not_configured() {
        let config = LlmConfig {
            ollama_url: "http://127.0.0.1:1".to_string(),
            extraction_model: "test".to_string(),
            classification_model: "test".to_string(),
            embedding_model: "test".to_string(),
            azure_openai_url: None,
            azure_openai_key: None,
        };

        let client = LlmClient::new(&config).expect("should build");
        let err = client
            .call_llm("test-model", "system", "user")
            .await
            .unwrap_err();

        // Should get NoFallback since Ollama is unreachable and Azure is not configured.
        assert!(matches!(err, LlmError::NoFallback));
    }

    // -- 429 rate limit retry behavior --------------------------------------

    #[tokio::test]
    async fn retry_on_429_rate_limit_then_success() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        // First two calls return 429 (rate limit), third succeeds.
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(429).set_body_string("rate limited"),
            )
            .up_to_n_times(2)
            .expect(2)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{
                    "message": { "content": "{\"ok\": true}" }
                }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let config = LlmConfig {
            ollama_url: server.uri(),
            extraction_model: "test".to_string(),
            classification_model: "test".to_string(),
            embedding_model: "test".to_string(),
            azure_openai_url: None,
            azure_openai_key: None,
        };

        let client = LlmClient::new(&config).expect("should build");
        let result = client
            .call_llm("test-model", "system", "user")
            .await
            .expect("should succeed after 429 retries");

        assert_eq!(result.get("ok").and_then(|v| v.as_bool()), Some(true));
    }

    #[tokio::test]
    async fn retry_exhausted_on_persistent_429() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        // All calls return 429 — retries should be exhausted.
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(429).set_body_string("rate limited"),
            )
            .expect(3)
            .mount(&server)
            .await;

        let config = LlmConfig {
            ollama_url: server.uri(),
            extraction_model: "test".to_string(),
            classification_model: "test".to_string(),
            embedding_model: "test".to_string(),
            azure_openai_url: None,
            azure_openai_key: None,
        };

        let client = LlmClient::new(&config).expect("should build");
        let err = client
            .call_llm("test-model", "system", "user")
            .await
            .unwrap_err();

        assert!(matches!(err, LlmError::RetriesExhausted(_)));
    }
}
