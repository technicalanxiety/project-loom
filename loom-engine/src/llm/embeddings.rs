//! Embedding generation via nomic-embed-text (768 dimensions) through Ollama.
//!
//! Provides functions to generate embeddings for episodes, entities, and
//! arbitrary text. All embeddings are validated to be exactly 768 dimensions
//! (the output size of nomic-embed-text).

use thiserror::Error;

use super::client::{LlmClient, LlmError};
use crate::config::LlmConfig;

/// Expected embedding dimension for nomic-embed-text.
pub const EXPECTED_DIMENSION: usize = 768;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors produced by the embedding generation layer.
#[derive(Debug, Error)]
pub enum EmbeddingError {
    /// An error from the underlying LLM client (HTTP, parse, retries, etc.).
    #[error("LLM client error: {0}")]
    Llm(#[from] LlmError),

    /// The returned embedding has an unexpected number of dimensions.
    #[error("embedding dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch {
        /// The expected number of dimensions.
        expected: usize,
        /// The actual number of dimensions returned.
        actual: usize,
    },
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Generate a 768-dimension embedding for the given text.
///
/// Calls `client.call_embeddings()` with the configured embedding model and
/// validates that the returned vector has exactly [`EXPECTED_DIMENSION`]
/// elements.
///
/// # Errors
///
/// Returns [`EmbeddingError::Llm`] if the upstream call fails, or
/// [`EmbeddingError::DimensionMismatch`] if the vector length is not 768.
#[tracing::instrument(skip(client, config), fields(text_len = text.len()))]
pub async fn generate_embedding(
    client: &LlmClient,
    config: &LlmConfig,
    text: &str,
) -> Result<Vec<f32>, EmbeddingError> {
    tracing::debug!("generating embedding for text ({} chars)", text.len());

    let embedding = client
        .call_embeddings(&config.embedding_model, text)
        .await?;

    validate_dimension(&embedding)?;

    tracing::debug!(dimensions = embedding.len(), "embedding generated successfully");
    Ok(embedding)
}

/// Generate a 768-dimension embedding for an entity by combining its name
/// and a context snippet.
///
/// The name and context are joined as `"{name}: {context}"` to produce a
/// single input string that captures both the entity identity and its
/// surrounding context from the source episode.
///
/// # Errors
///
/// Returns [`EmbeddingError`] on LLM failure or dimension mismatch.
#[tracing::instrument(skip(client, config), fields(entity_name = %name))]
pub async fn generate_entity_embedding(
    client: &LlmClient,
    config: &LlmConfig,
    name: &str,
    context: &str,
) -> Result<Vec<f32>, EmbeddingError> {
    let input = format!("{name}: {context}");
    tracing::debug!(
        entity_name = %name,
        context_len = context.len(),
        "generating entity embedding"
    );
    generate_embedding(client, config, &input).await
}

/// Generate a 768-dimension embedding for episode content.
///
/// This is a convenience wrapper around [`generate_embedding`] that accepts
/// the raw episode content string.
///
/// # Errors
///
/// Returns [`EmbeddingError`] on LLM failure or dimension mismatch.
#[tracing::instrument(skip(client, config), fields(content_len = content.len()))]
pub async fn generate_episode_embedding(
    client: &LlmClient,
    config: &LlmConfig,
    content: &str,
) -> Result<Vec<f32>, EmbeddingError> {
    tracing::debug!(content_len = content.len(), "generating episode embedding");
    generate_embedding(client, config, content).await
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Validate that an embedding vector has exactly [`EXPECTED_DIMENSION`]
/// elements.
///
/// On mismatch, logs an error with the expected and actual dimensions
/// to aid debugging model configuration issues.
fn validate_dimension(embedding: &[f32]) -> Result<(), EmbeddingError> {
    if embedding.len() != EXPECTED_DIMENSION {
        tracing::error!(
            expected = EXPECTED_DIMENSION,
            actual = embedding.len(),
            "embedding dimension mismatch — check embedding model configuration"
        );
        return Err(EmbeddingError::DimensionMismatch {
            expected: EXPECTED_DIMENSION,
            actual: embedding.len(),
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Helper: build an `LlmClient` + `LlmConfig` pointing at the given mock
    /// server.
    fn test_config(server_uri: &str) -> (LlmClient, LlmConfig) {
        let config = LlmConfig {
            ollama_url: server_uri.to_string(),
            extraction_model: "test".to_string(),
            classification_model: "test".to_string(),
            embedding_model: "nomic-embed-text".to_string(),
            azure_openai_url: None,
            azure_openai_key: None,
        };
        let client = LlmClient::new(&config).expect("should build client");
        (client, config)
    }

    /// Helper: build a mock embedding response with the given dimension.
    fn embedding_response(dim: usize) -> serde_json::Value {
        let vec: Vec<f32> = (0..dim).map(|i| i as f32 * 0.001).collect();
        json!({
            "data": [{
                "embedding": vec
            }]
        })
    }

    // -- generate_embedding -------------------------------------------------

    #[tokio::test]
    async fn generate_embedding_returns_768d_vector() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(embedding_response(768)),
            )
            .expect(1)
            .mount(&server)
            .await;

        let (client, config) = test_config(&server.uri());
        let result = generate_embedding(&client, &config, "hello world").await;

        let vec = result.expect("should succeed");
        assert_eq!(vec.len(), 768);
    }

    #[tokio::test]
    async fn generate_embedding_rejects_wrong_dimension() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(embedding_response(512)),
            )
            .expect(1)
            .mount(&server)
            .await;

        let (client, config) = test_config(&server.uri());
        let err = generate_embedding(&client, &config, "hello world")
            .await
            .unwrap_err();

        match err {
            EmbeddingError::DimensionMismatch {
                expected: 768,
                actual: 512,
            } => {} // expected
            other => panic!("unexpected error: {other}"),
        }
    }

    #[tokio::test]
    async fn generate_embedding_propagates_llm_error() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(400).set_body_string("bad request"))
            .expect(1)
            .mount(&server)
            .await;

