//! MCP (Model Context Protocol) JSON-RPC 2.0 dispatcher.
//!
//! Translates the MCP wire protocol used by MCP clients (Claude Desktop,
//! ChatGPT Desktop, GitHub Copilot, M365 Copilot, Claude Code, and the
//! `mcp-remote` stdio bridge) into calls against the existing per-tool
//! handlers in [`api::mcp`](super::mcp).
//!
//! The endpoint is a single route — `POST /mcp` — that accepts JSON-RPC
//! 2.0 envelopes and dispatches on the `method` field:
//!
//! | Method | Response |
//! |--------|----------|
//! | `initialize` | Protocol handshake — `protocolVersion`, `capabilities`, `serverInfo` |
//! | `notifications/initialized` | No response (notification) |
//! | `ping` | Empty result |
//! | `tools/list` | Catalog of the 3 Loom tools with JSON Schema `inputSchema`s |
//! | `tools/call` | Dispatches to `handle_loom_learn`, `handle_loom_think`, or `handle_loom_recall` |
//! | anything else | JSON-RPC error `-32601 Method not found` |
//!
//! Auth: the existing bearer-token middleware applies at the router level,
//! same as the REST-style per-tool endpoints under `/mcp/loom_*`. Those
//! REST endpoints remain mounted for direct-curl testing and for callers
//! that pre-date this dispatcher; new MCP clients should use this one.

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::mcp::{
    handle_loom_learn, handle_loom_recall, handle_loom_think, AppState,
};
use crate::types::mcp::{LearnRequest, RecallRequest, ThinkRequest};

// ---------------------------------------------------------------------------
// JSON-RPC 2.0 envelope types
// ---------------------------------------------------------------------------

/// Top-level JSON-RPC 2.0 request envelope.
///
/// Notifications are requests with no `id` field — they expect no response
/// per the spec. The dispatcher returns HTTP 204 for notifications.
#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

