//! Background sampler — writes host / Ollama / DB state into `TelemetryState`.
//!
//! The sampler ticks every second. Host CPU + memory are refreshed every tick;
//! Ollama `/api/ps` and the DB snapshot query fire every 5 ticks (each is
//! comparatively expensive). Results are written into a shared `Arc<RwLock<_>>`
//! that the SSE handler reads.
//!
//! Error handling: the sampler never panics. If Ollama or the DB is unreachable
//! on a given tick, the last-known values remain in state and a single
//! `tracing::warn!` is emitted on the healthy→error transition (and a
//! `tracing::info!` on recovery) — steady-state failure does not spam the logs.

use std::time::Duration;

use sysinfo::System;
use tokio_util::sync::CancellationToken;

use crate::db::pool::DbPools;
use crate::telemetry::state::{ExtractionError, SharedTelemetry, TelemetryState};

/// How often the sampler wakes up. Host resources refresh every tick.
const TICK: Duration = Duration::from_secs(1);

/// Tick multiplier for the slow paths (Ollama + DB). `5` → every 5 seconds.
const SLOW_PATH_EVERY: u64 = 5;

/// Timeout for each Ollama `/api/ps` request.
const OLLAMA_TIMEOUT: Duration = Duration::from_secs(2);

/// Subset of Ollama's `/api/ps` response — only the fields we render.
#[derive(Debug, serde::Deserialize)]
struct OllamaPsResponse {
    models: Vec<OllamaModel>,
}

#[derive(Debug, serde::Deserialize)]
struct OllamaModel {
    name: String,
    #[serde(default)]
    size_vram: u64,
}

