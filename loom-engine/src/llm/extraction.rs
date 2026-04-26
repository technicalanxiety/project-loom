//! Entity and fact extraction prompt execution via the configured extraction
//! model (see ADR-009 for tier guidance — `qwen2.5:14b` on iGPU/APU hosts,
//! `gemma4:26b` on discrete-GPU hosts, `gemma4:e4b` as a last resort).
//!
//! Provides [`extract_entities`] and [`extract_facts`] functions that call
//! the LLM via [`LlmClient::call_llm_with_schema`], passing JSON Schemas
//! derived from [`ExtractionResponse`] / [`FactExtractionResponse`] so the
//! provider constrains generation to schema-conformant output. See
//! ADR-011 for why constrained generation replaced free-form prompting.
//!
//! Prompt templates are embedded as constants loaded from the `prompts/`
//! directory at compile time.

use schemars::{schema_for, JsonSchema};
use serde::Deserialize;
use thiserror::Error;

use crate::config::LlmConfig;
use crate::llm::client::{LlmClient, LlmError};
use crate::types::entity::ExtractedEntity;
use crate::types::fact::ExtractedFact;

// ---------------------------------------------------------------------------
// Constants — prompt templates compiled into the binary
// ---------------------------------------------------------------------------

/// System prompt for entity extraction, loaded from `prompts/entity_extraction.txt`.
const ENTITY_EXTRACTION_PROMPT: &str = include_str!("../../prompts/entity_extraction.txt");

/// System prompt template for fact extraction, loaded from `prompts/fact_extraction.txt`.
/// Contains `{{PREDICATE_BLOCK}}` and `{{ENTITY_NAMES}}` placeholders for dynamic injection.
const FACT_EXTRACTION_TEMPLATE: &str = include_str!("../../prompts/fact_extraction.txt");

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors produced by the extraction layer.
#[derive(Debug, Error)]
pub enum ExtractionError {
    /// An error from the underlying LLM client (HTTP, retry, parse).
    #[error("LLM call failed: {0}")]
    Llm(#[from] LlmError),

    /// The LLM response could not be deserialized into the expected type.
    #[error("failed to deserialize LLM response: {0}")]
    Deserialization(String),

    /// The deserialized response failed validation checks.
    #[error("response validation failed: {0}")]
    Validation(String),
}

// ---------------------------------------------------------------------------
// Response wrappers (for serde deserialization)
// ---------------------------------------------------------------------------

/// Wrapper for the entity extraction LLM response.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ExtractionResponse {
    /// Extracted entities from the episode.
    pub entities: Vec<ExtractedEntity>,
}

/// Wrapper for the fact extraction LLM response.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct FactExtractionResponse {
    /// Extracted facts from the episode.
    pub facts: Vec<ExtractedFact>,
}

// ---------------------------------------------------------------------------
// Public result types (include model provenance)
// ---------------------------------------------------------------------------

/// Successful entity extraction result with model provenance.
#[derive(Debug, Clone)]
pub struct EntityExtractionResult {
    /// The extracted entities.
    pub entities: Vec<ExtractedEntity>,
    /// The model identifier used for extraction.
    pub model: String,
}

/// Successful fact extraction result with model provenance.
#[derive(Debug, Clone)]
pub struct FactExtractionResult {
    /// The extracted facts.
    pub facts: Vec<ExtractedFact>,
    /// The model identifier used for extraction.
    pub model: String,
}

// ---------------------------------------------------------------------------
// Entity extraction
// ---------------------------------------------------------------------------

