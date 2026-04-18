---
description: MCP protocol learning context — JSON-RPC patterns, tool model, and implementation guidance
inclusion: fileMatch
fileMatchPattern: "loom-engine/src/api/mcp.rs,loom-engine/src/types/mcp.rs"
---

# MCP Protocol Learning Context

Jason is actively learning the Model Context Protocol. When working on MCP-related code,
provide extra context and explain protocol patterns.

## MCP Overview

The Model Context Protocol (MCP) is a JSON-RPC 2.0 based protocol for AI tools to
communicate with context providers. Loom implements an MCP server that exposes three tools:

- **loom_think**: Online pipeline — classify intent, retrieve relevant memory, compile context.
- **loom_learn**: Offline pipeline — ingest a new episode for processing.
- **loom_recall**: Direct retrieval — fetch specific entities, facts, or episodes by ID/query.

## JSON-RPC 2.0 Basics

Every MCP message is a JSON-RPC 2.0 request or response:

```json
// Request
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "loom_think",
    "arguments": {
      "query": "What authentication pattern does project-x use?",
      "namespace": "project-x"
    }
  }
}

// Success response
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "content": [
      {
        "type": "text",
        "text": "..."
      }
    ]
  }
}

// Error response
{
  "jsonrpc": "2.0",
  "id": 1,
  "error": {
    "code": -32602,
    "message": "Invalid namespace"
  }
}
```

## MCP Server Implementation Pattern (axum)

```rust
/// Handle incoming MCP JSON-RPC requests.
/// Routes to the appropriate tool handler based on the method and tool name.
async fn handle_mcp_request(
    State(state): State<AppState>,
    Json(request): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    match request.method.as_str() {
        "tools/list" => list_tools(),
        "tools/call" => call_tool(&state, &request.params).await,
        "initialize" => handle_initialize(),
        _ => json_rpc_error(request.id, -32601, "Method not found"),
    }
}
```

## Key MCP Concepts

- **Tools**: Functions the AI can call. Each has a name, description, and JSON Schema for parameters.
- **Resources**: Data the AI can read (not used in Loom MVP).
- **Prompts**: Template prompts (not used in Loom MVP).
- **Transport**: HTTP with SSE for streaming, or stdio for local tools. Loom uses HTTP.

## Error Codes (JSON-RPC 2.0)

- `-32700`: Parse error
- `-32600`: Invalid request
- `-32601`: Method not found
- `-32602`: Invalid params
- `-32603`: Internal error
- `-32000` to `-32099`: Server-defined errors (use for Loom-specific errors)

## Testing MCP Endpoints

```bash
# List available tools
curl -X POST http://localhost:8080/mcp \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOOM_BEARER_TOKEN" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'

# Call loom_think
curl -X POST http://localhost:8080/mcp \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOOM_BEARER_TOKEN" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"loom_think","arguments":{"query":"...","namespace":"default"}}}'
```
