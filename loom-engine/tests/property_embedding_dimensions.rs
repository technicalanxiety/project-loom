//! Property-based tests for embedding dimension consistency.
//!
//! **Property 26: Embedding Dimension Consistency**
//! **Validates: Requirements 21.1, 22.1, 46.1**
//!
//! Tests that all generated embeddings have exactly 768 dimensions and that
//! dimension mismatches are correctly rejected with `EmbeddingError::DimensionMismatch`.
//!
//! Uses wiremock to mock Ollama embedding responses so no real LLM service is needed.

use proptest::prelude::*;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use loom_engine::config::LlmConfig;
use loom_engine::llm::client::LlmClient;
use loom_engine::llm::embeddings::{generate_embedding, EmbeddingError, EXPECTED_DIMENSION};

/// Build an `LlmClient` + `LlmConfig` pointing at the given mock server URI.
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

/// Build a mock embedding JSON response with the given number of dimensions.
fn embedding_response(dim: usize) -> serde_json::Value {
    let vec: Vec<f32> = (0..dim).map(|i| (i as f32) * 0.001).collect();
    json!({
        "data": [{
            "embedding": vec
        }]
    })
}

/// **Property 26: Embedding Dimension Consistency — Correct Dimensions**
///
/// **Validates: Requirements 21.1, 22.1, 46.1**
///
/// For arbitrary text inputs, when the Ollama mock returns a 768-dimension
/// embedding vector, `generate_embedding` should succeed and the resulting
/// vector should have exactly 768 elements.
mod correct_dimensions {
    use super::*;

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        #[test]
        fn embedding_has_768_dimensions(
            text in "\\PC{1,200}"
        ) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let server = MockServer::start().await;

                Mock::given(method("POST"))
                    .and(path("/v1/embeddings"))
                    .respond_with(
                        ResponseTemplate::new(200)
                            .set_body_json(embedding_response(EXPECTED_DIMENSION)),
                    )
                    .mount(&server)
                    .await;

                let (client, config) = test_config(&server.uri());
                let result = generate_embedding(&client, &config, &text).await;

                let vec = result.expect("embedding generation should succeed");
                prop_assert_eq!(
                    vec.len(),
                    EXPECTED_DIMENSION,
                    "embedding must have exactly {} dimensions, got {}",
                    EXPECTED_DIMENSION,
                    vec.len()
                );

                Ok(())
            })?;
        }
    }
}

/// **Property 26: Embedding Dimension Consistency — Wrong Dimensions**
///
/// **Validates: Requirements 21.1, 22.1, 46.1**
///
/// For arbitrary text inputs, when the Ollama mock returns an embedding with
/// the wrong number of dimensions (512, 1024, or 0), `generate_embedding`
/// should return `EmbeddingError::DimensionMismatch` with the correct
/// expected and actual values.
mod wrong_dimensions {
    use super::*;

    proptest! {
        #![proptest_config(ProptestConfig { cases: 100, .. ProptestConfig::default() })]

        #[test]
        fn wrong_dimension_returns_mismatch_error(
            text in "\\PC{1,200}",
            wrong_dim in prop::sample::select(vec![0_usize, 1, 256, 512, 767, 769, 1024, 1536]),
        ) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let server = MockServer::start().await;

                Mock::given(method("POST"))
                    .and(path("/v1/embeddings"))
                    .respond_with(
                        ResponseTemplate::new(200)
                            .set_body_json(embedding_response(wrong_dim)),
                    )
                    .mount(&server)
                    .await;

                let (client, config) = test_config(&server.uri());
                let result = generate_embedding(&client, &config, &text).await;

                let err = result.expect_err(
                    "embedding generation should fail for wrong dimensions",
                );

                match err {
                    EmbeddingError::DimensionMismatch { expected, actual } => {
                        prop_assert_eq!(
                            expected,
                            EXPECTED_DIMENSION,
                            "expected field should be {}",
                            EXPECTED_DIMENSION
                        );
                        prop_assert_eq!(
                            actual,
                            wrong_dim,
                            "actual field should match the mock dimension {}",
                            wrong_dim
                        );
                    }
                    other => {
                        prop_assert!(
                            false,
                            "expected DimensionMismatch error, got: {:?}",
                            other
                        );
                    }
                }

                Ok(())
            })?;
        }
    }
}