/// Top-level JSON-RPC 2.0 response envelope.
///
/// Exactly one of `result` and `error` is serialized; the other is skipped
/// via `skip_serializing_if`. This keeps responses compliant with clients
/// that reject envelopes containing both.
#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcErrorObj>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcErrorObj {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcResponse {
    fn result(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(JsonRpcErrorObj {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

// Canonical JSON-RPC 2.0 error codes. `ERR_PARSE` and `ERR_INTERNAL` are
// kept as named constants for future use and as a reference for anyone
// reading the dispatcher; the current code paths do not emit them, so
// `#[allow(dead_code)]` suppresses the unused-const warning.
#[allow(dead_code)]
const ERR_PARSE: i32 = -32700;
const ERR_INVALID_REQUEST: i32 = -32600;
const ERR_METHOD_NOT_FOUND: i32 = -32601;
const ERR_INVALID_PARAMS: i32 = -32602;
#[allow(dead_code)]
const ERR_INTERNAL: i32 = -32603;

// ---------------------------------------------------------------------------
// Protocol version + server info
// ---------------------------------------------------------------------------

/// Default MCP protocol version to advertise when the client does not send
/// one on `initialize`. Kept recent enough that modern clients accept it;
/// older clients negotiate down via their own echo logic.
const DEFAULT_PROTOCOL_VERSION: &str = "2025-06-18";

/// Negotiate protocol version. The MCP spec says the server returns the
/// highest version it supports that is compatible with the client. In
/// practice the surface we implement (initialize, tools/list, tools/call,
/// ping) has been stable across every shipped version, so echoing the
/// client's chosen version is correct and forward-compatible.
fn negotiate_protocol_version(client_version: Option<&str>) -> String {
    client_version
        .map(|s| s.to_string())
        .unwrap_or_else(|| DEFAULT_PROTOCOL_VERSION.to_string())
}

fn server_info() -> Value {
    json!({
        "name": "loom",
        "version": env!("CARGO_PKG_VERSION"),
    })
}

fn server_capabilities() -> Value {
    // We advertise only the `tools` capability. No resources, no prompts,
    // no sampling. Clients that check capabilities before calling methods
    // (notably M365 Copilot via Copilot Studio) will skip what we do not
    // advertise, which is what we want.
    json!({ "tools": {} })
}

// ---------------------------------------------------------------------------
// Tool catalog
// ---------------------------------------------------------------------------

/// Static catalog returned on `tools/list`. Each tool's `inputSchema` is a
/// JSON Schema that clients use to validate calls and render parameter UIs.
///
/// The tool descriptions are deliberately specific about the verbatim-content
/// invariant on `loom_learn` — MCP clients render these descriptions to the
/// model, and the model decides whether to call the tool. Embedding the
/// discipline rule here is a second layer on top of the per-client template
/// blocks under `templates/`.
fn tools_catalog() -> Value {
    json!({
        "tools": [
            {
                "name": "loom_learn",
                "description": "Ingest an interaction episode into Loom memory. Stores the episode and returns immediately; entity + fact extraction runs asynchronously in the offline pipeline. The `content` parameter MUST be verbatim — a transcript excerpt, quoted user prose, or raw source material. Never summaries, paraphrases, or reconstructions. The server hardcodes ingestion_mode to live_mcp_capture on this transport; clients cannot override it.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "content": {
                            "type": "string",
                            "description": "Verbatim episode text. Never a summary."
                        },
                        "source": {
                            "type": "string",
                            "description": "Free-form source identifier (e.g. claude-code, claude-desktop, chatgpt, github-copilot, m365-copilot, manual)."
                        },
                        "namespace": {
                            "type": "string",
                            "description": "Isolation boundary for this memory. Memory in one namespace is invisible to queries in another."
                        },
                        "occurred_at": {
                            "type": "string",
                            "format": "date-time",
                            "description": "ISO-8601 timestamp. Defaults to now."
                        },
                        "metadata": {
                            "type": "object",
                            "description": "Arbitrary source-specific metadata (session_id, channel, etc.)."
                        },
                        "participants": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "People involved in the interaction."
                        },
                        "source_event_id": {
                            "type": "string",
                            "description": "Deduplication key within the source system."
                        }
                    },
                    "required": ["content", "source", "namespace"]
                }
            },
            {
                "name": "loom_think",
                "description": "Compile a context package for a query. Runs the full online pipeline: intent classification → retrieval-profile selection → parallel retrieval → 4-dimension ranking → compilation. Returns the assembled context, its token count, and a compilation ID that can be used to audit the retrieval decisions in the Loom dashboard.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "The question to compile context for."
                        },
                        "namespace": {
                            "type": "string",
                            "description": "Which namespace to search."
                        },
                        "task_class_override": {
                            "type": "string",
                            "enum": ["debug", "architecture", "compliance", "writing", "chat"],
                            "description": "Force a task class, bypassing the classifier."
                        },
                        "target_model": {
                            "type": "string",
                            "description": "Target model identifier. Model names containing 'claude' get XML structured output; everything else gets JSON compact output."
                        }
                    },
                    "required": ["query", "namespace"]
                }
            },
            {
                "name": "loom_recall",
                "description": "Direct fact lookup for named entities. Bypasses intent classification and retrieval profiles. Returns raw facts with provenance — useful when you already know which entity you care about and want its current (or historical) relationships.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "entity_names": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Entity names to look up. Matched case-insensitively within the namespace."
                        },
                        "namespace": {
                            "type": "string",
                            "description": "Which namespace to search."
                        },
                        "include_historical": {
                            "type": "boolean",
                            "default": false,
                            "description": "Include superseded and soft-deleted facts."
                        }
                    },
                    "required": ["entity_names", "namespace"]
                }
            }
        ]
    })
}

// ---------------------------------------------------------------------------
// Dispatcher
// ---------------------------------------------------------------------------

/// JSON-RPC 2.0 MCP dispatcher. Single endpoint for all MCP wire-protocol
/// interactions with the Loom server.
pub async fn handle_mcp_rpc(
    State(state): State<AppState>,
    Json(req): Json<JsonRpcRequest>,
) -> Response {
    // Validate the JSON-RPC envelope. `jsonrpc` must be exactly "2.0".
    if req.jsonrpc != "2.0" {
        return json_error(
            req.id.unwrap_or(Value::Null),
            ERR_INVALID_REQUEST,
            "jsonrpc must be \"2.0\"",
        );
    }

    // Notifications (no id) must not receive a response per JSON-RPC 2.0.
    let is_notification = req.id.is_none();
    let id = req.id.clone().unwrap_or(Value::Null);

    match req.method.as_str() {
        "initialize" => {
            if is_notification {
                return empty_204();
            }
            let client_version = req
                .params
                .as_ref()
                .and_then(|p| p.get("protocolVersion"))
                .and_then(|v| v.as_str());
            let result = json!({
                "protocolVersion": negotiate_protocol_version(client_version),
                "capabilities": server_capabilities(),
                "serverInfo": server_info(),
            });
            json_ok(id, result)
        }
        "notifications/initialized" => empty_204(),
        "ping" => {
            if is_notification {
                return empty_204();
            }
            json_ok(id, json!({}))
        }
        "tools/list" => {
            if is_notification {
                return empty_204();
            }
            json_ok(id, tools_catalog())
        }
        "tools/call" => {
            if is_notification {
                return empty_204();
            }
            dispatch_tool_call(state, id, req.params).await
        }
        _ => {
            if is_notification {
                // Unknown notifications are silently ignored per JSON-RPC.
                return empty_204();
            }
            json_error(
                id,
                ERR_METHOD_NOT_FOUND,
                format!("method not found: {}", req.method),
            )
        }
    }
}

