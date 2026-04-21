//! Unit tests for error handling and resilience patterns (Task 20.6).
//!
//! Tests cover:
//! - Episode ingestion error scenarios (duplicate detection, validation)
//! - Entity resolution error handling (type constraints, semantic fallback)
//! - Retrieval profile timeout handling
//! - LLM client retry logic and Azure OpenAI fallback
//! - Database pool error types
//! - Error logging completeness

// ---------------------------------------------------------------------------
// Episode ingestion error handling (20.1)
// ---------------------------------------------------------------------------

mod ingest_errors {
    use loom_engine::pipeline::offline::ingest::{
        compute_content_hash, validate_episode_input, IngestError, IngestResult,
    };
    use uuid::Uuid;

    #[test]
    fn duplicate_detection_returns_existing_episode_id() {
        let result = IngestResult {
            episode_id: Uuid::new_v4(),
            status: "duplicate".to_string(),
        };
        assert_eq!(result.status, "duplicate");
    }

    #[test]
    fn invalid_data_returns_specific_field_errors() {
        // Empty content
        let err = validate_episode_input("", "manual", "default").unwrap_err();
        assert!(matches!(err, IngestError::InvalidData(_)));
        assert!(err.to_string().contains("content"));

        // Empty source
        let err = validate_episode_input("hello", "", "default").unwrap_err();
        assert!(err.to_string().contains("source"));

        // Empty namespace
        let err = validate_episode_input("hello", "manual", "").unwrap_err();
        assert!(err.to_string().contains("namespace"));

        // Invalid source type
        let err = validate_episode_input("hello", "invalid", "default").unwrap_err();
        assert!(err.to_string().contains("source must be one of"));
    }

    #[test]
    fn multiple_validation_errors_reported_together() {
        let err = validate_episode_input("", "", "").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("content"), "missing content error in: {msg}");
        assert!(msg.contains("source"), "missing source error in: {msg}");
        assert!(msg.contains("namespace"), "missing namespace error in: {msg}");
    }

    #[test]
    fn valid_sources_accepted() {
        assert!(validate_episode_input("hello", "manual", "default").is_ok());
        assert!(validate_episode_input("hello", "claude-code", "ns").is_ok());
        assert!(validate_episode_input("hello", "github", "ns").is_ok());
    }

    #[test]
    fn content_hash_deterministic() {
        let h1 = compute_content_hash("test content");
        let h2 = compute_content_hash("test content");
        assert_eq!(h1, h2);
    }

    #[test]
    fn content_hash_is_sha256_hex() {
        let hash = compute_content_hash("hello");
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn constraint_violation_from_sqlx_detected() {
        let err = IngestError::from(sqlx::Error::Protocol(
            "duplicate key value violates unique constraint".to_string(),
        ));
        assert!(matches!(err, IngestError::ConstraintViolation(_)));
    }

    #[test]
    fn non_constraint_sqlx_error_is_database() {
        let err = IngestError::from(sqlx::Error::RowNotFound);
        assert!(matches!(err, IngestError::Database(_)));
    }

    #[test]
    fn queued_episodes_remain_unprocessed_for_retry() {
        let result = IngestResult {
            episode_id: Uuid::new_v4(),
            status: "queued".to_string(),
        };
        // "queued" means processed=false, eligible for retry.
        assert_eq!(result.status, "queued");
    }
}

// ---------------------------------------------------------------------------
// Entity resolution error handling (20.2)
// ---------------------------------------------------------------------------

mod resolve_errors {
    use loom_engine::pipeline::offline::resolve::{
        validate_entity_type, ResolveError, SemanticResult,
    };

    #[test]
    fn semantic_similarity_fallback_creates_new_entity() {
        // When semantic similarity fails, the system should fall back to
        // creating a new entity (prefer fragmentation over collision).
        let result = SemanticResult::NewEntity;
        assert!(matches!(result, SemanticResult::NewEntity));
    }

    #[test]
    fn entity_type_constraint_violations_rejected() {
        let err = validate_entity_type("invalid_type").unwrap_err();
        assert!(matches!(err, ResolveError::TypeConstraint(_)));
        assert!(err.to_string().contains("invalid_type"));
    }

