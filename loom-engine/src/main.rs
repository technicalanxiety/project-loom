use axum::{routing::get, Router};
use std::net::SocketAddr;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod api;
mod config;
mod db;
mod llm;
mod pipeline;
mod types;
mod worker;

#[tokio::main]
async fn main() {
    // Load .env before tracing init so RUST_LOG is available
    dotenvy::dotenv().ok();

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "loom_engine=info,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let _config = match config::AppConfig::from_env() {
        Ok(cfg) => {
            tracing::info!(host = %cfg.loom_host, port = cfg.loom_port, "configuration loaded");
            cfg
        }
        Err(e) => {
            tracing::warn!("config not fully loaded (ok during scaffolding): {e}");
            // During scaffolding, allow startup without full config
            config::AppConfig::from_env().unwrap_or_else(|_| {
                // Provide minimal defaults for scaffolding phase
                config::AppConfig {
                    database_url: String::new(),
                    database_url_online: None,
                    database_url_offline: None,
                    loom_host: "0.0.0.0".to_string(),
                    loom_port: 8080,
                    loom_bearer_token: String::new(),
                    llm: config::LlmConfig {
                        ollama_url: String::new(),
                        extraction_model: String::new(),
                        classification_model: String::new(),
                        embedding_model: String::new(),
                        azure_openai_url: None,
                        azure_openai_key: None,
                    },
                }
            })
        }
    };

    let app = Router::new().route("/api/health", get(health_check));

    let addr = SocketAddr::from(([0, 0, 0, 0], _config.loom_port));
    tracing::info!("loom-engine listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn health_check() -> &'static str {
    "ok"
}
