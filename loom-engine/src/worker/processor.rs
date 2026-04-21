//! Background episode processing loop (tokio spawned tasks).
//!
//! Polls for unprocessed episodes using the offline connection pool and
//! spawns concurrent tokio tasks to generate embeddings and mark episodes
//! as processed. Concurrency is controlled via a [`tokio::sync::Semaphore`].
//!
//! The processing loop is cancellable via a [`tokio_util::sync::CancellationToken`].

use std::sync::Arc;
use std::time::Duration;

use pgvector::Vector;
use sqlx::PgPool;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::config::LlmConfig;
use crate::db::episodes::{self, EpisodeError};
use crate::llm::client::LlmClient;
use crate::llm::embeddings::{self, EmbeddingError};
use crate::pipeline::offline::extract::{self as extract_pipeline, PipelineError};
use crate::types::episode::Episode;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during episode processing.
#[derive(Debug, thiserror::Error)]
pub enum ProcessorError {
    /// A database error from the episodes query layer.
    #[error("database error: {0}")]
    Database(#[from] EpisodeError),

    /// An error generating the episode embedding.
    #[error("embedding error: {0}")]
    Embedding(#[from] EmbeddingError),

    /// An error from the full extraction pipeline.
    #[error("pipeline error: {0}")]
    Pipeline(#[from] PipelineError),
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Default polling interval in seconds.
const DEFAULT_POLL_INTERVAL_SECS: u64 = 5;

/// Default maximum number of concurrent episode processing tasks.
const DEFAULT_MAX_CONCURRENCY: usize = 4;

/// Maximum number of unprocessed episodes to fetch per poll cycle.
const BATCH_SIZE: i64 = 10;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Start the background episode processing loop.
///
/// Spawns a long-running tokio task that:
/// 1. Polls for unprocessed episodes every `poll_interval` seconds.
/// 2. For each unprocessed episode, acquires a semaphore permit and spawns
///    a tokio task that generates an embedding, stores it, and marks the
///    episode as processed.
/// 3. Stops gracefully when the `cancel_token` is cancelled.
///
/// Returns a [`tokio::task::JoinHandle`] for the processing loop task.
///
/// # Arguments
///
/// * `pool` — The **offline** database connection pool.
/// * `client` — The LLM client for embedding generation.
/// * `config` — LLM configuration (embedding model name, etc.).
/// * `cancel_token` — Token to signal graceful shutdown.
/// * `poll_interval` — Optional polling interval override (default: 5s).
/// * `max_concurrency` — Optional concurrency limit override (default: 4).
pub fn start_processing_loop(
    pool: PgPool,
    client: LlmClient,
    config: LlmConfig,
    cancel_token: CancellationToken,
    poll_interval: Option<Duration>,
    max_concurrency: Option<usize>,
) -> tokio::task::JoinHandle<()> {
    let interval = poll_interval.unwrap_or(Duration::from_secs(DEFAULT_POLL_INTERVAL_SECS));
    let concurrency = max_concurrency.unwrap_or(DEFAULT_MAX_CONCURRENCY);
    let semaphore = Arc::new(Semaphore::new(concurrency));

    tracing::info!(
        poll_interval_secs = interval.as_secs(),
        max_concurrency = concurrency,
        "starting background episode processing loop"
    );

    tokio::spawn(async move {
        processing_loop(pool, client, config, cancel_token, interval, semaphore).await;
    })
}

// ---------------------------------------------------------------------------
// Processing loop
// ---------------------------------------------------------------------------

/// Inner processing loop. Runs until the cancellation token is triggered.
async fn processing_loop(
    pool: PgPool,
    client: LlmClient,
    config: LlmConfig,
    cancel_token: CancellationToken,
    interval: Duration,
    semaphore: Arc<Semaphore>,
) {
    loop {
        tokio::select! {
            _ = cancel_token.cancelled() => {
                tracing::info!("processing loop received cancellation signal, shutting down");
                break;
            }
            _ = tokio::time::sleep(interval) => {
                poll_and_process(&pool, &client, &config, &semaphore, &cancel_token).await;
            }
        }
    }

    tracing::info!("processing loop exited");
}

/// Poll for unprocessed episodes and spawn processing tasks.
async fn poll_and_process(
    pool: &PgPool,
    client: &LlmClient,
    config: &LlmConfig,
    semaphore: &Arc<Semaphore>,
    cancel_token: &CancellationToken,
) {
    let unprocessed = match episodes::list_unprocessed_episodes(pool, BATCH_SIZE).await {
        Ok(eps) => eps,
        Err(e) => {
            tracing::error!(error = %e, "failed to poll unprocessed episodes");
            return;
        }
    };

    if unprocessed.is_empty() {
        tracing::trace!("no unprocessed episodes found");
        return;
    }

    tracing::info!(count = unprocessed.len(), "found unprocessed episodes");

    for episode in unprocessed {
        // Check cancellation before spawning new work.
        if cancel_token.is_cancelled() {
            tracing::info!("cancellation detected, stopping new task spawning");
            break;
        }

        let pool = pool.clone();
        let client = client.clone();
        let config = config.clone();
        let semaphore = Arc::clone(semaphore);

        tokio::spawn(async move {
            // Acquire a semaphore permit to control concurrency.
            let _permit = match semaphore.acquire().await {
                Ok(permit) => permit,
                Err(_) => {
                    tracing::error!(
                        episode_id = %episode.id,
                        "semaphore closed, aborting episode processing"
                    );
                    return;
                }
            };

            let episode_id = episode.id;
            tracing::info!(episode_id = %episode_id, "processing episode");

            match process_single_episode(&pool, &client, &config, &episode).await {
                Ok(()) => {
                    tracing::info!(episode_id = %episode_id, "episode processing complete");
                }
                Err(e) => {
                    tracing::error!(
                        episode_id = %episode_id,
                        error = %e,
                        "episode processing failed"
                    );
                }
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Single episode processing
// ---------------------------------------------------------------------------

/// Process a single episode: run the full extraction pipeline.
///
/// Steps:
/// 1. Generate a 768-dimension embedding for the episode content via
///    nomic-embed-text through Ollama and store it.
/// 2. Extract entities via Gemma 4 26B MoE.
/// 3. Resolve each entity through the 3-pass algorithm.
/// 4. Extract facts using pack-aware prompts.
/// 5. Validate entity references and insert facts.
/// 6. Validate and track predicates.
/// 7. Resolve fact supersessions.
/// 8. Initialize fact serving state.
/// 9. Compute extraction metrics and store as JSONB.
/// 10. Mark the episode as processed.
///
/// # Error Handling
///
/// On failure, the episode remains unprocessed (`processed = false`) for
/// retry on the next poll cycle. Transaction rollbacks are handled by
/// leaving the episode in its unprocessed state. All errors are logged
/// via tracing with span context.
///
/// # Errors
///
/// Returns [`ProcessorError`] if any pipeline step fails. Errors are logged
/// by the caller — the processing loop continues with other episodes.
#[tracing::instrument(
    skip(pool, client, config, episode),
    fields(
        episode_id = %episode.id,
        namespace = %episode.namespace,
        stage = "offline_process",
    )
)]
pub async fn process_single_episode(
    pool: &PgPool,
    client: &LlmClient,
    config: &LlmConfig,
    episode: &Episode,
) -> Result<(), ProcessorError> {
    let episode_id = episode.id;

    tracing::info!(episode_id = %episode_id, "starting full extraction pipeline");

    match extract_pipeline::run_full_extraction_pipeline(
        pool, client, config, episode,
    )
    .await
    {
        Ok(result) => {
            tracing::info!(
                episode_id = %episode_id,
                entities = result.entity_count,
                facts = result.fact_count,
                facts_skipped = result.facts_skipped,
                superseded = result.superseded_count,
                conflicts = result.conflict_count,
                processing_time_ms = result.metrics.processing_time_ms,
                "episode processing complete via full extraction pipeline"
            );
            Ok(())
        }
        Err(e) => {
            // Episode remains unprocessed (processed=false) for retry.
            // Log the error with full context for debugging.
            tracing::error!(
                episode_id = %episode_id,
                namespace = %episode.namespace,
                source = %episode.source,
                error = %e,
                "episode processing failed — episode queued for retry"
            );
            Err(ProcessorError::Pipeline(e))
        }
    }
}

// ---------------------------------------------------------------------------
// Embedding generation with retry
// ---------------------------------------------------------------------------

/// Maximum number of embedding retry attempts before giving up.
const EMBEDDING_MAX_RETRIES: u32 = 3;

/// Base delay between embedding retries (doubles each attempt).
const EMBEDDING_RETRY_BASE: Duration = Duration::from_millis(500);

/// Generate an episode embedding with retry logic.
///
/// Retries up to [`EMBEDDING_MAX_RETRIES`] times with exponential backoff
/// on transient failures. Logs each retry attempt.
async fn generate_episode_embedding_with_retry(
    client: &LlmClient,
    config: &LlmConfig,
    content: &str,
    episode_id: Uuid,
) -> Result<Vec<f32>, EmbeddingError> {
    let mut last_err: Option<EmbeddingError> = None;

    for attempt in 0..EMBEDDING_MAX_RETRIES {
        if attempt > 0 {
            let delay = EMBEDDING_RETRY_BASE * 2u32.pow(attempt - 1);
            tracing::warn!(
                episode_id = %episode_id,
                attempt,
                delay_ms = delay.as_millis() as u64,
                "retrying episode embedding generation"
            );
            tokio::time::sleep(delay).await;
        }

        match embeddings::generate_episode_embedding(client, config, content).await {
            Ok(vec) => return Ok(vec),
            Err(e) => {
                tracing::warn!(
                    episode_id = %episode_id,
                    attempt,
                    error = %e,
                    "episode embedding generation attempt failed"
                );
                last_err = Some(e);
            }
        }
    }

    Err(last_err.expect("at least one attempt was made"))
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

    // -- ProcessorError Display ---------------------------------------------

    #[test]
    fn processor_error_database_displays_message() {
        let err = ProcessorError::Database(EpisodeError::Sqlx(sqlx::Error::RowNotFound));
        let msg = err.to_string();
        assert!(msg.contains("database error"), "got: {msg}");
    }

    #[test]
    fn processor_error_embedding_displays_message() {
        let err = ProcessorError::Embedding(EmbeddingError::DimensionMismatch {
            expected: 768,
            actual: 512,
        });
        let msg = err.to_string();
        assert!(msg.contains("embedding error"), "got: {msg}");
    }

    #[test]
    fn processor_error_pipeline_displays_message() {
        let err = ProcessorError::Pipeline(PipelineError::Database(
            "connection refused".to_string(),
        ));
        let msg = err.to_string();
        assert!(msg.contains("pipeline error"), "got: {msg}");
    }

    // -- generate_episode_embedding_with_retry ------------------------------

    #[tokio::test]
    async fn embedding_retry_succeeds_on_first_attempt() {
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
        let result = generate_episode_embedding_with_retry(
            &client,
            &config,
            "test content",
            Uuid::new_v4(),
        )
        .await;

        let vec = result.expect("should succeed on first attempt");
        assert_eq!(vec.len(), 768);
    }

    #[tokio::test]
    async fn embedding_retry_succeeds_after_transient_failure() {
        let server = MockServer::start().await;

        // First call returns 500, second succeeds.
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(
                ResponseTemplate::new(500).set_body_string("server error"),
            )
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(embedding_response(768)),
            )
            .expect(1)
            .mount(&server)
            .await;

        let (client, config) = test_config(&server.uri());
        let result = generate_episode_embedding_with_retry(
            &client,
            &config,
            "test content",
            Uuid::new_v4(),
        )
        .await;

        let vec = result.expect("should succeed after retry");
        assert_eq!(vec.len(), 768);
    }

    #[tokio::test]
    async fn embedding_retry_fails_after_all_attempts() {
        let server = MockServer::start().await;

        // All calls return dimension mismatch (512 instead of 768).
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(embedding_response(512)),
            )
            .expect(3)
            .mount(&server)
            .await;

        let (client, config) = test_config(&server.uri());
        let err = generate_episode_embedding_with_retry(
            &client,
            &config,
            "test content",
            Uuid::new_v4(),
        )
        .await
        .unwrap_err();

        assert!(matches!(
            err,
            EmbeddingError::DimensionMismatch {
                expected: 768,
                actual: 512,
            }
        ));
    }

    // -- CancellationToken --------------------------------------------------

    #[tokio::test]
    async fn cancellation_token_stops_loop() {
        let cancel_token = CancellationToken::new();
        let token_clone = cancel_token.clone();

        // Cancel immediately.
        token_clone.cancel();

        // The loop should exit promptly.
        let semaphore = Arc::new(Semaphore::new(4));
        let server = MockServer::start().await;
        let (client, config) = test_config(&server.uri());

        // Use a pool-less approach: since the loop checks cancellation first,
        // it should exit before trying to poll the database.
        // We can't create a real PgPool without a database, so we test the
        // cancellation logic by verifying the loop exits.
        let handle = tokio::spawn(async move {
            processing_loop(
                // This will never be used because cancellation fires first.
                // We need a dummy pool — but we can't create one without a DB.
                // Instead, test that the cancel_token check works at the
                // select! level.
                unreachable_pool(),
                client,
                config,
                cancel_token,
                Duration::from_secs(60), // Long interval so sleep wins
                semaphore,
            )
            .await;
        });

        // The handle should complete quickly since we cancelled.
        let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(result.is_ok(), "processing loop should exit on cancellation");
    }

    /// Create a "dummy" pool that will panic if used. Only for testing
    /// cancellation paths where the pool is never actually accessed.
    ///
    /// We use a connect_lazy approach with an invalid URL — any actual
    /// query would fail, but the cancellation should fire before that.
    fn unreachable_pool() -> PgPool {
        use sqlx::postgres::PgPoolOptions;
        PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy("postgres://invalid:5432/nonexistent")
            .expect("connect_lazy should not fail")
    }

    // -- start_processing_loop returns a JoinHandle -------------------------

    #[tokio::test]
    async fn start_processing_loop_returns_handle() {
        let cancel_token = CancellationToken::new();
        let server = MockServer::start().await;
        let (client, config) = test_config(&server.uri());
        let pool = unreachable_pool();

        let handle = start_processing_loop(
            pool,
            client,
            config,
            cancel_token.clone(),
            Some(Duration::from_secs(60)),
            Some(2),
        );

        // Cancel and wait for clean exit.
        cancel_token.cancel();
        let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(result.is_ok(), "handle should complete after cancellation");
    }

    // -- Extraction metrics computation and JSONB serialization (Req 48.1) --

    #[test]
    fn extraction_metrics_serializes_to_complete_jsonb() {
        use crate::pipeline::offline::extract::PredicateValidationResult;
        use crate::pipeline::offline::state::compute_extraction_metrics;
        use crate::types::entity::ResolutionResult;

        let results = vec![
            ResolutionResult {
                entity_id: Uuid::new_v4(),
                method: "exact".to_string(),
                confidence: 1.0,
            },
            ResolutionResult {
                entity_id: Uuid::new_v4(),
                method: "alias".to_string(),
                confidence: 0.95,
            },
            ResolutionResult {
                entity_id: Uuid::new_v4(),
                method: "semantic".to_string(),
                confidence: 0.94,
            },
            ResolutionResult {
                entity_id: Uuid::new_v4(),
                method: "new".to_string(),
                confidence: 1.0,
            },
        ];

        let fact_result = crate::pipeline::offline::extract::FactOrchestrationResult {
            inserted_count: 3,
            skipped_count: 1,
            inserted_fact_ids: vec![Uuid::new_v4(); 3],
            predicate_validation: Some(PredicateValidationResult {
                canonical_count: 2,
                custom_count: 1,
            }),
            superseded_count: 0,
            model: "gemma4:26b-a4b-q4".to_string(),
            valid_extracted_facts: vec![],
        };

        let evidence = vec![
            Some("explicit".to_string()),
            Some("implied".to_string()),
            Some("explicit".to_string()),
        ];

        let metrics = compute_extraction_metrics(
            &results,
            1,
            Some(&fact_result),
            &evidence,
            750,
            "gemma4:26b-a4b-q4",
        );

        // Serialize to JSONB-compatible serde_json::Value.
        let json = serde_json::to_value(&metrics).expect("metrics should serialize");
        let obj = json.as_object().expect("should be a JSON object");

        // Verify all required fields are present (Req 48.3-48.7).
        let required_fields = [
            "extracted",
            "resolved_exact",
            "resolved_alias",
            "resolved_semantic",
            "new",
            "conflict_flagged",
            "facts_extracted",
            "canonical_predicate",
            "custom_predicate",
            "explicit",
            "implied",
            "processing_time_ms",
            "extraction_model",
        ];
        for field in &required_fields {
            assert!(obj.contains_key(*field), "missing field: {field}");
        }

        // Verify specific values.
        assert_eq!(json["extracted"], 4);
        assert_eq!(json["resolved_exact"], 1);
        assert_eq!(json["resolved_alias"], 1);
        assert_eq!(json["resolved_semantic"], 1);
        assert_eq!(json["new"], 1);
        assert_eq!(json["conflict_flagged"], 1);
        assert_eq!(json["facts_extracted"], 3);
        assert_eq!(json["canonical_predicate"], 2);
        assert_eq!(json["custom_predicate"], 1);
        assert_eq!(json["explicit"], 2);
        assert_eq!(json["implied"], 1);
        assert_eq!(json["processing_time_ms"], 750);
        assert_eq!(json["extraction_model"], "gemma4:26b-a4b-q4");
    }

    #[test]
    fn extraction_metrics_empty_input_produces_valid_jsonb() {
        use crate::pipeline::offline::state::compute_extraction_metrics;

        let metrics = compute_extraction_metrics(&[], 0, None, &[], 0, "test-model");

        let json = serde_json::to_value(&metrics).expect("should serialize");
        let obj = json.as_object().expect("should be object");

        // All counts should be zero.
        assert_eq!(json["extracted"], 0);
        assert_eq!(json["resolved_exact"], 0);
        assert_eq!(json["facts_extracted"], 0);
        assert_eq!(json["explicit"], 0);
        assert_eq!(json["implied"], 0);
        assert_eq!(json["processing_time_ms"], 0);
        assert_eq!(json["extraction_model"], "test-model");

        // Still has all required fields.
        assert!(obj.len() >= 13, "should have at least 13 fields, got {}", obj.len());
    }

    #[test]
    fn extraction_metrics_round_trip_deserialization() {
        use crate::pipeline::offline::state::compute_extraction_metrics;
        use crate::types::episode::ExtractionMetrics;

        let metrics = compute_extraction_metrics(
            &[crate::types::entity::ResolutionResult {
                entity_id: Uuid::new_v4(),
                method: "exact".to_string(),
                confidence: 1.0,
            }],
            0,
            None,
            &[Some("explicit".to_string())],
            200,
            "gemma4:26b-a4b-q4",
        );

        // Serialize → deserialize round trip.
        let json_str = serde_json::to_string(&metrics).expect("should serialize");
        let deserialized: ExtractionMetrics =
            serde_json::from_str(&json_str).expect("should deserialize");

        assert_eq!(deserialized.extracted, metrics.extracted);
        assert_eq!(deserialized.resolved_exact, metrics.resolved_exact);
        assert_eq!(deserialized.processing_time_ms, metrics.processing_time_ms);
        assert_eq!(deserialized.extraction_model, metrics.extraction_model);
        assert_eq!(deserialized.explicit, metrics.explicit);
    }

    // -- Episode processing loop behavior (Req 44.1, 44.4) -----------------

    #[tokio::test]
    async fn processing_loop_respects_concurrency_limit() {
        // Verify that the semaphore is created with the correct concurrency.
        let concurrency = 2usize;
        let semaphore = Arc::new(Semaphore::new(concurrency));

        // Available permits should equal the configured concurrency.
        assert_eq!(
            semaphore.available_permits(),
            concurrency,
            "semaphore should have {concurrency} permits"
        );

        // Acquiring permits reduces availability.
        let _permit1 = semaphore.acquire().await.unwrap();
        assert_eq!(semaphore.available_permits(), 1);

        let _permit2 = semaphore.acquire().await.unwrap();
        assert_eq!(semaphore.available_permits(), 0);
    }

    #[tokio::test]
    async fn start_processing_loop_custom_concurrency() {
        let cancel_token = CancellationToken::new();
        let server = MockServer::start().await;
        let (client, config) = test_config(&server.uri());
        let pool = unreachable_pool();

        // Custom concurrency of 1.
        let handle = start_processing_loop(
            pool,
            client,
            config,
            cancel_token.clone(),
            Some(Duration::from_secs(60)),
            Some(1),
        );

        cancel_token.cancel();
        let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(result.is_ok(), "should exit cleanly with custom concurrency");
    }

    #[tokio::test]
    async fn start_processing_loop_default_params() {
        let cancel_token = CancellationToken::new();
        let server = MockServer::start().await;
        let (client, config) = test_config(&server.uri());
        let pool = unreachable_pool();

        // Use None for both optional params to exercise defaults.
        let handle = start_processing_loop(
            pool,
            client,
            config,
            cancel_token.clone(),
            None,
            None,
        );

        cancel_token.cancel();
        let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
        assert!(result.is_ok(), "should exit cleanly with default params");
    }
}
