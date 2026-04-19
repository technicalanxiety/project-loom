//! Intent classification stage for the online pipeline.
//!
//! Classifies user queries into one of five [`TaskClass`] variants to
//! determine retrieval strategy. Delegates to the [`llm::classification`]
//! module for the actual keyword pre-check and LLM call, then logs the
//! classification result (primary class, secondary class, confidence scores)
//! via [`tracing`] for audit purposes.
//!
//! # Pipeline Position
//!
//! ```text
//! loom_think → **classify** → namespace → retrieve → weight → rank → compile
//! ```

use std::time::Instant;

use thiserror::Error;

use crate::config::LlmConfig;
use crate::llm::classification::{self, ClassificationError, ClassificationOutput};
use crate::llm::client::LlmClient;
use crate::types::classification::{ClassificationResult, TaskClass};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors produced by the classification pipeline stage.
#[derive(Debug, Error)]
pub enum ClassifyStageError {
    /// The underlying classification module returned an error.
    #[error("classification failed: {0}")]
    Classification(#[from] ClassificationError),
}

// ---------------------------------------------------------------------------
// Stage output
// ---------------------------------------------------------------------------

/// Output of the classification pipeline stage.
///
/// Carries the classification result along with timing and model provenance
/// for downstream audit logging.
#[derive(Debug, Clone)]
pub struct ClassifyStageOutput {
    /// The classification result with primary/secondary classes and scores.
    pub result: ClassificationResult,
    /// The model identifier used for classification (e.g. "keyword_precheck"
    /// or "gemma4:e4b").
    pub model: String,
    /// Wall-clock time spent in the classification stage (milliseconds).
    pub latency_ms: u64,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Run the intent classification stage of the online pipeline.
///
/// 1. Delegates to [`classification::classify_intent`] which performs keyword
///    pre-check followed by LLM fallback for ambiguous queries.
/// 2. Logs the classification result (primary class, optional secondary
///    class, confidence scores, model, latency) via structured tracing.
/// 3. Returns a [`ClassifyStageOutput`] for downstream pipeline stages and
///    audit log insertion.
///
/// # Errors
///
/// Returns [`ClassifyStageError`] if the underlying classification call
/// fails. On LLM parse failures the classification module itself defaults
/// to [`TaskClass::Chat`], so errors here typically indicate network or
/// infrastructure issues.
pub async fn classify_query(
    client: &LlmClient,
    config: &LlmConfig,
    query: &str,
) -> Result<ClassifyStageOutput, ClassifyStageError> {
    let start = Instant::now();

    let output: ClassificationOutput =
        classification::classify_intent(client, config, query).await?;

    let latency_ms = start.elapsed().as_millis() as u64;

    // Log classification result for audit trail.
    tracing::info!(
        primary_class = %output.result.primary_class,
        primary_confidence = output.result.primary_confidence,
        secondary_class = output
            .result
            .secondary_class
            .as_ref()
            .map(|c| c.to_string())
            .unwrap_or_default(),
        secondary_confidence = output.result.secondary_confidence.unwrap_or(0.0),
        model = %output.model,
        latency_ms,
        "classification stage complete"
    );

    Ok(ClassifyStageOutput {
        result: output.result,
        model: output.model,
        latency_ms,
    })
}

/// Apply a task class override, bypassing classification entirely.
///
/// Used when the caller provides an explicit `task_class_override` in the
/// `loom_think` request. Returns a [`ClassifyStageOutput`] with confidence
/// 1.0 and no secondary class.
pub fn apply_override(task_class: TaskClass) -> ClassifyStageOutput {
    tracing::info!(
        primary_class = %task_class,
        model = "override",
        "classification overridden by caller"
    );

    ClassifyStageOutput {
        result: ClassificationResult {
            primary_class: task_class,
            secondary_class: None,
            primary_confidence: 1.0,
            secondary_confidence: None,
        },
        model: "override".to_string(),
        latency_ms: 0,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Build a test `LlmClient` + `LlmConfig` pointing at the given mock
    /// server URI.
    fn test_fixtures(server_uri: &str) -> (LlmClient, LlmConfig) {
        let config = LlmConfig {
            ollama_url: server_uri.to_string(),
            extraction_model: "test-model".to_string(),
            classification_model: "test-model".to_string(),
            embedding_model: "nomic-embed-text".to_string(),
            azure_openai_url: None,
            azure_openai_key: None,
        };
        let client = LlmClient::new(&config).expect("should build client");
        (client, config)
    }

    // -- classify_query with keyword match ----------------------------------

    #[tokio::test]
    async fn classify_query_keyword_match_returns_debug() {
        let server = MockServer::start().await;

        // No LLM call expected — keyword match short-circuits.
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;

        let (client, config) = test_fixtures(&server.uri());
        let output = classify_query(&client, &config, "There is an error in the parser")
            .await
            .expect("should classify");

        assert_eq!(output.result.primary_class, TaskClass::Debug);
        assert!((output.result.primary_confidence - 1.0).abs() < f64::EPSILON);
        assert!(output.result.secondary_class.is_none());
        assert_eq!(output.model, "keyword_precheck");
    }

    #[tokio::test]
    async fn classify_query_keyword_match_returns_architecture() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;

        let (client, config) = test_fixtures(&server.uri());
        let output = classify_query(&client, &config, "Explain the system architecture")
            .await
            .expect("should classify");

        assert_eq!(output.result.primary_class, TaskClass::Architecture);
    }

    #[tokio::test]
    async fn classify_query_keyword_match_returns_compliance() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;

        let (client, config) = test_fixtures(&server.uri());
        let output = classify_query(&client, &config, "Run the compliance audit")
            .await
            .expect("should classify");

        assert_eq!(output.result.primary_class, TaskClass::Compliance);
    }