// ---------------------------------------------------------------------------
// tools/call dispatch
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct ToolCallParams {
    name: String,
    #[serde(default)]
    arguments: Value,
}

/// Dispatch a `tools/call` request to the matching per-tool handler.
///
/// MCP's tool-result convention is `{content: [{type: "text", text: "..."}]}`;
/// failures are signaled by adding `isError: true` to the result. The handler
/// response is serialized as pretty-printed JSON so clients that render tool
/// output (Claude Desktop, ChatGPT, Copilot) show the structured reply, not
/// a stringified blob.
async fn dispatch_tool_call(state: AppState, id: Value, params: Option<Value>) -> Response {
    let Some(params_value) = params else {
        return json_error(
            id,
            ERR_INVALID_PARAMS,
            "tools/call requires params: { name, arguments }",
        );
    };
    let params: ToolCallParams = match serde_json::from_value(params_value) {
        Ok(p) => p,
        Err(e) => {
            return json_error(
                id,
                ERR_INVALID_PARAMS,
                format!("invalid tools/call params: {e}"),
            );
        }
    };

    match params.name.as_str() {
        "loom_learn" => {
            let learn_req: LearnRequest = match serde_json::from_value(params.arguments) {
                Ok(r) => r,
                Err(e) => {
                    return tool_error(id, format!("invalid arguments for loom_learn: {e}"));
                }
            };
            match handle_loom_learn(State(state), Json(learn_req)).await {
                Ok(Json(resp)) => tool_ok(id, &resp),
                Err(err) => tool_error(id, err.to_string()),
            }
        }
        "loom_think" => {
            let think_req: ThinkRequest = match serde_json::from_value(params.arguments) {
                Ok(r) => r,
                Err(e) => {
                    return tool_error(id, format!("invalid arguments for loom_think: {e}"));
                }
            };
            match handle_loom_think(State(state), Json(think_req)).await {
                Ok(Json(resp)) => tool_ok(id, &resp),
                Err(err) => tool_error(id, err.to_string()),
            }
        }
        "loom_recall" => {
            let recall_req: RecallRequest = match serde_json::from_value(params.arguments) {
                Ok(r) => r,
                Err(e) => {
                    return tool_error(id, format!("invalid arguments for loom_recall: {e}"));
                }
            };
            match handle_loom_recall(State(state), Json(recall_req)).await {
                Ok(Json(resp)) => tool_ok(id, &resp),
                Err(err) => tool_error(id, err.to_string()),
            }
        }
        other => tool_error(id, format!("unknown tool: {other}")),
    }
}

// ---------------------------------------------------------------------------
// Response helpers
// ---------------------------------------------------------------------------

fn json_ok(id: Value, result: Value) -> Response {
    (StatusCode::OK, Json(JsonRpcResponse::result(id, result))).into_response()
}

fn json_error(id: Value, code: i32, message: impl Into<String>) -> Response {
    // JSON-RPC errors are carried inside a 200 response body per spec — the
    // HTTP layer only reports transport-level failures.
    (
        StatusCode::OK,
        Json(JsonRpcResponse::error(id, code, message)),
    )
        .into_response()
}

fn tool_ok<T: Serialize>(id: Value, payload: &T) -> Response {
    let text = serde_json::to_string_pretty(payload).unwrap_or_else(|e| {
        format!("{{\"error\": \"failed to serialize response: {e}\"}}")
    });
    json_ok(
        id,
        json!({
            "content": [
                { "type": "text", "text": text }
            ]
        }),
    )
}

fn tool_error(id: Value, text: impl Into<String>) -> Response {
    json_ok(
        id,
        json!({
            "content": [
                { "type": "text", "text": text.into() }
            ],
            "isError": true
        }),
    )
}