/// Extract entities from episode content using the configured extraction model.
///
/// Sends the episode content to the LLM with the entity extraction system
/// prompt, deserializes the JSON response into [`ExtractedEntity`] structs,
/// and validates that each entity has a non-empty name.
///
/// # Errors
///
/// Returns [`ExtractionError`] if the LLM call fails, the response cannot
/// be deserialized, or validation checks fail.
pub async fn extract_entities(
    client: &LlmClient,
    config: &LlmConfig,
    episode_content: &str,
) -> Result<EntityExtractionResult, ExtractionError> {
    let model = &config.extraction_model;

    tracing::info!(model, content_len = episode_content.len(), "starting entity extraction");

    let schema = serde_json::to_value(schema_for!(ExtractionResponse))
        .map_err(|e| ExtractionError::Deserialization(format!("entity schema build: {e}")))?;

    let response = client
        .call_llm_with_schema(
            model,
            ENTITY_EXTRACTION_PROMPT,
            episode_content,
            "entity_extraction",
            &schema,
        )
        .await?;

    let extraction: ExtractionResponse = deserialize_response(&response, "entity extraction")?;

    // Validate: every entity must have a non-empty name.
    for (i, entity) in extraction.entities.iter().enumerate() {
        if entity.name.trim().is_empty() {
            return Err(ExtractionError::Validation(format!(
                "entity at index {i} has an empty name"
            )));
        }
    }

    tracing::info!(
        model,
        entity_count = extraction.entities.len(),
        "entity extraction complete"
    );

    Ok(EntityExtractionResult {
        entities: extraction.entities,
        model: model.clone(),
    })
}

// ---------------------------------------------------------------------------
// Fact extraction
// ---------------------------------------------------------------------------

/// Extract facts from episode content using a pack-aware prompt.
///
/// Assembles the fact extraction prompt by injecting the predicate block
/// and entity names into the template, then sends the episode content to
/// the LLM. The JSON response is deserialized into [`ExtractedFact`]
/// structs and validated.
///
/// # Arguments
///
/// * `client` — The LLM client for Ollama / Azure OpenAI calls.
/// * `config` — LLM configuration with model names.
/// * `episode_content` — The raw episode text to extract facts from.
/// * `entity_names` — Names of entities already extracted from this episode.
/// * `predicate_block` — Formatted predicate registry block grouped by pack.
///
/// # Errors
///
/// Returns [`ExtractionError`] if the LLM call fails, the response cannot
/// be deserialized, or validation checks fail.
pub async fn extract_facts(
    client: &LlmClient,
    config: &LlmConfig,
    episode_content: &str,
    entity_names: &[String],
    predicate_block: &str,
) -> Result<FactExtractionResult, ExtractionError> {
    let model = &config.extraction_model;

    tracing::info!(
        model,
        content_len = episode_content.len(),
        entity_count = entity_names.len(),
        "starting fact extraction"
    );

    let system_prompt = assemble_fact_prompt(predicate_block, entity_names);

    let schema = serde_json::to_value(schema_for!(FactExtractionResponse))
        .map_err(|e| ExtractionError::Deserialization(format!("fact schema build: {e}")))?;

    let response = client
        .call_llm_with_schema(
            model,
            &system_prompt,
            episode_content,
            "fact_extraction",
            &schema,
        )
        .await?;

    let extraction: FactExtractionResponse = deserialize_response(&response, "fact extraction")?;

    // Validate: subject and object must be non-empty.
    for (i, fact) in extraction.facts.iter().enumerate() {
        if fact.subject.trim().is_empty() {
            return Err(ExtractionError::Validation(format!(
                "fact at index {i} has an empty subject"
            )));
        }
        if fact.predicate.trim().is_empty() {
            return Err(ExtractionError::Validation(format!(
                "fact at index {i} has an empty predicate"
            )));
        }
        if fact.object.trim().is_empty() {
            return Err(ExtractionError::Validation(format!(
                "fact at index {i} has an empty object"
            )));
        }
    }

    tracing::info!(
        model,
        fact_count = extraction.facts.len(),
        "fact extraction complete"
    );

    Ok(FactExtractionResult {
        facts: extraction.facts,
        model: model.clone(),
    })
}

// ---------------------------------------------------------------------------
// Prompt assembly
// ---------------------------------------------------------------------------

