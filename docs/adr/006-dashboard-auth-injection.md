# ADR-006: Caddy-Injected Bearer Token for /dashboard/api/*

## Status

Accepted

## Context

The operational dashboard is a React SPA served as static files from `/` by Caddy. It makes XHR calls to `/dashboard/api/*` endpoints on the loom-engine for all its data — pipeline health, compilation traces, entity graph, parser health, ingestion distribution, and so on. Every one of those endpoints sits behind the same bearer-token middleware that protects the MCP and REST surfaces.

External clients of the MCP and REST paths (Claude Code, Claude Desktop, bootstrap parsers, the PostSession hook, the CLI seed tool) carry the bearer token themselves. They read it from their local env or config.

The browser cannot do that. When the SPA calls `fetch('/dashboard/api/health')`, there is no token in the fetch options and the request lands at the engine with no `Authorization` header. The engine's auth middleware correctly returns 401 — the behavior observed in logs as `auth rejected: missing Authorization header path=/dashboard/api/health`.

Three ways to fix it:

1. **Build a login flow.** Dashboard prompts for the token on first load, stores in `localStorage`, attaches to every `fetch` call. Requires a login page, token-storage logic, handling of `localStorage` unavailability, CSRF protection if the stored token is used from anywhere else, and some way to recover from a wrong or expired token.
2. **Reverse-proxy-injected header.** Caddy adds the bearer token to requests as they flow through. The browser never sees the token and never needs to manage one. One line in the Caddyfile.
3. **Different auth scheme for dashboard endpoints.** Cookie-based session, same-origin check, separate dashboard-only token, etc. More moving parts than either of the above.

## Decision

Caddy injects `Authorization: Bearer ${LOOM_BEARER_TOKEN}` on `/dashboard/api/*` requests only. The token flows `.env` → `docker-compose.yml` environment → Caddy env → Caddyfile expansion → outgoing header. `/mcp/*` and `/api/*` paths are untouched: external clients continue to supply their own token.

```caddyfile
handle /dashboard/api/* {
    reverse_proxy loom-engine:8080 {
        header_up Authorization "Bearer {env.LOOM_BEARER_TOKEN}"
    }
}
```

The engine's auth middleware is unchanged. Whether the `Authorization` header was set by the browser, by Caddy, or by an external client, the validation logic is the same: constant-time compare against the expected token, 401 on any mismatch.

## Consequences

### Positive

- Zero frontend auth code. No login page, no token-storage, no error recovery flow, no edge cases around `localStorage` being disabled.
- One token (`LOOM_BEARER_TOKEN`) governs every surface. Rotate it in `.env`, restart the stack, everything updates.
- External clients are unaffected — they still supply their own header on `/mcp/*` and `/api/*`. The injection is narrowly scoped.
- Keeps the personal-infrastructure footprint small. A login flow would be more code to maintain for a one-user deployment than it's worth.

### Negative

- **CSRF exposure on `/dashboard/api/*`.** Any page loaded in the user's browser can issue `fetch('https://loom.yourdomain.com/dashboard/api/...')`, and Caddy will dutifully attach the auth header. For read-only endpoints that is an information-disclosure risk; for the two write endpoints (`/conflicts/:id/resolve`, `/predicates/candidates/:id/resolve`) that is a state-change risk. Mitigated by:
  - Loom being bound to localhost or a trusted network per PROJECT-STANCE.md.
  - No cross-origin access being configured — an attacker would need the user to visit a malicious page *and* that page would need to know or guess the loom URL.
  - The two write endpoints requiring structured JSON bodies with specific UUIDs; a blind CSRF would need to know the target resource ID.
- Deploying to a hostile network (exposing Loom on a public interface with real CORS opened up) invalidates the trade-off. A fork doing that should replace this pattern with a proper login flow or an origin-restricted session cookie.
- Token rotation now requires restarting Caddy too, not just the engine — the env-expanded value is read at Caddy startup.

### Neutral

- The engine's bearer-token middleware is untouched. Nothing prevents a future fork from replacing the Caddy injection with any other auth mechanism at the proxy layer; the engine's contract is "produce a valid `Authorization` header however you like."
- Logs record the path but never the token value (`auth.rs` explicitly redacts it). Injected tokens are invisible in request logs at both Caddy and the engine.
