# ADR-008: MCP Wire Protocol at `POST /mcp`

## Status

Accepted

## Context

Project Loom advertised itself as an MCP server from its first commit. The
README called it one, the spec called it one, the client guides in
`docs/clients/` told users how to register Loom at
`https://<host>/mcp/`. What actually shipped was three REST endpoints
under the `/mcp/` path prefix:

```
POST /mcp/loom_learn
POST /mcp/loom_think
POST /mcp/loom_recall
```

Each endpoint accepted a tool-specific JSON body and returned a
tool-specific JSON response. That works for `curl` and integration
tests. It does not work for MCP clients.

The Model Context Protocol is a JSON-RPC 2.0 protocol. A client speaks
it by POSTing JSON-RPC envelopes — `{"jsonrpc":"2.0","id":N,"method":...}` — to
a single endpoint and dispatching on `method`. The methods a real
client needs are:

- `initialize` — protocol handshake; returns server info, capabilities,
  and the negotiated protocol version.
- `notifications/initialized` — client tells the server it's ready.
- `ping` — keepalive.
- `tools/list` — the server advertises its tools with JSON Schema
  input schemas.
- `tools/call` — the server runs a tool and returns a result envelope.

Loom's per-tool REST endpoints do none of these. Attempting to connect
a real MCP client (Claude Desktop via `mcp-remote`, the error chain
that forced this ADR) reveals the gap immediately:

```
Received error (status 405): Streamable HTTP error
Fatal error: SseError: Invalid content type, expected "text/event-stream"
```

`mcp-remote` POSTs `{"jsonrpc":"2.0","method":"initialize",...}` to
`/mcp` and Caddy routes it to the static-file handler (there is no
`/mcp` route in the engine). 405 on POST, HTML response on the SSE
fallback. Not Loom's fault *per se* — we just never implemented the
protocol we claimed to support.

This ADR retroactively closes the gap.

## Decision

Add a JSON-RPC 2.0 dispatcher at `POST /mcp` that speaks the MCP wire
protocol. Keep the per-tool REST endpoints mounted. Share handler code
between the two surfaces so validation, dedup, ingestion-mode
enforcement, and pipeline invocation are bit-identical regardless of
transport.

The dispatcher lives in `loom-engine/src/api/mcp_rpc.rs`. It implements:

| Method | Behavior |
|--------|----------|
| `initialize` | Returns `protocolVersion` (negotiated from the client's request), `capabilities: {tools: {}}`, and `serverInfo: {name: "loom", version: <crate version>}`. |
| `notifications/initialized` | No response per JSON-RPC 2.0 (it's a notification — no `id`). HTTP 204. |
| `ping` | Empty `result: {}`. |
| `tools/list` | Returns the three Loom tools with full JSON Schema input schemas covering every field the REST endpoints accept. The `loom_learn` description carries the verbatim-content invariant (ADR-005) — it's the third line of defense alongside the per-client discipline templates and the DB CHECK constraints. |
| `tools/call` | Deserializes `params.arguments` into the tool-specific Rust type (`LearnRequest` / `ThinkRequest` / `RecallRequest`), calls the existing `handle_loom_*` function, and wraps the response in MCP's `content: [{type: text, text: ...}]` envelope. Tool-level failures use `isError: true`; JSON-RPC-level failures (malformed params, unknown tool) use canonical JSON-RPC error codes. |
| Any other method | JSON-RPC error `-32601 Method not found`. |

The Caddyfile matcher is updated from `handle /mcp/*` (which does NOT
match bare `/mcp`) to a named matcher `@mcp path /mcp /mcp/*` so both
the new dispatcher and the legacy REST paths route to the engine.

Notifications (requests with no `id`) get HTTP 204 and no body, per
the JSON-RPC 2.0 spec. Any other shape mismatch — `jsonrpc != "2.0"`,
missing `method`, malformed `params` — returns an HTTP 200 with a
JSON-RPC error envelope rather than an HTTP error, because the
transport request itself was well-formed and the JSON-RPC layer is
where the error belongs.

Session management (`Mcp-Session-Id` header) is deliberately omitted.
The dispatcher is stateless per request; any client that requires a
session id will generate and track its own, and stateless operation is
the right default for a single-operator deployment. If multi-tenancy
ever becomes a concern, session state lands in its own migration.

The dispatcher does not support the SSE fallback transport. Streamable
HTTP (POST with JSON response) is the only supported mode. Every
modern MCP client supports Streamable HTTP; the SSE fallback in
`mcp-remote` exists for servers that still only speak the older
transport, and implementing SSE in axum would be significant code for
approximately zero users.

## Alternatives considered

1. **Use the `rmcp` crate (Rust MCP SDK).** Rejected for now. The
   surface we actually implement (initialize, tools/list, tools/call,
   ping) is small enough to hand-roll in ~400 lines including tests,
   and hand-rolling keeps the engine's dependency graph lean.
   Revisitable if the SDK matures and we need resources / prompts /
   sampling capabilities.

2. **stdio-only bridge, no HTTP dispatcher.** Ship a Node script that
   speaks MCP-over-stdio to the client and translates to the existing
   REST endpoints. Rejected: it would work for Claude Desktop (which
   accepts stdio MCP servers via `claude_desktop_config.json`) but
   leaves ChatGPT Developer Mode, GitHub Copilot, M365 Copilot, and
   Claude Code's HTTP transport uncovered. Those four clients all
   speak HTTP MCP natively; a stdio-only shim would turn every one of
   them into a second-class citizen.

3. **Replace the per-tool REST endpoints with the JSON-RPC dispatcher
   exclusively.** Rejected. The REST endpoints are trivially
   callable from curl, shell scripts, and any stack without an MCP
   client library. Removing them would make one-off testing and
   integration scripts meaningfully harder for no protocol gain. The
   REST endpoints cost nothing (same handlers, same validation) and
   deleting them would be gratuitous.

4. **Implement the SSE transport.** Rejected. Streamable HTTP covers
   every shipped client we target. SSE would duplicate the routing
   surface, require a second handler, and complicate the
   request/response lifecycle (long-lived GET stream plus a POST
   endpoint for messages) for zero additional reachability.

## Consequences

### Positive

- The five first-class clients in `docs/clients/` (Claude Code, Claude
  Desktop, ChatGPT Desktop, GitHub Copilot, M365 Copilot) actually
  connect now. This is the unblocker the whole client-integration
  pivot rested on.
- `tools/list` is a real source of tool-surface truth. Clients render
  tool descriptions and input schemas from it, which means the
  verbatim-content invariant (ADR-005) now appears in the MCP client's
  own UI at the point where the model decides whether to call
  `loom_learn` — a third line of defense alongside the per-client
  discipline template and the DB-level ingestion-mode hardcoding.
- Dual-surface routing is transparent to handler code. Any future tool
  added to the project lands in both the JSON-RPC dispatcher's
  tools/list catalog and as a REST endpoint by adding the route; no
  protocol-layer changes required.
- Sharing the handler code means the same request validation, the
  same dedup logic, and the same `ingestion_mode = live_mcp_capture`
  hardcoding apply regardless of transport. No "bypass the discipline
  by using the REST endpoint" attack surface.

### Negative

- Two transports means two test matrices. The 13 integration tests in
  `tests/mcp_rpc.rs` cover the dispatcher path; the property and
  integration tests in `tests/property_mcp_endpoints.rs` and
  `tests/api_rest_dashboard.rs` cover the REST path. Maintained
  separately until/unless we consolidate on a shared request fixture
  layer.
- Protocol version negotiation is intentionally permissive — we echo
  whatever version the client sent (defaulting to `2025-06-18` when
  omitted). This is correct for the tiny method surface we implement
  but carries a small forward-compat risk: if MCP's `tools/call`
  envelope shape changes in a future spec version, we'd keep returning
  the old shape for new clients. Acceptable for now; the review
  trigger is an MCP spec revision that changes any of the five
  methods we implement.
- The `Mcp-Session-Id` header is ignored. Clients that require a
  session id on subsequent requests must generate one client-side.
  None of the five first-class clients exhibit this requirement at
  time of writing.

### Neutral

- The dispatcher advertises only the `tools` capability. `resources`,
  `prompts`, and `sampling` are unimplemented. Clients that check
  capabilities before calling corresponding methods will correctly
  skip those code paths. Adding them later is additive, not breaking.
- `tools/call` responses are serialized as pretty-printed JSON inside
  the MCP text content block. This is the convention every major MCP
  client renders well. A future iteration could return structured
  content blocks (the spec allows `type: "resource"` etc.), but the
  text path is the interoperable default.
- JSON-RPC error code `-32700 Parse error` is defined in code but not
  currently emitted (axum's JSON extractor rejects malformed bodies
  before the dispatcher sees them). Left as a named constant for
  future use and as reference for readers.
