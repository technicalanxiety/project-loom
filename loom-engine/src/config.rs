//! Application and LLM configuration loaded from environment variables.

use serde::Deserialize;
use std::env;

/// Top-level application configuration.
///
/// Loaded once at startup via [`AppConfig::from_env`]. All secrets and
/// connection strings come from environment variables — nothing is
/// hard-coded.
#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    /// Fallback database URL used when online/offline URLs are not set.
    pub database_url: String,
    /// Database URL for the online (query-serving) connection pool.
    pub database_url_online: Option<String>,
    /// Database URL for the offline (episode-processing) connection pool.
    pub database_url_offline: Option<String>,
    /// Maximum connections for the online pool (default: 10).
    pub online_pool_max: u32,
    /// Maximum connections for the offline pool (default: 5).
    pub offline_pool_max: u32,
    /// Minimum connections for the online pool (default: 2).
    pub online_pool_min: u32,
    /// Minimum connections for the offline pool (default: 1).
    pub offline_pool_min: u32,
    /// Connection acquire timeout in seconds (default: 5).
    pub pool_acquire_timeout_secs: u64,
    /// Idle connection timeout in seconds (default: 300 = 5 minutes).
    pub pool_idle_timeout_secs: u64,
    /// SQL statement timeout in seconds (default: 30).
    pub statement_timeout_secs: u64,
    /// Hot tier cache TTL in seconds (default: 60).
    pub hot_tier_cache_ttl_secs: u64,
    /// Bind address for the HTTP server.
    pub loom_host: String,
    /// Port for the HTTP server.
    pub loom_port: u16,
    /// Bearer token for API authentication.
    pub loom_bearer_token: String,
    /// LLM / embedding service configuration.
    pub llm: LlmConfig,
}

/// Configuration for LLM inference services (Ollama primary, Azure OpenAI fallback).
#[derive(Debug, Clone, Deserialize)]
pub struct LlmConfig {
    /// Base URL for the Ollama API (e.g. `http://ollama:11434`).
    pub ollama_url: String,
    /// Model name for entity/fact extraction (e.g. `gemma4:26b`).
    pub extraction_model: String,
    /// Model name for intent classification (e.g. `gemma4:e4b`).
    pub classification_model: String,
    /// Model name for embedding generation (e.g. `nomic-embed-text`).
    pub embedding_model: String,
    /// Optional Azure OpenAI endpoint URL (fallback).
    pub azure_openai_url: Option<String>,
    /// Optional Azure OpenAI API key (fallback).
    pub azure_openai_key: Option<String>,
}