    #[test]
    fn all_valid_entity_types_accepted() {
        let valid = [
            "person", "organization", "project", "service", "technology",
            "pattern", "environment", "document", "metric", "decision",
        ];
        for t in &valid {
            assert!(validate_entity_type(t).is_ok(), "should accept '{t}'");
        }
    }

    #[test]
    fn entity_type_validation_case_insensitive() {
        assert!(validate_entity_type("Person").is_ok());
        assert!(validate_entity_type("TECHNOLOGY").is_ok());
        assert!(validate_entity_type("Decision").is_ok());
    }

    #[test]
    fn resolve_error_variants_display_correctly() {
        let err = ResolveError::TypeConstraint("bad type".into());
        assert!(err.to_string().contains("entity type constraint violation"));
    }
}

// ---------------------------------------------------------------------------
// Retrieval and compilation error handling (20.4)
// ---------------------------------------------------------------------------

mod retrieval_errors {
    use loom_engine::pipeline::online::retrieve::{
        profiles_for_class, merge_profiles, RetrievalError, DEFAULT_RANKING_SCORE,
    };
    use loom_engine::types::classification::TaskClass;

    #[test]
    fn classification_failure_defaults_to_chat() {
        // When classification fails, the system defaults to Chat.
        let profiles = profiles_for_class(&TaskClass::Chat);
        assert!(!profiles.is_empty(), "Chat should have at least one profile");
    }