/// Assemble the fact extraction system prompt by injecting the predicate
/// block and entity names into the template.
///
/// Replaces `{{PREDICATE_BLOCK}}` with the formatted predicate registry
/// and `{{ENTITY_NAMES}}` with a bullet list of entity names.
pub fn assemble_fact_prompt(predicate_block: &str, entity_names: &[String]) -> String {
    let entity_list = if entity_names.is_empty() {
        "(none)".to_string()
    } else {
        entity_names
            .iter()
            .map(|n| format!("- {n}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    FACT_EXTRACTION_TEMPLATE
        .replace("{{PREDICATE_BLOCK}}", predicate_block)
        .replace("{{ENTITY_NAMES}}", &entity_list)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Deserialize a [`serde_json::Value`] LLM response into the target type `T`.
///
/// Handles two cases:
/// - The response is already a JSON object (parsed by the client).
/// - The response is a `Value::String` containing JSON text that needs a
///   second parse pass.
///
/// Returns [`ExtractionError::Deserialization`] on failure.
fn deserialize_response<T: serde::de::DeserializeOwned>(
    value: &serde_json::Value,
    context: &str,
) -> Result<T, ExtractionError> {
    // First try: direct deserialization from the Value (already parsed JSON).
    if let Ok(result) = serde_json::from_value::<T>(value.clone()) {
        return Ok(result);
    }

    // Second try: the Value might be a String wrapping JSON text.
    if let Some(text) = value.as_str() {
        let trimmed = text.trim();
        // Strip markdown code fences if the LLM wrapped the response.
        let json_text = strip_code_fences(trimmed);
        return serde_json::from_str::<T>(json_text).map_err(|e| {
            ExtractionError::Deserialization(format!("{context}: {e}"))
        });
    }

    Err(ExtractionError::Deserialization(format!(
        "{context}: response is neither a valid JSON object nor a JSON string"
    )))
}

/// Strip optional markdown code fences (```json ... ```) from LLM output.
fn strip_code_fences(s: &str) -> &str {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("```json") {
        rest.strip_suffix("```").unwrap_or(rest).trim()
    } else if let Some(rest) = s.strip_prefix("```") {
        rest.strip_suffix("```").unwrap_or(rest).trim()
    } else {
        s
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -- ExtractionResponse deserialization ----------------------------------

    #[test]
    fn deserialize_valid_entity_response() {
        let value = json!({
            "entities": [
                {
                    "name": "Rust",
                    "entity_type": "technology",
                    "aliases": ["rust-lang"],
                    "properties": {}
                },
                {
                    "name": "Alice",
                    "entity_type": "person",
                    "aliases": [],
                    "properties": {}
                }
            ]
        });

        let result: ExtractionResponse =
            deserialize_response(&value, "test").expect("should deserialize");
        assert_eq!(result.entities.len(), 2);
        assert_eq!(result.entities[0].name, "Rust");
        assert_eq!(result.entities[0].entity_type, "technology");
        assert_eq!(result.entities[0].aliases, vec!["rust-lang"]);
        assert_eq!(result.entities[1].name, "Alice");
    }

    #[test]
    fn deserialize_entity_response_with_defaults() {
        // aliases and properties should default when missing.
        let value = json!({
            "entities": [
                {
                    "name": "PostgreSQL",
                    "entity_type": "technology"
                }
            ]
        });

        let result: ExtractionResponse =
            deserialize_response(&value, "test").expect("should deserialize");
        assert_eq!(result.entities.len(), 1);
        assert_eq!(result.entities[0].name, "PostgreSQL");
        assert!(result.entities[0].aliases.is_empty());
    }

    #[test]
    fn deserialize_entity_response_from_string_value() {
        let json_text = r#"{"entities": [{"name": "Tokio", "entity_type": "technology"}]}"#;
        let value = serde_json::Value::String(json_text.to_string());

        let result: ExtractionResponse =
            deserialize_response(&value, "test").expect("should deserialize from string");
        assert_eq!(result.entities.len(), 1);
        assert_eq!(result.entities[0].name, "Tokio");
    }

    #[test]
    fn deserialize_entity_response_from_fenced_string() {
        let json_text = "```json\n{\"entities\": [{\"name\": \"Axum\", \"entity_type\": \"technology\"}]}\n```";
        let value = serde_json::Value::String(json_text.to_string());

        let result: ExtractionResponse =
            deserialize_response(&value, "test").expect("should deserialize from fenced string");
        assert_eq!(result.entities[0].name, "Axum");
    }

    #[test]
    fn deserialize_malformed_entity_response_missing_entities_key() {
        let value = json!({"items": [{"name": "X"}]});
        let err = deserialize_response::<ExtractionResponse>(&value, "test").unwrap_err();
        assert!(matches!(err, ExtractionError::Deserialization(_)));
    }

    #[test]
    fn deserialize_malformed_entity_response_wrong_type() {
        let value = json!({"entities": "not an array"});
        let err = deserialize_response::<ExtractionResponse>(&value, "test").unwrap_err();
        assert!(matches!(err, ExtractionError::Deserialization(_)));
    }

    #[test]
    fn deserialize_malformed_entity_response_missing_name() {
        let value = json!({"entities": [{"entity_type": "person"}]});
        let err = deserialize_response::<ExtractionResponse>(&value, "test").unwrap_err();
        assert!(matches!(err, ExtractionError::Deserialization(_)));
    }

    // -- FactExtractionResponse deserialization ------------------------------

    #[test]
    fn deserialize_valid_fact_response() {
        let value = json!({
            "facts": [
                {
                    "subject": "Project Sentinel",
                    "predicate": "uses",
                    "object": "Rust",
                    "evidence_strength": "explicit",
                    "custom": false,
                    "temporal_markers": {
                        "valid_from": "2025-01-15T00:00:00Z",
                        "valid_until": null
                    }
                }
            ]
        });

        let result: FactExtractionResponse =
            deserialize_response(&value, "test").expect("should deserialize");
        assert_eq!(result.facts.len(), 1);
        assert_eq!(result.facts[0].subject, "Project Sentinel");
        assert_eq!(result.facts[0].predicate, "uses");
        assert_eq!(result.facts[0].object, "Rust");
        assert!(!result.facts[0].custom);
        assert_eq!(
            result.facts[0].evidence_strength.as_deref(),
            Some("explicit")
        );
        assert!(result.facts[0].temporal_markers.is_some());
    }

    #[test]
    fn deserialize_fact_response_with_defaults() {
        // custom defaults to false, temporal_markers defaults to None.
        let value = json!({
            "facts": [
                {
                    "subject": "Alice",
                    "predicate": "manages",
                    "object": "Platform Team",
                    "evidence_strength": "implied"
                }
            ]
        });

        let result: FactExtractionResponse =
            deserialize_response(&value, "test").expect("should deserialize");
        assert_eq!(result.facts.len(), 1);
        assert!(!result.facts[0].custom);
        assert!(result.facts[0].temporal_markers.is_none());
    }

    #[test]
    fn deserialize_malformed_fact_response_missing_subject() {
        let value = json!({
            "facts": [{"predicate": "uses", "object": "Rust"}]
        });
        let err = deserialize_response::<FactExtractionResponse>(&value, "test").unwrap_err();
        assert!(matches!(err, ExtractionError::Deserialization(_)));
    }

    #[test]
    fn deserialize_malformed_fact_response_not_array() {
        let value = json!({"facts": "not an array"});
        let err = deserialize_response::<FactExtractionResponse>(&value, "test").unwrap_err();
        assert!(matches!(err, ExtractionError::Deserialization(_)));
    }

    #[test]
    fn deserialize_empty_entity_response() {
        let value = json!({"entities": []});
        let result: ExtractionResponse =
            deserialize_response(&value, "test").expect("should deserialize");
        assert!(result.entities.is_empty());
    }

    #[test]
    fn deserialize_empty_fact_response() {
        let value = json!({"facts": []});
        let result: FactExtractionResponse =
            deserialize_response(&value, "test").expect("should deserialize");
        assert!(result.facts.is_empty());
    }

    // -- assemble_fact_prompt -----------------------------------------------

    #[test]
    fn assemble_fact_prompt_injects_predicate_block() {
        let predicate_block = "## core\n- uses\n- depends_on";
        let entity_names = vec!["Rust".to_string(), "PostgreSQL".to_string()];

        let prompt = assemble_fact_prompt(predicate_block, &entity_names);

        assert!(prompt.contains("## core\n- uses\n- depends_on"));
        assert!(prompt.contains("- Rust"));
        assert!(prompt.contains("- PostgreSQL"));
        // Placeholders should be replaced.
        assert!(!prompt.contains("{{PREDICATE_BLOCK}}"));
        assert!(!prompt.contains("{{ENTITY_NAMES}}"));
    }

    #[test]
    fn assemble_fact_prompt_handles_empty_entities() {
        let prompt = assemble_fact_prompt("## core\n- uses", &[]);
        assert!(prompt.contains("(none)"));
        assert!(!prompt.contains("{{ENTITY_NAMES}}"));
    }

    #[test]
    fn assemble_fact_prompt_handles_empty_predicate_block() {
        let entity_names = vec!["Rust".to_string()];
        let prompt = assemble_fact_prompt("", &entity_names);
        assert!(!prompt.contains("{{PREDICATE_BLOCK}}"));
        assert!(prompt.contains("- Rust"));
    }

    // -- strip_code_fences --------------------------------------------------

    #[test]
    fn strip_code_fences_removes_json_fence() {
        let input = "```json\n{\"key\": \"value\"}\n```";
        assert_eq!(strip_code_fences(input), "{\"key\": \"value\"}");
    }

    #[test]
    fn strip_code_fences_removes_plain_fence() {
        let input = "```\n{\"key\": \"value\"}\n```";
        assert_eq!(strip_code_fences(input), "{\"key\": \"value\"}");
    }

    #[test]
    fn strip_code_fences_leaves_plain_json() {
        let input = "{\"key\": \"value\"}";
        assert_eq!(strip_code_fences(input), "{\"key\": \"value\"}");
    }

    // -- ExtractionError display --------------------------------------------

    #[test]
    fn extraction_error_display_messages() {
        let err = ExtractionError::Deserialization("bad json".into());
        assert!(err.to_string().contains("bad json"));

        let err = ExtractionError::Validation("empty name".into());
        assert!(err.to_string().contains("empty name"));
    }

    // -- End-to-end extract_entities with wiremock --------------------------

    /// Helper: build an `LlmClient` + `LlmConfig` pointing at the given mock
    /// server.
    fn test_config(server_uri: &str) -> (crate::llm::client::LlmClient, LlmConfig) {
        let config = LlmConfig {
            ollama_url: server_uri.to_string(),
            extraction_model: "test-model".to_string(),
            classification_model: "test-model".to_string(),
            embedding_model: "nomic-embed-text".to_string(),
            azure_openai_url: None,
            azure_openai_key: None,
        };
        let client =
            crate::llm::client::LlmClient::new(&config).expect("should build client");
        (client, config)
    }

    #[tokio::test]
    async fn extract_entities_end_to_end_valid_json() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        let llm_content = json!({
            "entities": [
                {
                    "name": "Rust",
                    "entity_type": "technology",
                    "aliases": ["rust-lang"],
                    "properties": {}
                },
                {
                    "name": "Alice",
                    "entity_type": "person",
                    "aliases": [],
                    "properties": {}
                }
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

        let (client, config) = test_config(&server.uri());
        let result = extract_entities(&client, &config, "Alice uses Rust for systems programming")
            .await
            .expect("should extract entities");

        assert_eq!(result.entities.len(), 2);
        assert_eq!(result.entities[0].name, "Rust");
        assert_eq!(result.entities[1].name, "Alice");
        assert_eq!(result.model, "test-model");
    }

    #[tokio::test]
    async fn extract_entities_end_to_end_malformed_json() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{
                    "message": { "content": "this is not valid json at all" }
                }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let (client, config) = test_config(&server.uri());
        let err = extract_entities(&client, &config, "some episode content")
            .await
            .unwrap_err();

        assert!(matches!(err, ExtractionError::Deserialization(_)));
    }

    #[tokio::test]
    async fn extract_entities_end_to_end_empty_name_validation() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        let llm_content = json!({
            "entities": [
                {
                    "name": "",
                    "entity_type": "person",
                    "aliases": [],
                    "properties": {}
                }
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

        let (client, config) = test_config(&server.uri());
        let err = extract_entities(&client, &config, "some episode content")
            .await
            .unwrap_err();

        assert!(matches!(err, ExtractionError::Validation(_)));
        assert!(err.to_string().contains("empty name"));
    }

    // -- End-to-end extract_facts with wiremock -----------------------------

    #[tokio::test]
    async fn extract_facts_end_to_end_valid_json() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        let llm_content = json!({
            "facts": [
                {
                    "subject": "Project Sentinel",
                    "predicate": "uses",
                    "object": "Rust",
                    "custom": false,
                    "evidence_strength": "explicit"
                }
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

        let (client, config) = test_config(&server.uri());
        let result = extract_facts(
            &client,
            &config,
            "Project Sentinel uses Rust",
            &["Project Sentinel".to_string(), "Rust".to_string()],
            "## core\n- uses",
        )
        .await
        .expect("should extract facts");

        assert_eq!(result.facts.len(), 1);
        assert_eq!(result.facts[0].subject, "Project Sentinel");
        assert_eq!(result.facts[0].predicate, "uses");
        assert_eq!(result.facts[0].object, "Rust");
        assert!(!result.facts[0].custom);
        assert_eq!(result.model, "test-model");
    }

    #[tokio::test]
    async fn extract_facts_end_to_end_empty_subject_validation() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        let llm_content = json!({
            "facts": [
                {
                    "subject": "",
                    "predicate": "uses",
                    "object": "Rust",
                    "custom": false,
                    "evidence_strength": "explicit"
                }
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

        let (client, config) = test_config(&server.uri());
        let err = extract_facts(
            &client,
            &config,
            "some content",
            &["Rust".to_string()],
            "## core\n- uses",
        )
        .await
        .unwrap_err();

        assert!(matches!(err, ExtractionError::Validation(_)));
        assert!(err.to_string().contains("empty subject"));
    }

    #[tokio::test]
    async fn extract_facts_end_to_end_empty_predicate_validation() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        let llm_content = json!({
            "facts": [
                {
                    "subject": "Alice",
                    "predicate": "  ",
                    "object": "Rust",
                    "custom": false,
                    "evidence_strength": "explicit"
                }
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

        let (client, config) = test_config(&server.uri());
        let err = extract_facts(
            &client,
            &config,
            "some content",
            &["Alice".to_string(), "Rust".to_string()],
            "## core\n- uses",
        )
        .await
        .unwrap_err();

        assert!(matches!(err, ExtractionError::Validation(_)));
        assert!(err.to_string().contains("empty predicate"));
    }

    // -- Entity with invalid entity_type (still deserializes as String) -----

    #[test]
    fn deserialize_entity_with_invalid_entity_type_value() {
        let value = json!({
            "entities": [
                {
                    "name": "FooBar",
                    "entity_type": "completely_made_up_type",
                    "aliases": [],
                    "properties": {}
                }
            ]
        });

        let result: ExtractionResponse =
            deserialize_response(&value, "test").expect("should deserialize");
        assert_eq!(result.entities.len(), 1);
        assert_eq!(result.entities[0].entity_type, "completely_made_up_type");
    }

    // -- Fact extraction with multiple facts including temporal markers ------

    #[tokio::test]
    async fn extract_facts_with_multiple_facts_and_temporal_markers() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        let llm_content = json!({
            "facts": [
                {
                    "subject": "Alice",
                    "predicate": "manages",
                    "object": "Platform Team",
                    "custom": false,
                    "evidence_strength": "explicit",
                    "temporal_markers": {
                        "valid_from": "2025-01-15T00:00:00Z",
                        "valid_until": null
                    }
                },
                {
                    "subject": "Platform Team",
                    "predicate": "uses",
                    "object": "Rust",
                    "custom": false,
                    "evidence_strength": "implied"
                },
                {
                    "subject": "Alice",
                    "predicate": "authored",
                    "object": "Design Doc",
                    "custom": false,
                    "evidence_strength": "explicit",
                    "temporal_markers": {
                        "valid_from": "2025-02-01T00:00:00Z",
                        "valid_until": "2025-06-01T00:00:00Z"
                    }
                }
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

        let (client, config) = test_config(&server.uri());
        let result = extract_facts(
            &client,
            &config,
            "Alice manages Platform Team which uses Rust. Alice authored Design Doc.",
            &[
                "Alice".to_string(),
                "Platform Team".to_string(),
                "Rust".to_string(),
                "Design Doc".to_string(),
            ],
            "## core\n- manages\n- uses\n- authored",
        )
        .await
        .expect("should extract multiple facts");

        assert_eq!(result.facts.len(), 3);

        // First fact has temporal markers with valid_from only.
        assert!(result.facts[0].temporal_markers.is_some());
        let tm0 = result.facts[0].temporal_markers.as_ref().unwrap();
        assert!(tm0.valid_from.is_some());
        assert!(tm0.valid_until.is_none());

        // Second fact has no temporal markers.
        assert!(result.facts[1].temporal_markers.is_none());

        // Third fact has both valid_from and valid_until.
        assert!(result.facts[2].temporal_markers.is_some());
        let tm2 = result.facts[2].temporal_markers.as_ref().unwrap();
        assert!(tm2.valid_from.is_some());
        assert!(tm2.valid_until.is_some());
    }
}