impl AppConfig {
    /// Load configuration from environment variables.
    ///
    /// # Required
    /// `DATABASE_URL`, `LOOM_BEARER_TOKEN`, `OLLAMA_URL`,
    /// `EXTRACTION_MODEL`, `CLASSIFICATION_MODEL`, `EMBEDDING_MODEL`
    ///
    /// # Optional
    /// `DATABASE_URL_ONLINE`, `DATABASE_URL_OFFLINE`,
    /// `ONLINE_POOL_MAX` (default 10), `OFFLINE_POOL_MAX` (default 5),
    /// `AZURE_OPENAI_URL`, `AZURE_OPENAI_KEY`,
    /// `LOOM_HOST` (default `0.0.0.0`), `LOOM_PORT` (default `8080`)
    pub fn from_env() -> Result<Self, String> {
        let database_url = env::var("DATABASE_URL")
            .map_err(|_| "DATABASE_URL must be set".to_string())?;

        let database_url_online = env::var("DATABASE_URL_ONLINE").ok();
        let database_url_offline = env::var("DATABASE_URL_OFFLINE").ok();

        let online_pool_max = env::var("ONLINE_POOL_MAX")
            .unwrap_or_else(|_| "10".to_string())
            .parse::<u32>()
            .map_err(|e| format!("ONLINE_POOL_MAX must be a valid u32: {e}"))?;

        let offline_pool_max = env::var("OFFLINE_POOL_MAX")
            .unwrap_or_else(|_| "5".to_string())
            .parse::<u32>()
            .map_err(|e| format!("OFFLINE_POOL_MAX must be a valid u32: {e}"))?;

        let online_pool_min = env::var("ONLINE_POOL_MIN")
            .unwrap_or_else(|_| "2".to_string())
            .parse::<u32>()
            .map_err(|e| format!("ONLINE_POOL_MIN must be a valid u32: {e}"))?;

        let offline_pool_min = env::var("OFFLINE_POOL_MIN")
            .unwrap_or_else(|_| "1".to_string())
            .parse::<u32>()
            .map_err(|e| format!("OFFLINE_POOL_MIN must be a valid u32: {e}"))?;

        let pool_acquire_timeout_secs = env::var("POOL_ACQUIRE_TIMEOUT_SECS")
            .unwrap_or_else(|_| "5".to_string())
            .parse::<u64>()
            .map_err(|e| format!("POOL_ACQUIRE_TIMEOUT_SECS must be a valid u64: {e}"))?;

        let pool_idle_timeout_secs = env::var("POOL_IDLE_TIMEOUT_SECS")
            .unwrap_or_else(|_| "300".to_string())
            .parse::<u64>()
            .map_err(|e| format!("POOL_IDLE_TIMEOUT_SECS must be a valid u64: {e}"))?;

        let statement_timeout_secs = env::var("STATEMENT_TIMEOUT_SECS")
            .unwrap_or_else(|_| "30".to_string())
            .parse::<u64>()
            .map_err(|e| format!("STATEMENT_TIMEOUT_SECS must be a valid u64: {e}"))?;

        let hot_tier_cache_ttl_secs = env::var("HOT_TIER_CACHE_TTL_SECS")
            .unwrap_or_else(|_| "60".to_string())
            .parse::<u64>()
            .map_err(|e| format!("HOT_TIER_CACHE_TTL_SECS must be a valid u64: {e}"))?;

        let loom_host = env::var("LOOM_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        let loom_port = env::var("LOOM_PORT")
            .unwrap_or_else(|_| "8080".to_string())
            .parse::<u16>()
            .map_err(|e| format!("LOOM_PORT must be a valid u16: {e}"))?;

        let loom_bearer_token = env::var("LOOM_BEARER_TOKEN")
            .map_err(|_| "LOOM_BEARER_TOKEN must be set".to_string())?;

        let ollama_url = env::var("OLLAMA_URL")
            .map_err(|_| "OLLAMA_URL must be set".to_string())?;
        let extraction_model = env::var("EXTRACTION_MODEL")
            .map_err(|_| "EXTRACTION_MODEL must be set".to_string())?;
        let classification_model = env::var("CLASSIFICATION_MODEL")
            .map_err(|_| "CLASSIFICATION_MODEL must be set".to_string())?;
        let embedding_model = env::var("EMBEDDING_MODEL")
            .map_err(|_| "EMBEDDING_MODEL must be set".to_string())?;

        let azure_openai_url = env::var("AZURE_OPENAI_URL")
            .ok()
            .filter(|s| !s.is_empty());
        let azure_openai_key = env::var("AZURE_OPENAI_KEY")
            .ok()
            .filter(|s| !s.is_empty());

        Ok(Self {
            database_url,
            database_url_online,
            database_url_offline,
            online_pool_max,
            offline_pool_max,
            online_pool_min,
            offline_pool_min,
            pool_acquire_timeout_secs,
            pool_idle_timeout_secs,
            statement_timeout_secs,
            hot_tier_cache_ttl_secs,
            loom_host,
            loom_port,
            loom_bearer_token,
            llm: LlmConfig {
                ollama_url,
                extraction_model,
                classification_model,
                embedding_model,
                azure_openai_url,
                azure_openai_key,
            },
        })
    }
}
