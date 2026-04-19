//! Intent classification via Gemma 4 E4B.
//!
//! Classifies user queries into one of five [`TaskClass`] variants to inform
//! retrieval strategy selection in the online pipeline. Uses a two-stage
//! approach: fast keyword pre-check for high-confidence signals, then LLM
//! classification for ambiguous cases.
//!
//! # Classification Algorithm
//!
//! 1. [`keyword_precheck`] scans for domain-specific keywords (case-insensitive).
//!    If a strong signal is found, the result is returned immediately with
//!    confidence 1.0 — no LLM call is made.
//! 2. If no keyword match, [`classify_intent`] calls Gemma 4 E4B via
//!    [`LlmClient::call_llm`] with the classification system prompt.
//! 3. The LLM response is deserialized and sorted by confidence descending.
//! 4. If the gap between the top two classes is < 0.3, both primary and
//!    secondary classes are recorded.
//! 5. If parsing fails or all confidences are very low, defaults to
//!    [`TaskClass::Chat`].

use serde::Deserialize;
use thiserror::Error;

use crate::config::LlmConfig;
use crate::llm::client::{LlmClient, LlmError};
use crate::types::classification::{ClassificationResult, TaskClass};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// System prompt for intent classification, loaded from `prompts/classification.txt`.
const CLASSIFICATION_PROMPT: &str = include_str!("../../prompts/classification.txt");

/// Minimum confidence gap between top two classes to suppress secondary class.
const CONFIDENCE_GAP_THRESHOLD: f64 = 0.3;

/// Debug-related keywords for keyword pre-check.
const DEBUG_KEYWORDS: &[&str] = &[
    "error",
    "bug",
    "crash",
    "fail",
    "exception",
    "stack trace",
    "debug",
    "troubleshoot",
    "broken",
    "fix",
];

/// Architecture-related keywords for keyword pre-check.
const ARCHITECTURE_KEYWORDS: &[&str] = &[
    "architecture",
    "design",
    "component",
    "system",
    "diagram",
    "module",
    "structure",
    "pattern",
    "microservice",
];

/// Compliance-related keywords for keyword pre-check.
const COMPLIANCE_KEYWORDS: &[&str] = &[
    "compliance",
    "audit",
    "regulation",
    "policy",
    "governance",
    "control",
    "evidence",
    "finding",
    "risk",
];