    #[tokio::test]
    async fn classify_query_keyword_match_returns_writing() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;

        let (client, config) = test_fixtures(&server.uri());
        let output = classify_query(&client, &config, "Write a summary of the project")
            .await
            .expect("should classify");

        assert_eq!(output.result.primary_class, TaskClass::Writing);
    }

    // -- classify_query with LLM fallback -----------------------------------

    #[tokio::test]
    async fn classify_query_llm_fallback_with_secondary() {
        let server = MockServer::start().await;

        let llm_content = json!({
            "classes": [
                {"class": "architecture", "confidence": 0.50},
                {"class": "debug", "confidence": 0.30},
                {"class": "compliance", "confidence": 0.10},
                {"class": "writing", "confidence": 0.05},
                {"class": "chat", "confidence": 0.05}
            ]
        });

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{
                    "message": { "content": llm_content.to_string() }
                }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let (client, config) = test_fixtures(&server.uri());
        let output = classify_query(&client, &config, "How does the project work?")
            .await
            .expect("should classify via LLM");

        assert_eq!(output.result.primary_class, TaskClass::Architecture);
        // Gap is 0.50 - 0.30 = 0.20 < 0.3, so secondary should be present.
        assert_eq!(output.result.secondary_class, Some(TaskClass::Debug));
        assert_eq!(output.model, "test-model");
        assert!(output.latency_ms < 5000); // sanity check
    }

    #[tokio::test]
    async fn classify_query_llm_fallback_no_secondary() {
        let server = MockServer::start().await;

        let llm_content = json!({
            "classes": [
                {"class": "debug", "confidence": 0.85},
                {"class": "architecture", "confidence": 0.05},
                {"class": "compliance", "confidence": 0.04},
                {"class": "writing", "confidence": 0.03},
                {"class": "chat", "confidence": 0.03}
            ]
        });

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{
                    "message": { "content": llm_content.to_string() }
                }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let (client, config) = test_fixtures(&server.uri());
        let output = classify_query(&client, &config, "Tell me about the project")
            .await
            .expect("should classify via LLM");

        assert_eq!(output.result.primary_class, TaskClass::Debug);
        // Gap is 0.85 - 0.05 = 0.80 >= 0.3, so no secondary.
        assert!(output.result.secondary_class.is_none());
    }

    // -- apply_override -----------------------------------------------------

    #[test]
    fn apply_override_returns_full_confidence() {
        let output = apply_override(TaskClass::Compliance);

        assert_eq!(output.result.primary_class, TaskClass::Compliance);
        assert!((output.result.primary_confidence - 1.0).abs() < f64::EPSILON);
        assert!(output.result.secondary_class.is_none());
        assert_eq!(output.model, "override");
        assert_eq!(output.latency_ms, 0);
    }

    #[test]
    fn apply_override_works_for_all_classes() {
        for class in [
            TaskClass::Debug,
            TaskClass::Architecture,
            TaskClass::Compliance,
            TaskClass::Writing,
            TaskClass::Chat,
        ] {
            let output = apply_override(class.clone());
            assert_eq!(output.result.primary_class, class);
        }
    }

    // -- ClassifyStageError display -----------------------------------------