        let (client, config) = test_config(&server.uri());
        let err = generate_embedding(&client, &config, "hello world")
            .await
            .unwrap_err();

        assert!(matches!(err, EmbeddingError::Llm(_)));
    }

    // -- generate_entity_embedding ------------------------------------------

    #[tokio::test]
    async fn generate_entity_embedding_combines_name_and_context() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(embedding_response(768)),
            )
            .expect(1)
            .mount(&server)
            .await;

        let (client, config) = test_config(&server.uri());
        let result =
            generate_entity_embedding(&client, &config, "Rust", "A systems programming language")
                .await;

        let vec = result.expect("should succeed");
        assert_eq!(vec.len(), 768);
    }

    // -- generate_episode_embedding -----------------------------------------

    #[tokio::test]
    async fn generate_episode_embedding_succeeds() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(embedding_response(768)),
            )
            .expect(1)
            .mount(&server)
            .await;

        let (client, config) = test_config(&server.uri());
        let result = generate_episode_embedding(
            &client,
            &config,
            "Discussed APIM authentication flow changes with the team.",
        )
        .await;

        let vec = result.expect("should succeed");
        assert_eq!(vec.len(), 768);
    }

    // -- validate_dimension -------------------------------------------------

    #[test]
    fn validate_dimension_accepts_768() {
        let vec = vec![0.0_f32; 768];
        assert!(validate_dimension(&vec).is_ok());
    }

    #[test]
    fn validate_dimension_rejects_wrong_size() {
        let vec = vec![0.0_f32; 512];
        let err = validate_dimension(&vec).unwrap_err();
        match err {
            EmbeddingError::DimensionMismatch {
                expected: 768,
                actual: 512,
            } => {}
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn validate_dimension_rejects_empty() {
        let vec: Vec<f32> = vec![];
        let err = validate_dimension(&vec).unwrap_err();
        match err {
            EmbeddingError::DimensionMismatch {
                expected: 768,
                actual: 0,
            } => {}
            other => panic!("unexpected error: {other}"),
        }
    }

    // -- EmbeddingError Display ---------------------------------------------

    #[test]
    fn embedding_error_display_messages() {
        let err = EmbeddingError::DimensionMismatch {
            expected: 768,
            actual: 512,
        };
        assert!(err.to_string().contains("768"));
        assert!(err.to_string().contains("512"));

        let llm_err = LlmError::NoFallback;
        let err = EmbeddingError::Llm(llm_err);
        assert!(err.to_string().contains("LLM client error"));
    }

    // -- generate_embedding with various text inputs ------------------------

    #[tokio::test]
    async fn generate_embedding_with_empty_text() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(embedding_response(768)),
            )
            .expect(1)
            .mount(&server)
            .await;

        let (client, config) = test_config(&server.uri());
        let result = generate_embedding(&client, &config, "").await;

        let vec = result.expect("should succeed with empty text");
        assert_eq!(vec.len(), 768);
    }

    #[tokio::test]
    async fn generate_embedding_with_very_long_text() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(embedding_response(768)),
            )
            .expect(1)
            .mount(&server)
            .await;

        let (client, config) = test_config(&server.uri());
        // Build a string longer than 10000 characters.
        let long_text = "a".repeat(15_000);
        let result = generate_embedding(&client, &config, &long_text).await;

        let vec = result.expect("should succeed with very long text");
        assert_eq!(vec.len(), 768);
    }

    #[tokio::test]
    async fn generate_embedding_with_unicode_emoji_text() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(embedding_response(768)),
            )
            .expect(1)
            .mount(&server)
            .await;

        let (client, config) = test_config(&server.uri());
        let result =
            generate_embedding(&client, &config, "Hello 🌍🚀 日本語テスト Ñoño café")
                .await;

        let vec = result.expect("should succeed with unicode/emoji text");
        assert_eq!(vec.len(), 768);
    }

    #[tokio::test]
    async fn generate_embedding_with_whitespace_only_text() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(embedding_response(768)),
            )
            .expect(1)
            .mount(&server)
            .await;

        let (client, config) = test_config(&server.uri());
        let result = generate_embedding(&client, &config, "   \t\n  ").await;

        let vec = result.expect("should succeed with whitespace-only text");
        assert_eq!(vec.len(), 768);
    }

    // -- Property 26: Embedding Dimension Consistency -----------------------
    // Validates: Requirements 21.1, 22.1, 46.1
    //
    // For any text input, when the mock Ollama returns a 768-dimension
    // vector the embedding service must accept it. When the mock returns
    // any other dimension the service must reject it with DimensionMismatch.

    // Property: validate_dimension accepts exactly 768 and rejects all others.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(20))]

        #[test]
        fn prop_validate_dimension_accepts_only_768(dim in 0_usize..2048) {
            let vec = vec![0.0_f32; dim];
            let result = validate_dimension(&vec);
            if dim == EXPECTED_DIMENSION {
                prop_assert!(result.is_ok(), "expected Ok for dim={dim}");
            } else {
                let err = result.unwrap_err();
                match err {
                    EmbeddingError::DimensionMismatch { expected, actual } => {
                        prop_assert_eq!(expected, EXPECTED_DIMENSION);
                        prop_assert_eq!(actual, dim);
                    }
                    other => prop_assert!(false, "unexpected error variant: {other}"),
                }
            }
        }
    }

    // Property: generate_embedding always returns exactly 768 dimensions
    // when the upstream mock returns 768, for any arbitrary input text.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(20))]

        #[test]
        fn prop_embedding_output_always_768d(text in ".{1,500}") {
            // Run the async test inside a tokio runtime.
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let server = MockServer::start().await;

                // Mock always returns a valid 768-d embedding.
                Mock::given(method("POST"))
                    .and(path("/v1/embeddings"))
                    .respond_with(
                        ResponseTemplate::new(200)
                            .set_body_json(embedding_response(768)),
                    )
                    .mount(&server)
                    .await;

                let (client, config) = test_config(&server.uri());
                let result = generate_embedding(&client, &config, &text).await;
                let vec = result.expect("should succeed for any input text");
                assert_eq!(vec.len(), EXPECTED_DIMENSION);
            });
        }
    }

    // Property: generate_embedding rejects any non-768 dimension returned
    // by the upstream, for arbitrary dimension values.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(20))]

        #[test]
        fn prop_embedding_rejects_wrong_dimension(
            dim in (0_usize..2048).prop_filter("not 768", |d| *d != 768)
        ) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let server = MockServer::start().await;

                Mock::given(method("POST"))
                    .and(path("/v1/embeddings"))
                    .respond_with(
                        ResponseTemplate::new(200)
                            .set_body_json(embedding_response(dim)),
                    )
                    .mount(&server)
                    .await;

                let (client, config) = test_config(&server.uri());
                let err = generate_embedding(&client, &config, "test input")
                    .await
                    .unwrap_err();

                match err {
                    EmbeddingError::DimensionMismatch { expected, actual } => {
                        assert_eq!(expected, EXPECTED_DIMENSION);
                        assert_eq!(actual, dim);
                    }
                    other => panic!("expected DimensionMismatch, got: {other}"),
                }
            });
        }
    }
}
