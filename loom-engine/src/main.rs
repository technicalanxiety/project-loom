use axum::{extract::DefaultBodyLimit, middleware, routing::{get, post}, Router};
use std::net::SocketAddr;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

// Maximum request body size for ingestion endpoints. Raw Claude Code
// session JSONL files regularly exceed axum's 2 MiB default (one of my
// own is 3.5 MiB), and the design decision is one-episode-per-session
// rather than chunking, so the limit has to accommodate a full session.
// 64 MiB is comfortable headroom for the largest transcripts I have
// observed while still refusing pathological uploads.
const INGEST_BODY_LIMIT_BYTES: usize = 64 * 1024 * 1024;

mod api;
mod config;
mod crypto;
mod db;
mod llm;
mod pipeline;
mod telemetry;
mod types;
mod worker;

use api::auth::require_bearer_token;
use api::dashboard;
use api::mcp::{handle_loom_learn, handle_loom_recall, handle_loom_think, AppState};
use api::mcp_rpc::handle_mcp_rpc;
use api::rest::{handle_api_learn, handle_github_webhook, handle_health};
use worker::{processor, scheduler};

#[tokio::main]
async fn main() {
    // Install ring as the default rustls crypto provider (avoids aws-lc-sys
    // which cannot cross-compile to musl). Idempotent via Once — also called
    // from LlmClient::new so test harnesses don't need to do it themselves.
    crypto::ensure_crypto_provider();

    // Load .env before tracing init so RUST_LOG is available.
    dotenvy::dotenv().ok();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "loom_engine=info,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = match config::AppConfig::from_env() {
        Ok(cfg) => {
            tracing::info!(host = %cfg.loom_host, port = cfg.loom_port, "configuration loaded");
            cfg
        }
        Err(e) => {
            tracing::error!("failed to load configuration: {e}");
            std::process::exit(1);
        }
    };

    // Initialize database pools and run migrations.
    let pools = match db::pool::DbPools::init(&config).await {
        Ok(p) => {
            tracing::info!("database pools initialized");
            p
        }
        Err(e) => {
            tracing::error!("failed to initialize database pools: {e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = pools.run_migrations().await {
        tracing::error!("failed to run migrations: {e}");
        std::process::exit(1);
    }

    // Build LLM client.
    let llm_client = match llm::client::LlmClient::new(&config.llm) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("failed to build LLM client: {e}");
            std::process::exit(1);
        }
    };

    let bearer_token = config.loom_bearer_token.clone();

    let telemetry_state = telemetry::new_shared();

    let state = AppState {
        pools,
        llm_client,
        config,
        telemetry: telemetry_state.clone(),
    };

    // MCP routes — all protected by bearer token middleware.
    //
    // - `POST /mcp` is the JSON-RPC 2.0 dispatcher used by every real MCP
    //   client (Claude Desktop, ChatGPT Desktop, GitHub Copilot, M365 Copilot,
    //   Claude Code, and the mcp-remote stdio bridge).
    // - The per-tool REST endpoints (`/mcp/loom_learn` etc.) remain mounted
    //   for direct curl testing, integration tests, and callers that predate
    //   the dispatcher.
    let mcp_routes = Router::new()
        .route("/mcp", post(handle_mcp_rpc))
        .route("/mcp/loom_learn", post(handle_loom_learn))
        .route("/mcp/loom_think", post(handle_loom_think))
        .route("/mcp/loom_recall", post(handle_loom_recall))
        .layer(DefaultBodyLimit::max(INGEST_BODY_LIMIT_BYTES))
        .layer(middleware::from_fn_with_state(
            bearer_token.clone(),
            require_bearer_token,
        ))
        .with_state(state.clone());

    // REST /api/learn — protected by bearer token middleware.
    // Raised body limit so bootstrap parsers can POST full session
    // transcripts (Claude Code JSONL, Claude.ai export, Purview audit
    // bundles) as single episodes without chunking.
    let rest_learn_route = Router::new()
        .route("/api/learn", post(handle_api_learn))
        .layer(DefaultBodyLimit::max(INGEST_BODY_LIMIT_BYTES))
        .layer(middleware::from_fn_with_state(
            bearer_token.clone(),
            require_bearer_token,
        ))
        .with_state(state.clone());

    // GitHub webhook — protected by bearer token middleware.
    let webhook_routes = Router::new()
        .route("/api/webhooks/github", post(handle_github_webhook))
        .layer(middleware::from_fn_with_state(
            bearer_token.clone(),
            require_bearer_token,
        ))
        .with_state(state.clone());

    // Unauthenticated routes — health check and legacy stub.
    let public_routes = Router::new()
        .route("/api/health", get(handle_health))
        .with_state(state.clone());

    // Dashboard routes — all protected by bearer token middleware.
    let dashboard_routes = Router::new()
        .route(
            "/dashboard/api/health",
            get(dashboard::handle_dashboard_health),
        )
        .route(
            "/dashboard/api/namespaces",
            get(dashboard::handle_namespaces),
        )
        .route(
            "/dashboard/api/compilations",
            get(dashboard::handle_compilations),
        )
        .route(
            "/dashboard/api/compilations/{id}",
            get(dashboard::handle_compilation_detail),
        )
        .route("/dashboard/api/entities", get(dashboard::handle_entities))
        .route(
            "/dashboard/api/entities/{id}",
            get(dashboard::handle_entity_detail),
        )
        .route(
            "/dashboard/api/entities/{id}/graph",
            get(dashboard::handle_entity_graph),
        )
        .route("/dashboard/api/facts", get(dashboard::handle_facts))
        .route("/dashboard/api/conflicts", get(dashboard::handle_conflicts))
        .route(
            "/dashboard/api/predicates/candidates",
            get(dashboard::handle_predicate_candidates),
        )
        .route(
            "/dashboard/api/predicates/packs",
            get(dashboard::handle_predicate_packs),
        )
        .route(
            "/dashboard/api/predicates/packs/{pack}",
            get(dashboard::handle_pack_detail),
        )
        .route(
            "/dashboard/api/predicates/active/{namespace}",
            get(dashboard::handle_active_predicates),
        )
        .route(
            "/dashboard/api/metrics/retrieval",
            get(dashboard::handle_metrics_retrieval),
        )
        .route(
            "/dashboard/api/metrics/extraction",
            get(dashboard::handle_metrics_extraction),
        )
        .route(
            "/dashboard/api/metrics/classification",
            get(dashboard::handle_metrics_classification),
        )
        .route(
            "/dashboard/api/metrics/hot-tier",
            get(dashboard::handle_metrics_hot_tier),
        )
        .route(
            "/dashboard/api/metrics/parser-health",
            get(dashboard::handle_metrics_parser_health),
        )
        .route(
            "/dashboard/api/metrics/ingestion-distribution",
            get(dashboard::handle_metrics_ingestion_distribution),
        )
        .route(
            "/dashboard/api/conflicts/{id}/resolve",
            post(dashboard::handle_resolve_conflict),
        )
        .route(
            "/dashboard/api/predicates/candidates/{id}/resolve",
            post(dashboard::handle_resolve_predicate_candidate),
        )
        .route(
            "/dashboard/api/episodes/failed",
            get(dashboard::handle_failed_episodes),
        )
        .route(
            "/dashboard/api/episodes/failed/requeue-all",
            post(dashboard::handle_requeue_all_failed),
        )
        .route(
            "/dashboard/api/episodes/{id}/requeue",
            post(dashboard::handle_requeue_episode),
        )
        .route(
            "/dashboard/api/benchmarks",
            get(dashboard::handle_benchmark_runs),
        )
        .route(
            "/dashboard/api/benchmarks/run",
            post(dashboard::handle_run_benchmark),
        )
        .route(
            "/dashboard/api/benchmarks/seed",
            post(dashboard::handle_seed_benchmark),
        )
        .route(
            "/dashboard/api/benchmarks/{id}",
            get(dashboard::handle_benchmark_detail),
        )
        .route(
            "/dashboard/api/stream/telemetry",
            get(dashboard::handle_telemetry_stream),
        )
        .layer(middleware::from_fn_with_state(
            bearer_token.clone(),
            require_bearer_token,
        ))
        .with_state(state.clone());

    let app = Router::new()
        .merge(mcp_routes)
        .merge(rest_learn_route)
        .merge(webhook_routes)
        .merge(public_routes)
        .merge(dashboard_routes);

    // ── Background workers ──────────────────────────────────────────
    // The offline processor polls for unprocessed episodes and runs
    // the full extraction pipeline against each. The scheduler runs
    // daily hot-tier snapshots, daily tier management, and a weekly
    // entity-health-check. Both are spawned before axum::serve so the
    // runtime has them registered; both respect the same cancellation
    // token so a graceful shutdown stops everything in order.
    let cancel_token = CancellationToken::new();

    let retry_policy = processor::RetryPolicy::new(
        state.config.episode_max_attempts,
        state.config.episode_backoff_base_secs,
    );

    let _processor_handle = processor::start_processing_loop(
        state.pools.offline.clone(),
        state.llm_client.clone(),
        state.config.llm.clone(),
        retry_policy,
        cancel_token.clone(),
        None, // default poll interval (5s)
        Some(state.config.worker_concurrency),
    );

    let _sampler_task = tokio::spawn(telemetry::sampler::run_sampler(
        telemetry_state,
        state.pools.clone(),
        state.config.llm.ollama_url.clone(),
        cancel_token.clone(),
    ));

    let _scheduler_handles = scheduler::start_scheduler(
        state.pools.offline.clone(),
        cancel_token.clone(),
    );

    let addr = SocketAddr::from(([0, 0, 0, 0], state.config.loom_port));
    tracing::info!("loom-engine listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();

    // Graceful shutdown: on Ctrl-C, cancel the background workers and
    // let axum drain in-flight requests before exiting.
    let shutdown_token = cancel_token.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("shutdown signal received, cancelling background workers");
            shutdown_token.cancel();
        })
        .await
        .unwrap();
}