fn empty_204() -> Response {
    (StatusCode::NO_CONTENT, ()).into_response()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- JsonRpcResponse serialization --------------------------------------

    #[test]
    fn result_response_serializes_without_error_field() {
        let resp = JsonRpcResponse::result(json!(1), json!({"ok": true}));
        let s = serde_json::to_string(&resp).unwrap();
        assert!(s.contains("\"result\""));
        assert!(!s.contains("\"error\""));
        assert!(s.contains("\"jsonrpc\":\"2.0\""));
        assert!(s.contains("\"id\":1"));
    }

    #[test]
    fn error_response_serializes_without_result_field() {
        let resp = JsonRpcResponse::error(json!(1), ERR_METHOD_NOT_FOUND, "not found");
        let s = serde_json::to_string(&resp).unwrap();
        assert!(s.contains("\"error\""));
        assert!(!s.contains("\"result\""));
        assert!(s.contains("-32601"));
    }

    #[test]
    fn error_response_omits_data_field_when_absent() {
        let resp = JsonRpcResponse::error(json!(7), ERR_INVALID_PARAMS, "bad params");
        let s = serde_json::to_string(&resp).unwrap();
        assert!(!s.contains("\"data\""));
    }

    // -- Protocol version negotiation ---------------------------------------

    #[test]
    fn negotiates_to_client_version_when_provided() {
        assert_eq!(
            negotiate_protocol_version(Some("2025-11-25")),
            "2025-11-25"
        );
    }

    #[test]
    fn negotiates_to_default_when_client_silent() {
        assert_eq!(negotiate_protocol_version(None), DEFAULT_PROTOCOL_VERSION);
    }

    // -- Tool catalog shape -------------------------------------------------

    #[test]
    fn tools_catalog_lists_three_tools() {
        let catalog = tools_catalog();
        let tools = catalog["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 3);
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"loom_learn"));
        assert!(names.contains(&"loom_think"));
        assert!(names.contains(&"loom_recall"));
    }

    #[test]
    fn every_tool_has_input_schema_with_required_fields() {
        let catalog = tools_catalog();
        for tool in catalog["tools"].as_array().unwrap() {
            assert!(tool["name"].is_string(), "tool must name itself");
            assert!(tool["description"].is_string(), "tool must describe itself");
            let schema = &tool["inputSchema"];
            assert_eq!(schema["type"], "object");
            assert!(schema["properties"].is_object());
            assert!(schema["required"].is_array());
        }
    }

    #[test]
    fn loom_learn_schema_requires_content_source_namespace() {
        let catalog = tools_catalog();
        let learn = catalog["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "loom_learn")
            .unwrap();
        let required: Vec<&str> = learn["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        for field in &["content", "source", "namespace"] {
            assert!(required.contains(field), "loom_learn missing required: {field}");
        }
    }

    #[test]
    fn loom_think_schema_requires_query_and_namespace() {
        let catalog = tools_catalog();
        let think = catalog["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "loom_think")
            .unwrap();
        let required: Vec<&str> = think["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(required.contains(&"query"));
        assert!(required.contains(&"namespace"));
    }

    #[test]
    fn loom_recall_schema_requires_entity_names_and_namespace() {
        let catalog = tools_catalog();
        let recall = catalog["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "loom_recall")
            .unwrap();
        let required: Vec<&str> = recall["inputSchema"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(required.contains(&"entity_names"));
        assert!(required.contains(&"namespace"));
    }

    #[test]
    fn loom_learn_description_enforces_verbatim_invariant() {
        // ADR-005 lives on the doc string, the shipped templates, AND this
        // tool description — which clients surface to the model at the
        // point of tool choice. Regressing the language here weakens the
        // third line of defense.
        let catalog = tools_catalog();
        let learn = catalog["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "loom_learn")
            .unwrap();
        let description = learn["description"].as_str().unwrap();
        assert!(description.contains("verbatim"), "learn desc must say verbatim");
        assert!(
            description.to_lowercase().contains("never summar")
                || description.to_lowercase().contains("never reconstruct")
                || description.to_lowercase().contains("never paraphrase")
                || description.contains("Never summaries"),
            "learn desc must forbid summarization"
        );
    }

    // -- Server info --------------------------------------------------------

    #[test]
    fn server_info_carries_crate_name_and_version() {
        let info = server_info();
        assert_eq!(info["name"], "loom");
        assert_eq!(info["version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn capabilities_advertise_tools_only() {
        let caps = server_capabilities();
        assert!(caps.get("tools").is_some(), "must advertise tools");
        assert!(caps.get("resources").is_none(), "must not advertise resources");
        assert!(caps.get("prompts").is_none(), "must not advertise prompts");
        assert!(caps.get("sampling").is_none(), "must not advertise sampling");
    }

    // -- JSON-RPC error code constants --------------------------------------

    #[test]
    fn error_codes_match_jsonrpc_spec() {
        assert_eq!(ERR_PARSE, -32700);
        assert_eq!(ERR_INVALID_REQUEST, -32600);
        assert_eq!(ERR_METHOD_NOT_FOUND, -32601);
        assert_eq!(ERR_INVALID_PARAMS, -32602);
        assert_eq!(ERR_INTERNAL, -32603);
    }
}