/// Run the sampler until `cancel` is triggered.
pub async fn run_sampler(
    telemetry: SharedTelemetry,
    pools: DbPools,
    ollama_base_url: String,
    cancel: CancellationToken,
) {
    tracing::info!("telemetry sampler starting");

    let mut sys = System::new_all();
    // Prime the CPU sample — sysinfo requires two refreshes separated by
    // ~200 ms before the first reading is meaningful.
    sys.refresh_cpu_all();
    tokio::time::sleep(Duration::from_millis(250)).await;

    let http = reqwest::Client::new();
    let mut tick_count: u64 = 0;
    let mut interval = tokio::time::interval(TICK);
    // Burst of missed ticks (paused task, blocked DB) should not be replayed.
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let mut ollama_healthy = true;
    let mut db_healthy = true;

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!("telemetry sampler shutting down");
                break;
            }
            _ = interval.tick() => {}
        }

        tick_count += 1;
        let now_ms = chrono::Utc::now().timestamp_millis();

        // Host resources every tick.
        sys.refresh_cpu_all();
        sys.refresh_memory();
        let cpu_pct = sys.global_cpu_usage();
        let mem_used_mib = sys.used_memory() / 1024 / 1024;
        let mem_total_mib = sys.total_memory() / 1024 / 1024;

        let slow_path = tick_count.is_multiple_of(SLOW_PATH_EVERY);

        let ollama_snapshot = if slow_path {
            match poll_ollama(&http, &ollama_base_url).await {
                Ok(snap) => {
                    if !ollama_healthy {
                        tracing::info!("telemetry: ollama reachable again");
                        ollama_healthy = true;
                    }
                    Some(snap)
                }
                Err(e) => {
                    if ollama_healthy {
                        tracing::warn!(error = %e, "telemetry: ollama /api/ps poll failed");
                        ollama_healthy = false;
                    }
                    None
                }
            }
        } else {
            None
        };

        let db_snapshot = if slow_path {
            match query_db_snapshot(&pools).await {
                Ok(snap) => {
                    if !db_healthy {
                        tracing::info!("telemetry: db snapshot queries recovered");
                        db_healthy = true;
                    }
                    Some(snap)
                }
                Err(e) => {
                    if db_healthy {
                        tracing::warn!(error = %e, "telemetry: db snapshot query failed");
                        db_healthy = false;
                    }
                    None
                }
            }
        } else {
            None
        };

        // Brief write — slow I/O above is already complete.
        let mut state = telemetry.write().await;

        state.cpu_pct = cpu_pct;
        state.mem_used_mib = mem_used_mib;
        state.mem_total_mib = mem_total_mib;

        if let Some(ollama) = ollama_snapshot {
            state.ollama_model = ollama.model;
            state.ollama_on_gpu = ollama.on_gpu;
            state.ollama_vram_mib = ollama.vram_mib;
        }

        if let Some(db) = db_snapshot {
            state.active_ingestions = db.active_ingestions;
            state.queue_depth = db.queue_depth;
            state.failed_episodes = db.failed_episodes;
            state.latency_classify_p50_ms = db.latency_classify_p50_ms;
            state.latency_retrieve_p50_ms = db.latency_retrieve_p50_ms;
            state.latency_rank_p50_ms = db.latency_rank_p50_ms;
            state.latency_compile_p50_ms = db.latency_compile_p50_ms;
            state.latency_total_p50_ms = db.latency_total_p50_ms;

            if let Some(lat) = db.latency_total_p50_ms {
                TelemetryState::push_sparkline(&mut state.sparkline_latency, now_ms, lat);
            }
            TelemetryState::push_sparkline(
                &mut state.sparkline_ingestion_rate,
                now_ms,
                db.ingestion_completions_last_min as f64,
            );
            TelemetryState::push_sparkline(
                &mut state.sparkline_compilation_rate,
                now_ms,
                db.compilation_requests_last_min as f64,
            );

            for err in db.recent_errors {
                TelemetryState::push_error(&mut state.recent_errors, err);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Ollama poll
// ---------------------------------------------------------------------------

struct OllamaSnapshot {
    model: Option<String>,
    on_gpu: bool,
    vram_mib: Option<u64>,
}

async fn poll_ollama(http: &reqwest::Client, base_url: &str) -> Result<OllamaSnapshot, String> {
    let url = format!("{}/api/ps", base_url.trim_end_matches('/'));
    let resp = http
        .get(&url)
        .timeout(OLLAMA_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("request: {e}"))?;

    if !resp.status().is_success() {
        // 5xx / 404 — treat as "no model loaded" without flagging the transport
        // as broken. The server is reachable, it just has nothing to report.
        return Ok(OllamaSnapshot {
            model: None,
            on_gpu: false,
            vram_mib: None,
        });
    }

    let ps: OllamaPsResponse = resp.json().await.map_err(|e| format!("decode: {e}"))?;

    if ps.models.is_empty() {
        return Ok(OllamaSnapshot {
            model: None,
            on_gpu: false,
            vram_mib: None,
        });
    }

    // Loom only keeps one model resident at a time; take the first entry.
    let m = &ps.models[0];
    let on_gpu = m.size_vram > 0;
    let vram_mib = if on_gpu {
        Some(m.size_vram / 1024 / 1024)
    } else {
        None
    };

    Ok(OllamaSnapshot {
        model: Some(m.name.clone()),
        on_gpu,
        vram_mib,
    })
}

// ---------------------------------------------------------------------------
// DB snapshot
// ---------------------------------------------------------------------------

struct DbSnapshot {
    active_ingestions: i64,
    queue_depth: i64,
    failed_episodes: i64,
    ingestion_completions_last_min: i64,
    compilation_requests_last_min: i64,
    latency_classify_p50_ms: Option<f64>,
    latency_retrieve_p50_ms: Option<f64>,
    latency_rank_p50_ms: Option<f64>,
    latency_compile_p50_ms: Option<f64>,
    latency_total_p50_ms: Option<f64>,
    recent_errors: Vec<ExtractionError>,
}

async fn query_db_snapshot(pools: &DbPools) -> Result<DbSnapshot, sqlx::Error> {
    let pool = &pools.online;

    let (active, pending, failed): (i64, i64, i64) = sqlx::query_as(
        r#"
        SELECT
          COUNT(*) FILTER (WHERE processing_status = 'processing') AS active,
          COUNT(*) FILTER (WHERE processing_status = 'pending')    AS pending,
          COUNT(*) FILTER (WHERE processing_status = 'failed')     AS failed
        FROM loom_episodes
        WHERE deleted_at IS NULL
        "#,
    )
    .fetch_one(pool)
    .await?;

    let completions: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) FROM loom_episodes
        WHERE processing_status = 'completed'
          AND processing_last_attempt > NOW() - INTERVAL '60 seconds'
          AND deleted_at IS NULL
        "#,
    )
    .fetch_one(pool)
    .await?;

    // loom_audit_log is written exclusively by loom_think — one row per
    // compilation — so a row count in the last 60 s is exactly
    // "compilations per minute".
    let compilations: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM loom_audit_log WHERE created_at > NOW() - INTERVAL '60 seconds'",
    )
    .fetch_one(pool)
    .await?;

    #[derive(sqlx::FromRow)]
    struct LatencyRow {
        p50_classify: Option<f64>,
        p50_retrieve: Option<f64>,
        p50_rank: Option<f64>,
        p50_compile: Option<f64>,
        p50_total: Option<f64>,
    }

    let latency: Option<LatencyRow> = sqlx::query_as(
        r#"
        SELECT
          PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY latency_classify_ms) AS p50_classify,
          PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY latency_retrieve_ms) AS p50_retrieve,
          PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY latency_rank_ms)     AS p50_rank,
          PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY latency_compile_ms)  AS p50_compile,
          PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY latency_total_ms)    AS p50_total
        FROM (
          SELECT latency_classify_ms, latency_retrieve_ms, latency_rank_ms,
                 latency_compile_ms, latency_total_ms
          FROM loom_audit_log
          WHERE latency_total_ms IS NOT NULL
          ORDER BY created_at DESC
          LIMIT 60
        ) recent
        "#,
    )
    .fetch_optional(pool)
    .await?;

    #[derive(sqlx::FromRow)]
    struct ErrorRow {
        id: uuid::Uuid,
        source: String,
        processing_last_error: Option<String>,
        processing_last_attempt: Option<chrono::DateTime<chrono::Utc>>,
    }

    let error_rows: Vec<ErrorRow> = sqlx::query_as(
        r#"
        SELECT id, source, processing_last_error, processing_last_attempt
        FROM loom_episodes
        WHERE processing_status = 'failed' AND deleted_at IS NULL
        ORDER BY processing_last_attempt DESC NULLS LAST
        LIMIT 10
        "#,
    )
    .fetch_all(pool)
    .await?;

    let recent_errors = error_rows
        .into_iter()
        .map(|r| ExtractionError {
            episode_id: r.id.to_string(),
            source: r.source,
            error: r.processing_last_error.unwrap_or_else(|| "unknown".into()),
            occurred_at: r
                .processing_last_attempt
                .map(|t| t.timestamp_millis())
                .unwrap_or(0),
        })
        .collect();

    Ok(DbSnapshot {
        active_ingestions: active,
        queue_depth: pending,
        failed_episodes: failed,
        ingestion_completions_last_min: completions,
        compilation_requests_last_min: compilations,
        latency_classify_p50_ms: latency.as_ref().and_then(|l| l.p50_classify),
        latency_retrieve_p50_ms: latency.as_ref().and_then(|l| l.p50_retrieve),
        latency_rank_p50_ms: latency.as_ref().and_then(|l| l.p50_rank),
        latency_compile_p50_ms: latency.as_ref().and_then(|l| l.p50_compile),
        latency_total_p50_ms: latency.as_ref().and_then(|l| l.p50_total),
        recent_errors,
    })
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

    /// reqwest with `rustls-no-provider` panics on first TLS use unless a
    /// provider is installed. Install once per test module.
    fn setup() {
        crate::crypto::ensure_crypto_provider();
    }

    #[tokio::test]
    async fn poll_ollama_maps_loaded_gpu_model() {
        setup();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/ps"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "models": [{
                    "name": "gemma4:26b",
                    // 8 GiB reported — non-zero means on GPU
                    "size_vram": 8_u64 * 1024 * 1024 * 1024,
                }]
            })))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let snap = poll_ollama(&http, &server.uri()).await.expect("ok");

        assert_eq!(snap.model.as_deref(), Some("gemma4:26b"));
        assert!(snap.on_gpu);
        assert_eq!(snap.vram_mib, Some(8192));
    }

    #[tokio::test]
    async fn poll_ollama_reports_cpu_when_size_vram_zero() {
        setup();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/ps"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "models": [{ "name": "qwen2.5:14b", "size_vram": 0 }]
            })))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let snap = poll_ollama(&http, &server.uri()).await.expect("ok");

        assert_eq!(snap.model.as_deref(), Some("qwen2.5:14b"));
        assert!(!snap.on_gpu);
        assert_eq!(snap.vram_mib, None);
    }

    #[tokio::test]
    async fn poll_ollama_empty_models_means_no_model_loaded() {
        setup();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/ps"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "models": [] })))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let snap = poll_ollama(&http, &server.uri()).await.expect("ok");

        assert!(snap.model.is_none());
        assert!(!snap.on_gpu);
        assert!(snap.vram_mib.is_none());
    }

    #[tokio::test]
    async fn poll_ollama_non_2xx_returns_no_model_loaded() {
        // Ollama reachable but returning an error body — treat as
        // "no model loaded" rather than flagging the transport as broken.
        setup();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/ps"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let http = reqwest::Client::new();
        let snap = poll_ollama(&http, &server.uri()).await.expect("ok");

        assert!(snap.model.is_none());
        assert!(!snap.on_gpu);
    }

    #[tokio::test]
    async fn poll_ollama_connection_error_surfaces_as_err() {
        // A port that cannot accept connections → request layer error.
        // This is the path that triggers the healthy→unhealthy log
        // transition in run_sampler.
        setup();
        let http = reqwest::Client::new();
        let res = poll_ollama(&http, "http://127.0.0.1:1").await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn poll_ollama_trims_trailing_slash_from_base_url() {
        setup();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/ps"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "models": [] })))
            .mount(&server)
            .await;

        // Pass the base URL with a trailing slash — must not produce //api/ps.
        let http = reqwest::Client::new();
        let url_with_slash = format!("{}/", server.uri());
        poll_ollama(&http, &url_with_slash).await.expect("ok");
    }
}
