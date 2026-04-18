// loom-engine/src/config.rs
// Application and LLM configuration loaded from environment variables.

use serde::Deserialize;
use std::env;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub database_url: String,
    pub database_url_online: Option<String>,
    pub database_url_offline: Option<String>,
    pub loom_host: String,
    pub loom_port: u16,
    pub loom_bearer_token: String,
    pub llm: LlmConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmConfig {
    pub ollama_url: String,
    pub extraction_model: String,
    pub classification_model: String,
    pub embedding_model: String,
    pub azure_openai_url: Option<String>,
    pub azure_openai_key: Option<String>,
}

impl AppConfig {
    /// Load configuration from environment variables.
    ///
    /// Required: DATABASE_URL, LOOM_BEARER_TOKEN, OLLAMA_URL,
    ///           EXTRACTION_MODEL, CLASSIFICATION_MODEL, EMBEDDING_MODEL
    ///
    /// Optional: DATABASE_URL_ONLINE, DATABASE_URL_OFFLINE,
    ///           AZURE_OPENAI_URL, AZURE_OPENAI_KEY,
    ///           LOOM_HOST (default "0.0.0.0"), LOOM_PORT (default 8080)
    pub fn from_env() -> Result<Self, String> {
        let database_url = env::var("DATABASE_URL")
            .map_err(|_| "DATABASE_URL must be set".to_string())?;

        let database_url_online = env::var("DATABASE_URL_ONLINE").ok();
        let database_url_offline = env::var("DATABASE_URL_OFFLINE").ok();

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