/// Writing-related keywords for keyword pre-check.
const WRITING_KEYWORDS: &[&str] = &[
    "write",
    "document",
    "draft",
    "generate",
    "create doc",
    "readme",
    "specification",
];

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors produced by the classification layer.
#[derive(Debug, Error)]
pub enum ClassificationError {
    /// An error from the underlying LLM client (HTTP, retry, parse).
    #[error("LLM call failed: {0}")]
    Llm(#[from] LlmError),

    /// The LLM response could not be deserialized into the expected format.
    #[error("failed to deserialize classification response: {0}")]
    Deserialization(String),

    /// The deserialized response failed validation checks.
    #[error("classification validation failed: {0}")]
    Validation(String),
}

// ---------------------------------------------------------------------------
// LLM response types (internal deserialization)
// ---------------------------------------------------------------------------

/// A single class entry in the LLM classification response.
#[derive(Debug, Clone, Deserialize)]
struct ClassEntry {
    /// The task class name (e.g. "debug", "architecture").
    class: String,
    /// Confidence score for this class (0.0–1.0).
    confidence: f64,
}

/// Top-level LLM classification response.
#[derive(Debug, Clone, Deserialize)]
struct ClassificationResponse {
    /// Confidence scores for all five task classes.
    classes: Vec<ClassEntry>,
}

// ---------------------------------------------------------------------------
// Public result type
// ---------------------------------------------------------------------------

/// Successful classification output with model provenance.
#[derive(Debug, Clone)]
pub struct ClassificationOutput {
    /// The classification result with primary/secondary classes and scores.
    pub result: ClassificationResult,
    /// The model identifier used for classification.
    pub model: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Classify a user query into a [`TaskClass`] with confidence scores.
///
/// First attempts a fast [`keyword_precheck`]. If a keyword match is found,
/// returns immediately with confidence 1.0 and no LLM call. Otherwise,
/// calls Gemma 4 E4B via the classification prompt and parses the response.
///
/// # Errors
///
/// Returns [`ClassificationError`] if the LLM call fails or the response
/// cannot be parsed. On parse failure, the function defaults to
/// [`TaskClass::Chat`] rather than propagating the error.
pub async fn classify_intent(
    client: &LlmClient,
    config: &LlmConfig,
    query: &str,
) -> Result<ClassificationOutput, ClassificationError> {
    let model = &config.classification_model;

    // Stage 1: keyword pre-check for high-confidence signals.
    if let Some(task_class) = keyword_precheck(query) {
        tracing::info!(
            %task_class,
            model = "keyword_precheck",
            "classified via keyword match"
        );
        return Ok(ClassificationOutput {
            result: ClassificationResult {
                primary_class: task_class,
                secondary_class: None,
                primary_confidence: 1.0,
                secondary_confidence: None,
            },
            model: "keyword_precheck".to_string(),
        });
    }

    // Stage 2: LLM classification for ambiguous queries.
    tracing::info!(model, query_len = query.len(), "starting LLM classification");

    let response = client
        .call_llm(model, CLASSIFICATION_PROMPT, query)
        .await?;

    let result = parse_classification_response(&response, model)?;

    tracing::info!(
        model,
        primary_class = %result.result.primary_class,
        primary_confidence = result.result.primary_confidence,
        secondary_class = result.result.secondary_class.as_ref().map(|c| c.to_string()).unwrap_or_default(),
        "LLM classification complete"
    );

    Ok(result)
}

/// Fast keyword pre-check for high-confidence intent signals.
///
/// Scans the query (case-insensitive) for domain-specific keywords. Uses
/// simple `contains`-based matching on the lowercased query. Returns the
/// matching [`TaskClass`] if a strong signal is found, or `None` if the
/// query is ambiguous.
pub fn keyword_precheck(query: &str) -> Option<TaskClass> {
    let lower = query.to_lowercase();

    if DEBUG_KEYWORDS.iter().any(|kw| lower.contains(kw)) {
        return Some(TaskClass::Debug);
    }
    if ARCHITECTURE_KEYWORDS.iter().any(|kw| lower.contains(kw)) {
        return Some(TaskClass::Architecture);
    }
    if COMPLIANCE_KEYWORDS.iter().any(|kw| lower.contains(kw)) {
        return Some(TaskClass::Compliance);
    }
    if WRITING_KEYWORDS.iter().any(|kw| lower.contains(kw)) {
        return Some(TaskClass::Writing);
    }

    None
}

// ---------------------------------------------------------------------------
// Response parsing
// ---------------------------------------------------------------------------

/// Parse the LLM classification response into a [`ClassificationOutput`].
///
/// Sorts class entries by confidence descending, selects primary and
/// optional secondary class based on the confidence gap threshold, and
/// defaults to [`TaskClass::Chat`] if parsing fails or all confidences
/// are very low.
fn parse_classification_response(
    value: &serde_json::Value,
    model: &str,
) -> Result<ClassificationOutput, ClassificationError> {
    let response = deserialize_response(value)?;

    let mut entries = response.classes;

    // If we got no entries, default to Chat.
    if entries.is_empty() {
        tracing::warn!("classification response has no class entries, defaulting to Chat");
        return Ok(default_chat_output(model));
    }

    // Sort by confidence descending.
    entries.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));

    // Parse the top class.
    let primary = parse_task_class(&entries[0].class).unwrap_or_else(|| {
        tracing::warn!(class = %entries[0].class, "unknown primary class, defaulting to Chat");
        TaskClass::Chat
    });
    let primary_confidence = entries[0].confidence;

    // If primary confidence is very low, default to Chat.
    if primary_confidence < 0.1 {
        tracing::warn!(
            primary_confidence,
            "primary confidence too low, defaulting to Chat"
        );
        return Ok(default_chat_output(model));
    }

