// loom-engine/src/config.rs
// Application and LLM configuration loaded from environment variables.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    pub database_url: String,
    pub database_url_online: Option<String>,
    pub database_url_offline: Option<String>,
    pub loom_host: String,
    pub loom_port: u16,
    pub loom_bearer_token: String,
    pub llm: LlmConfig,
}

#[derive(Debug, Deserialize)]
pub struct LlmConfig {
    pub ollama_url: String,
    pub extraction_model: String,
    pub classification_model: String,
    pub embedding_model: String,
    pub azure_openai_url: Option<String>,
    pub azure_openai_key: Option<String>,
}
