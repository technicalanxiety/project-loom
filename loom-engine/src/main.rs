use axum::{middleware, routing::{get, post}, Router};
use std::net::SocketAddr;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod api;
mod config;
mod db;
mod llm;
mod pipeline;
mod types;
mod worker;

use api::auth::require_bearer_token;
use api::dashboard;
use api::mcp::{handle_loom_learn, handle_loom_recall, handle_loom_think, AppState};
use api::rest::{handle_api_learn, handle_github_webhook, handle_health};

#[tokio::main]
async fn main() {
    // Install ring as the default rustls crypto provider (avoids aws-lc-sys
    // which cannot cross-compile to musl). Idempotent via Once — also called
    // from LlmClient::new so test harnesses don't need to do it themselves.
    loom_engine::ensure_crypto_provider();

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

    let state = AppState {
        pools,
        llm_client,
        config,
    };

    // MCP routes — all protected by bearer token middleware.
    let mcp_routes = Router::new()
        .route("/mcp/loom_learn", post(handle_loom_learn))
        .route("/mcp/loom_think", post(handle_loom_think))
        .route("/mcp/loom_recall", post(handle_loom_recall))
        .layer(middleware::from_fn_with_state(
            bearer_token.clone(),
            require_bearer_token,
        ))
        .with_state(state.clone());

    // REST /api/learn — protected by bearer token middleware.
    let rest_learn_route = Router::new()
        .route("/api/learn", post(handle_api_learn))
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
        .route("/dashboard/api/health", get(dashboard::handle_dashboard_health))
        .route("/dashboard/api/namespaces", get(dashboard::handle_namespaces))
        .route("/dashboard/api/compilations", get(dashboard::handle_compilations))
        .route("/dashboard/api/compilations/:id", get(dashboard::handle_compilation_detail))
        .route("/dashboard/api/entities", get(dashboard::handle_entities))
        .route("/dashboard/api/entities/:id", get(dashboard::handle_entity_detail))
        .route("/dashboard/api/entities/:id/graph", get(dashboard::handle_entity_graph))
        .route("/dashboard/api/facts", get(dashboard::handle_facts))
        .route("/dashboard/api/conflicts", get(dashboard::handle_conflicts))
        .route("/dashboard/api/predicates/candidates", get(dashboard::handle_predicate_candidates))
        .route("/dashboard/api/predicates/packs", get(dashboard::handle_predicate_packs))
        .route("/dashboard/api/predicates/packs/:pack", get(dashboard::handle_pack_detail))
        .route("/dashboard/api/predicates/active/:namespace", get(dashboard::handle_active_predicates))
        .route("/dashboard/api/metrics/retrieval", get(dashboard::handle_metrics_retrieval))
        .route("/dashboard/api/metrics/extraction", get(dashboard::handle_metrics_extraction))
        .route("/dashboard/api/metrics/classification", get(dashboard::handle_metrics_classification))
        .route("/dashboard/api/metrics/hot-tier", get(dashboard::handle_metrics_hot_tier))
        .route(
            "/dashboard/api/metrics/parser-health",
            get(dashboard::handle_metrics_parser_health),
        )
        .route(
            "/dashboard/api/metrics/ingestion-distribution",
            get(dashboard::handle_metrics_ingestion_distribution),
        )
        .route("/dashboard/api/conflicts/:id/resolve", post(dashboard::handle_resolve_conflict))
        .route("/dashboard/api/predicates/candidates/:id/resolve", post(dashboard::handle_resolve_predicate_candidate))
        .route("/dashboard/api/benchmarks", get(dashboard::handle_benchmark_runs))
        .route("/dashboard/api/benchmarks/run", post(dashboard::handle_run_benchmark))
        .route("/dashboard/api/benchmarks/:id", get(dashboard::handle_benchmark_detail))
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

    let addr = SocketAddr::from(([0, 0, 0, 0], state.config.loom_port));
    tracing::info!("loom-engine listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