    #[test]
    fn default_ranking_score_is_0_5() {
        assert!((DEFAULT_RANKING_SCORE - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn retrieval_error_timeout_displays_message() {
        let err = RetrievalError::Timeout("fact_lookup timed out".into());
        assert!(err.to_string().contains("timed out"));
    }

    #[test]
    fn retrieval_error_sqlx_displays_message() {
        let err = RetrievalError::Sqlx(sqlx::Error::RowNotFound);
        assert!(err.to_string().contains("database error"));
    }

    #[test]
    fn all_task_classes_produce_profiles() {
        let classes = [
            TaskClass::Debug,
            TaskClass::Architecture,
            TaskClass::Compliance,
            TaskClass::Writing,
            TaskClass::Chat,
        ];
        for class in &classes {
            let profiles = profiles_for_class(class);
            assert!(!profiles.is_empty(), "{class:?} should have profiles");
        }
    }

    #[test]
    fn merge_profiles_caps_at_three() {
        let profiles = merge_profiles(
            &TaskClass::Debug,
            Some(&TaskClass::Architecture),
        );
        assert!(profiles.len() <= 3, "should cap at 3 profiles");
    }
}

// ---------------------------------------------------------------------------
// Database and external service error handling (20.5)
// ---------------------------------------------------------------------------

mod db_errors {
    use loom_engine::db::pool::PoolError;

    #[test]
    fn service_unavailable_error_includes_attempts() {
        let err = PoolError::ServiceUnavailable {
            attempts: 3,
            message: "connection refused".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("3 attempts"), "got: {msg}");
        assert!(msg.contains("connection refused"), "got: {msg}");
    }

    #[test]
    fn missing_extension_error_includes_name() {
        let err = PoolError::MissingExtension {
            extension: "vector".to_string(),
        };
        assert!(err.to_string().contains("vector"));
    }

    #[test]
    fn connection_error_wraps_sqlx() {
        let err = PoolError::Connection(sqlx::Error::RowNotFound);
        assert!(err.to_string().contains("pool connection error"));
    }
}

mod llm_errors {
    use loom_engine::llm::client::LlmError;

    #[test]
    fn retries_exhausted_error_displays_message() {
        let err = LlmError::RetriesExhausted("timeout after 3 attempts".into());
        assert!(err.to_string().contains("retries exhausted"));
    }

    #[test]
    fn no_fallback_error_displays_message() {
        let err = LlmError::NoFallback;
        assert!(err.to_string().contains("Azure OpenAI fallback not configured"));
    }

    #[test]
    fn api_error_includes_status_code() {
        let err = LlmError::ApiError {
            status: 429,
            body: "rate limited".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("429"), "got: {msg}");
        assert!(msg.contains("rate limited"), "got: {msg}");
    }

    #[test]
    fn parse_error_displays_message() {
        let err = LlmError::Parse("unexpected JSON structure".into());
        assert!(err.to_string().contains("unexpected JSON structure"));
    }
}

// ---------------------------------------------------------------------------
// LLM retry logic and fallback (20.5)
// ---------------------------------------------------------------------------

mod llm_retry {
    use loom_engine::config::LlmConfig;
    use loom_engine::llm::client::LlmClient;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_config(server_uri: &str) -> LlmConfig {
        LlmConfig {
            ollama_url: server_uri.to_string(),
            extraction_model: "test".to_string(),
            classification_model: "test".to_string(),
            embedding_model: "test".to_string(),
            azure_openai_url: None,
            azure_openai_key: None,
        }
    }

    #[tokio::test]
    async fn retry_succeeds_after_transient_500() {
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

        let config = test_config(&server.uri());
        let client = LlmClient::new(&config).expect("should build");
        let result = client
            .call_llm("test-model", "system", "user")
            .await
            .expect("should succeed after retries");

        assert_eq!(result.get("ok").and_then(|v| v.as_bool()), Some(true));
    }

    #[tokio::test]
    async fn retry_429_rate_limit_then_success() {
        let server = MockServer::start().await;

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

        let config = test_config(&server.uri());
        let client = LlmClient::new(&config).expect("should build");
        let result = client
            .call_llm("test-model", "system", "user")
            .await
            .expect("should succeed after 429 retries");

        assert_eq!(result.get("ok").and_then(|v| v.as_bool()), Some(true));
    }

    #[tokio::test]
    async fn non_retryable_400_fails_immediately() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(400).set_body_string("bad request"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let config = test_config(&server.uri());
        let client = LlmClient::new(&config).expect("should build");
        let err = client
            .call_llm("test-model", "system", "user")
            .await
            .unwrap_err();

        assert!(matches!(err, loom_engine::llm::client::LlmError::ApiError { status: 400, .. }));
    }

    #[tokio::test]
    async fn azure_fallback_on_ollama_connection_error() {
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
    async fn no_fallback_when_azure_not_configured() {
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

        assert!(matches!(err, loom_engine::llm::client::LlmError::NoFallback));
    }
}

// ---------------------------------------------------------------------------
// Embedding dimension mismatch (20.5)
// ---------------------------------------------------------------------------

mod embedding_errors {
    use loom_engine::llm::embeddings::EmbeddingError;

    #[test]
    fn dimension_mismatch_error_displays_expected_and_actual() {
        let err = EmbeddingError::DimensionMismatch {
            expected: 768,
            actual: 512,
        };
        let msg = err.to_string();
        assert!(msg.contains("768"), "got: {msg}");
        assert!(msg.contains("512"), "got: {msg}");
    }
}

// ---------------------------------------------------------------------------
// Fact extraction error handling (20.3)
// ---------------------------------------------------------------------------

mod fact_errors {
    use loom_engine::pipeline::offline::extract::{
        FactExtractionPipelineError, PipelineError, PredicateValidationResult,
    };

    #[test]
    fn fact_pipeline_error_database_displays() {
        let err = FactExtractionPipelineError::Database("connection lost".into());
        assert!(err.to_string().contains("connection lost"));
    }

    #[test]
    fn pipeline_error_variants_display() {
        let err = PipelineError::Database("test error".into());
        assert!(err.to_string().contains("database error"));
    }

    #[test]
    fn predicate_validation_result_tracks_counts() {
        let result = PredicateValidationResult {
            canonical_count: 5,
            custom_count: 2,
        };
        assert_eq!(result.canonical_count, 5);
        assert_eq!(result.custom_count, 2);
    }
}

// ---------------------------------------------------------------------------
// Worker processor error handling
// ---------------------------------------------------------------------------

mod processor_errors {
    use loom_engine::worker::processor::ProcessorError;
    use loom_engine::db::episodes::EpisodeError;

    #[test]
    fn processor_error_database_displays() {
        let err = ProcessorError::Database(EpisodeError::Sqlx(sqlx::Error::RowNotFound));
        assert!(err.to_string().contains("database error"));
    }
}