    #[test]
    fn classify_stage_error_display() {
        let inner = ClassificationError::Deserialization("bad json".into());
        let err = ClassifyStageError::Classification(inner);
        assert!(err.to_string().contains("classification failed"));
        assert!(err.to_string().contains("bad json"));
    }

    // -- latency tracking ---------------------------------------------------

    #[tokio::test]
    async fn classify_query_records_latency() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;

        let (client, config) = test_fixtures(&server.uri());
        let output = classify_query(&client, &config, "There is a bug")
            .await
            .expect("should classify");

        // Keyword match should be near-instant.
        assert!(output.latency_ms < 1000);
    }

    // -- 10.11 additional unit tests ----------------------------------------

    /// Test that the default chat fallback has zero confidence and no
    /// secondary class (Requirement 8.5).
    #[tokio::test]
    async fn classify_query_defaults_to_chat_for_ambiguous_input() {
        let server = MockServer::start().await;

        // LLM returns very low confidence for all classes.
        let llm_content = json!({
            "classes": [
                {"class": "debug", "confidence": 0.05},
                {"class": "architecture", "confidence": 0.04},
                {"class": "compliance", "confidence": 0.03},
                {"class": "writing", "confidence": 0.02},
                {"class": "chat", "confidence": 0.01}
            ]
        });

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{
                    "message": { "content": llm_content.to_string() }
                }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let (client, config) = test_fixtures(&server.uri());
        let output = classify_query(&client, &config, "hello there")
            .await
            .expect("should classify");

        // Very low confidence → defaults to Chat.
        assert_eq!(output.result.primary_class, TaskClass::Chat);
        assert!(output.result.secondary_class.is_none());
    }

    /// Test that apply_override always produces confidence 1.0 and zero
    /// latency for every task class (Requirement 8.1).
    #[test]
    fn apply_override_produces_consistent_output_for_all_classes() {
        for class in [
            TaskClass::Debug,
            TaskClass::Architecture,
            TaskClass::Compliance,
            TaskClass::Writing,
            TaskClass::Chat,
        ] {
            let output = apply_override(class.clone());
            assert_eq!(output.result.primary_class, class);
            assert!((output.result.primary_confidence - 1.0).abs() < f64::EPSILON);
            assert!(output.result.secondary_class.is_none());
            assert!(output.result.secondary_confidence.is_none());
            assert_eq!(output.model, "override");
            assert_eq!(output.latency_ms, 0);
        }
    }

    /// Test that keyword match queries never trigger an LLM call, verifying
    /// the two-stage classification pipeline (Requirement 8.1).
    #[tokio::test]
    async fn classify_query_keyword_match_skips_llm_for_all_classes() {
        let server = MockServer::start().await;

        // Mount a mock that should NOT be called.
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;

        let (client, config) = test_fixtures(&server.uri());

        let test_cases = vec![
            ("There is an error in the code", TaskClass::Debug),
            ("Explain the system architecture", TaskClass::Architecture),
            ("Run the compliance audit", TaskClass::Compliance),
            ("Write a summary of the project", TaskClass::Writing),
        ];

        for (query, expected_class) in test_cases {
            let output = classify_query(&client, &config, query)
                .await
                .expect("should classify");
            assert_eq!(
                output.result.primary_class, expected_class,
                "Query '{}' should classify as {:?}",
                query, expected_class
            );
            assert_eq!(output.model, "keyword_precheck");
        }
    }

    /// Test that LLM fallback correctly records secondary class when
    /// confidence gap is small (Requirement 8.3).
    #[tokio::test]
    async fn classify_query_llm_records_secondary_when_gap_small() {
        let server = MockServer::start().await;

        // Gap between top two: 0.45 - 0.30 = 0.15 < 0.3
        let llm_content = json!({
            "classes": [
                {"class": "debug", "confidence": 0.45},
                {"class": "compliance", "confidence": 0.30},
                {"class": "architecture", "confidence": 0.15},
                {"class": "writing", "confidence": 0.05},
                {"class": "chat", "confidence": 0.05}
            ]
        });

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{
                    "message": { "content": llm_content.to_string() }
                }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let (client, config) = test_fixtures(&server.uri());
        let output = classify_query(&client, &config, "Tell me about the project")
            .await
            .expect("should classify via LLM");

        assert_eq!(output.result.primary_class, TaskClass::Debug);
        assert_eq!(output.result.secondary_class, Some(TaskClass::Compliance));
        assert!((output.result.secondary_confidence.unwrap() - 0.30).abs() < f64::EPSILON);
    }
}
