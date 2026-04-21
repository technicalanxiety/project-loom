//! MCP (Model Context Protocol) request and response types.
//!
//! These types define the JSON-RPC interface for AI tool integration.
//! Three endpoints are exposed: `loom_learn` (episode ingestion),
//! `loom_think` (context compilation), and `loom_recall` (raw search).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::fact::Fact;
use super::ingestion::IngestionMode;

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

/// Request payload for the `loom_learn` endpoint.
///
/// Accepts episode content from AI assistants and other source systems.
///
/// # Ingestion mode contract
///
/// `ingestion_mode` is optional on the wire but required in effect.
///
/// * **MCP handler**: the server overwrites any client-supplied value with
///   [`IngestionMode::LiveMcpCapture`]. MCP clients are not expected to
///   include the field in their `loom_learn` tool calls at all.
/// * **REST handler**: the field is mandatory. Requests missing it are
///   rejected with HTTP 400. Bootstrap scripts send `VendorImport` plus
///   parser metadata, the CLI seed tool sends `UserAuthoredSeed`, and the
///   Claude Code PostSession hook sends `LiveMcpCapture`.
///
/// `parser_version` and `parser_source_schema` are required when
/// `ingestion_mode = VendorImport` and must be absent otherwise. This
/// mirrors the `chk_parser_fields_vendor_import` database constraint.
///
/// The `content` field must be verbatim: a transcript excerpt, a vendor
/// export excerpt, or user-authored prose. Never LLM summarization output.
/// This is a trust-based invariant documented in ADR 005; the server
/// cannot enforce it at runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearnRequest {
    /// Raw episode text content.
    pub content: String,
    /// Source system identifier (e.g. "claude-code", "manual", "github").
    pub source: String,
    /// Namespace isolation boundary.
    pub namespace: String,
    /// Provenance class for this episode. Optional on the wire so that MCP
    /// clients do not need to know about it (the MCP handler always
    /// overwrites it to `live_mcp_capture`); required in effect on the
    /// REST path, which returns 400 if absent.
    #[serde(default)]
    pub ingestion_mode: Option<IngestionMode>,
    /// Semantic version of the parser that produced this content.
    /// Required when `ingestion_mode = VendorImport`; rejected otherwise.
    #[serde(default)]
    pub parser_version: Option<String>,
    /// Vendor export schema version asserted against.
    /// Required when `ingestion_mode = VendorImport`; rejected otherwise.
    #[serde(default)]
    pub parser_source_schema: Option<String>,
    /// When the interaction occurred.
    pub occurred_at: Option<DateTime<Utc>>,
    /// Flexible source-specific metadata.
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    /// People involved in the interaction.
    #[serde(default)]
    pub participants: Option<Vec<String>>,
    /// Deduplication key within source.
    pub source_event_id: Option<String>,
}

/// Request payload for the `loom_think` endpoint.
///
/// Triggers intent classification, retrieval, ranking, and context
/// package compilation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkRequest {
    /// The query to compile context for.
    pub query: String,
    /// Namespace isolation boundary.
    pub namespace: String,
    /// Override the classified task class.
    pub task_class_override: Option<String>,
    /// Which AI model the context is being compiled for.
    pub target_model: Option<String>,
}

/// Request payload for the `loom_recall` endpoint.
///
/// Returns raw search results without compilation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallRequest {
    /// Entity names to look up.
    pub entity_names: Vec<String>,
    /// Namespace isolation boundary.
    pub namespace: String,
    /// Whether to include historical (superseded) facts.
    #[serde(default)]
    pub include_historical: bool,
}

// ---------------------------------------------------------------------------
// Responses
// ---------------------------------------------------------------------------

/// Response payload for the `loom_learn` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearnResponse {
    /// The ingested episode identifier.
    pub episode_id: Uuid,
    /// Ingestion status: "accepted", "duplicate", or "queued".
    pub status: String,
}

/// Response payload for the `loom_think` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThinkResponse {
    /// The compiled context package (XML structured or JSON compact).
    pub context_package: String,
    /// Total token count of the compiled package.
    pub token_count: i32,
    /// Unique compilation identifier for audit trail.
    pub compilation_id: Uuid,
}

/// Response payload for the `loom_recall` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallResponse {
    /// Raw facts matching the recall query.
    pub facts: Vec<Fact>,
}