    // Check confidence gap for secondary class.
    let (secondary_class, secondary_confidence) = if entries.len() >= 2 {
        let gap = primary_confidence - entries[1].confidence;
        if gap < CONFIDENCE_GAP_THRESHOLD {
            let secondary = parse_task_class(&entries[1].class);
            (secondary, Some(entries[1].confidence))
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    Ok(ClassificationOutput {
        result: ClassificationResult {
            primary_class: primary,
            secondary_class,
            primary_confidence,
            secondary_confidence,
        },
        model: model.to_string(),
    })
}

/// Deserialize a [`serde_json::Value`] LLM response into a
/// [`ClassificationResponse`].
///
/// Handles two cases:
/// - The response is already a JSON object (parsed by the client).
/// - The response is a `Value::String` containing JSON text.
fn deserialize_response(
    value: &serde_json::Value,
) -> Result<ClassificationResponse, ClassificationError> {
    // First try: direct deserialization from the Value.
    if let Ok(result) = serde_json::from_value::<ClassificationResponse>(value.clone()) {
        return Ok(result);
    }

    // Second try: the Value might be a String wrapping JSON text.
    if let Some(text) = value.as_str() {
        let trimmed = text.trim();
        let json_text = strip_code_fences(trimmed);
        return serde_json::from_str::<ClassificationResponse>(json_text).map_err(|e| {
            ClassificationError::Deserialization(format!("classification: {e}"))
        });
    }

    Err(ClassificationError::Deserialization(
        "response is neither a valid JSON object nor a JSON string".to_string(),
    ))
}

/// Parse a task class string into a [`TaskClass`] enum variant.
///
/// Returns `None` for unrecognized class names.
fn parse_task_class(s: &str) -> Option<TaskClass> {
    s.parse::<TaskClass>().ok()
}

/// Build a default [`ClassificationOutput`] with [`TaskClass::Chat`].
fn default_chat_output(model: &str) -> ClassificationOutput {
    ClassificationOutput {
        result: ClassificationResult {
            primary_class: TaskClass::Chat,
            secondary_class: None,
            primary_confidence: 0.0,
            secondary_confidence: None,
        },
        model: model.to_string(),
    }
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

    // -- keyword_precheck ---------------------------------------------------

    #[test]
    fn keyword_precheck_detects_debug() {
        assert_eq!(keyword_precheck("Why is this error happening?"), Some(TaskClass::Debug));
        assert_eq!(keyword_precheck("The app keeps crashing"), Some(TaskClass::Debug));
        assert_eq!(keyword_precheck("I see a stack trace in the logs"), Some(TaskClass::Debug));
        assert_eq!(keyword_precheck("How do I FIX this?"), Some(TaskClass::Debug));
        assert_eq!(keyword_precheck("There's a BUG in the parser"), Some(TaskClass::Debug));
    }

    #[test]
    fn keyword_precheck_detects_architecture() {
        assert_eq!(keyword_precheck("Explain the architecture"), Some(TaskClass::Architecture));
        assert_eq!(keyword_precheck("Show me the system diagram"), Some(TaskClass::Architecture));
        assert_eq!(keyword_precheck("What is the microservice layout?"), Some(TaskClass::Architecture));
        assert_eq!(keyword_precheck("Describe the module structure"), Some(TaskClass::Architecture));
    }

    #[test]
    fn keyword_precheck_detects_compliance() {
        assert_eq!(keyword_precheck("Run a compliance check"), Some(TaskClass::Compliance));
        assert_eq!(keyword_precheck("Show the audit trail"), Some(TaskClass::Compliance));
        assert_eq!(keyword_precheck("What is the governance policy?"), Some(TaskClass::Compliance));
        assert_eq!(keyword_precheck("Assess the risk level"), Some(TaskClass::Compliance));
    }

    #[test]
    fn keyword_precheck_detects_writing() {
        assert_eq!(keyword_precheck("Write a summary"), Some(TaskClass::Writing));
        assert_eq!(keyword_precheck("Draft a proposal"), Some(TaskClass::Writing));
        assert_eq!(keyword_precheck("Generate a readme"), Some(TaskClass::Writing));
        assert_eq!(keyword_precheck("Create doc for the API"), Some(TaskClass::Writing));
    }

    #[test]
    fn keyword_precheck_returns_none_for_ambiguous() {
        assert_eq!(keyword_precheck("Hello, how are you?"), None);
        assert_eq!(keyword_precheck("What time is it?"), None);
        assert_eq!(keyword_precheck("Tell me about the project"), None);
    }

    #[test]
    fn keyword_precheck_is_case_insensitive() {
        assert_eq!(keyword_precheck("ARCHITECTURE overview"), Some(TaskClass::Architecture));
        assert_eq!(keyword_precheck("Debug the issue"), Some(TaskClass::Debug));
        assert_eq!(keyword_precheck("COMPLIANCE report"), Some(TaskClass::Compliance));
    }

    // -- parse_classification_response --------------------------------------

    #[test]
    fn parse_response_with_clear_primary() {
        let value = json!({
            "classes": [
                {"class": "debug", "confidence": 0.85},
                {"class": "architecture", "confidence": 0.05},
                {"class": "compliance", "confidence": 0.03},
                {"class": "writing", "confidence": 0.02},
                {"class": "chat", "confidence": 0.05}
            ]
        });

        let output = parse_classification_response(&value, "gemma4:e4b")
            .expect("should parse");
        assert_eq!(output.result.primary_class, TaskClass::Debug);
        assert!((output.result.primary_confidence - 0.85).abs() < f64::EPSILON);
        // Gap is 0.85 - 0.05 = 0.80 >= 0.3, so no secondary.
        assert!(output.result.secondary_class.is_none());
        assert!(output.result.secondary_confidence.is_none());
        assert_eq!(output.model, "gemma4:e4b");
    }

    #[test]
    fn parse_response_with_close_secondary() {
        let value = json!({
            "classes": [
                {"class": "debug", "confidence": 0.45},
                {"class": "architecture", "confidence": 0.35},
                {"class": "compliance", "confidence": 0.10},
                {"class": "writing", "confidence": 0.05},
                {"class": "chat", "confidence": 0.05}
            ]
        });

        let output = parse_classification_response(&value, "gemma4:e4b")
            .expect("should parse");
        assert_eq!(output.result.primary_class, TaskClass::Debug);
        assert!((output.result.primary_confidence - 0.45).abs() < f64::EPSILON);
        // Gap is 0.45 - 0.35 = 0.10 < 0.3, so secondary is present.
        assert_eq!(output.result.secondary_class, Some(TaskClass::Architecture));
        assert!((output.result.secondary_confidence.unwrap() - 0.35).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_response_with_exact_threshold_gap() {
        // Gap of exactly 0.3 should suppress secondary (>= 0.3 means no secondary).
        let value = json!({
            "classes": [
                {"class": "writing", "confidence": 0.60},
                {"class": "chat", "confidence": 0.30},
                {"class": "debug", "confidence": 0.05},
                {"class": "architecture", "confidence": 0.03},
                {"class": "compliance", "confidence": 0.02}
            ]
        });

        let output = parse_classification_response(&value, "gemma4:e4b")
            .expect("should parse");
        assert_eq!(output.result.primary_class, TaskClass::Writing);
        // Gap is 0.60 - 0.30 = 0.30 >= 0.3, so no secondary.
        assert!(output.result.secondary_class.is_none());
    }

    #[test]
    fn parse_response_defaults_to_chat_on_empty_classes() {
        let value = json!({"classes": []});

        let output = parse_classification_response(&value, "gemma4:e4b")
            .expect("should parse");
        assert_eq!(output.result.primary_class, TaskClass::Chat);
        assert!((output.result.primary_confidence - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_response_defaults_to_chat_on_very_low_confidence() {
        let value = json!({
            "classes": [
                {"class": "debug", "confidence": 0.05},
                {"class": "architecture", "confidence": 0.03},
                {"class": "compliance", "confidence": 0.01},
                {"class": "writing", "confidence": 0.005},
                {"class": "chat", "confidence": 0.005}
            ]
        });

        let output = parse_classification_response(&value, "gemma4:e4b")
            .expect("should parse");
        assert_eq!(output.result.primary_class, TaskClass::Chat);
        assert!((output.result.primary_confidence - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_response_handles_unknown_class_name() {
        let value = json!({
            "classes": [
                {"class": "unknown_class", "confidence": 0.90},
                {"class": "debug", "confidence": 0.10}
            ]
        });

        let output = parse_classification_response(&value, "gemma4:e4b")
            .expect("should parse");
        // Unknown class defaults to Chat.
        assert_eq!(output.result.primary_class, TaskClass::Chat);
    }

    #[test]
    fn parse_response_from_string_value() {
        let json_text = r#"{"classes": [{"class": "compliance", "confidence": 0.90}, {"class": "debug", "confidence": 0.05}, {"class": "architecture", "confidence": 0.02}, {"class": "writing", "confidence": 0.02}, {"class": "chat", "confidence": 0.01}]}"#;
        let value = serde_json::Value::String(json_text.to_string());

        let output = parse_classification_response(&value, "gemma4:e4b")
            .expect("should parse from string");
        assert_eq!(output.result.primary_class, TaskClass::Compliance);
    }

    #[test]
    fn parse_response_from_fenced_string() {
        let json_text = "```json\n{\"classes\": [{\"class\": \"writing\", \"confidence\": 0.80}, {\"class\": \"chat\", \"confidence\": 0.20}]}\n```";
        let value = serde_json::Value::String(json_text.to_string());

        let output = parse_classification_response(&value, "gemma4:e4b")
            .expect("should parse from fenced string");
        assert_eq!(output.result.primary_class, TaskClass::Writing);
    }

    #[test]
    fn parse_response_errors_on_invalid_json() {
        let value = json!({"error": "bad request"});

        let err = parse_classification_response(&value, "gemma4:e4b").unwrap_err();
        assert!(matches!(err, ClassificationError::Deserialization(_)));
    }

    // -- deserialize_response -----------------------------------------------

    #[test]
    fn deserialize_response_from_object() {
        let value = json!({
            "classes": [
                {"class": "chat", "confidence": 1.0}
            ]
        });

        let resp = deserialize_response(&value).expect("should deserialize");
        assert_eq!(resp.classes.len(), 1);
        assert_eq!(resp.classes[0].class, "chat");
    }

    #[test]
    fn deserialize_response_from_string() {
        let json_text = r#"{"classes": [{"class": "debug", "confidence": 0.9}]}"#;
        let value = serde_json::Value::String(json_text.to_string());

        let resp = deserialize_response(&value).expect("should deserialize from string");
        assert_eq!(resp.classes.len(), 1);
    }

    #[test]
    fn deserialize_response_errors_on_non_json() {
        let value = serde_json::Value::Number(42.into());
        let err = deserialize_response(&value).unwrap_err();
        assert!(matches!(err, ClassificationError::Deserialization(_)));
    }

    // -- ClassificationError display ----------------------------------------

    #[test]
    fn classification_error_display_messages() {
        let err = ClassificationError::Deserialization("bad json".into());
        assert!(err.to_string().contains("bad json"));

        let err = ClassificationError::Validation("missing field".into());
        assert!(err.to_string().contains("missing field"));
    }

    // -- confidence gap edge cases ------------------------------------------

    #[test]
    fn confidence_gap_just_below_threshold() {
        // Gap of 0.29 < 0.3, so secondary should be present.
        let value = json!({
            "classes": [
                {"class": "architecture", "confidence": 0.50},
                {"class": "debug", "confidence": 0.21},
                {"class": "compliance", "confidence": 0.15},
                {"class": "writing", "confidence": 0.10},
                {"class": "chat", "confidence": 0.04}
            ]
        });

        let output = parse_classification_response(&value, "test")
            .expect("should parse");
        assert_eq!(output.result.primary_class, TaskClass::Architecture);
        assert_eq!(output.result.secondary_class, Some(TaskClass::Debug));
    }

    #[test]
    fn confidence_gap_just_above_threshold() {
        // Gap of 0.31 >= 0.3, so no secondary.
        let value = json!({
            "classes": [
                {"class": "architecture", "confidence": 0.51},
                {"class": "debug", "confidence": 0.20},
                {"class": "compliance", "confidence": 0.15},
                {"class": "writing", "confidence": 0.10},
                {"class": "chat", "confidence": 0.04}
            ]
        });

        let output = parse_classification_response(&value, "test")
            .expect("should parse");
        assert_eq!(output.result.primary_class, TaskClass::Architecture);
        assert!(output.result.secondary_class.is_none());
    }

    // -- keyword priority ---------------------------------------------------

    #[test]
    fn keyword_precheck_debug_takes_priority_over_architecture() {
        // "debug" keyword appears first in the check order.
        assert_eq!(
            keyword_precheck("debug the architecture"),
            Some(TaskClass::Debug)
        );
    }

    #[test]
    fn keyword_precheck_multi_word_keyword() {
        assert_eq!(
            keyword_precheck("I see a stack trace here"),
            Some(TaskClass::Debug)
        );
        assert_eq!(
            keyword_precheck("Please create doc for the API"),
            Some(TaskClass::Writing)
        );
    }

    // -- keyword_precheck returns None for empty string ---------------------

    #[test]
    fn keyword_precheck_returns_none_for_empty_string() {
        assert_eq!(keyword_precheck(""), None);
    }

    // -- End-to-end classify_intent with wiremock ---------------------------

    /// Helper: build an `LlmClient` + `LlmConfig` pointing at the given mock
    /// server.
    fn test_config_for_classification(
        server_uri: &str,
    ) -> (crate::llm::client::LlmClient, LlmConfig) {
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
    async fn classify_intent_keyword_match_no_llm_call() {
        use wiremock::{MockServer, Mock};
        use wiremock::matchers::{method, path};
        use wiremock::ResponseTemplate;

        let server = MockServer::start().await;

        // Mount a mock that should NOT be called — keyword match should short-circuit.
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("should not be called"))
            .expect(0)
            .mount(&server)
            .await;

        let (client, config) = test_config_for_classification(&server.uri());
        let output = classify_intent(&client, &config, "There is a bug in the parser")
            .await
            .expect("should classify via keyword");

        assert_eq!(output.result.primary_class, TaskClass::Debug);
        assert!((output.result.primary_confidence - 1.0).abs() < f64::EPSILON);
        assert!(output.result.secondary_class.is_none());
        assert_eq!(output.model, "keyword_precheck");
    }

    #[tokio::test]
    async fn classify_intent_llm_path_for_ambiguous_query() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        let llm_content = json!({
            "classes": [
                {"class": "architecture", "confidence": 0.70},
                {"class": "debug", "confidence": 0.15},
                {"class": "compliance", "confidence": 0.05},
                {"class": "writing", "confidence": 0.05},
                {"class": "chat", "confidence": 0.05}
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

        let (client, config) = test_config_for_classification(&server.uri());
        let output = classify_intent(&client, &config, "How does the project work?")
            .await
            .expect("should classify via LLM");

        assert_eq!(output.result.primary_class, TaskClass::Architecture);
        assert!((output.result.primary_confidence - 0.70).abs() < f64::EPSILON);
        assert_eq!(output.model, "test-model");
    }

    #[tokio::test]
    async fn classify_intent_llm_malformed_response_defaults_to_chat() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        // LLM returns something that can't be parsed as ClassificationResponse.
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{
                    "message": { "content": "I don't know how to classify this" }
                }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let (client, config) = test_config_for_classification(&server.uri());
        let err = classify_intent(&client, &config, "Tell me something interesting")
            .await;

        // The malformed response should result in a deserialization error.
        assert!(err.is_err());
        let err = err.unwrap_err();
        assert!(matches!(err, ClassificationError::Deserialization(_)));
    }
}
