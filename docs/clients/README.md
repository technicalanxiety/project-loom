# Loom client integrations

Loom is transport-agnostic. Any surface that can speak HTTP to `/mcp/*`
or `/api/learn` can drive it. This folder documents the five clients I
actively use and ship templates + parsers for; each has its own guide.

## Clients

| Client | MCP live capture | Vendor import | Guide |
|--------|------------------|---------------|-------|
| Claude Code | Yes — HTTP transport + PostSession hook for exhaustive capture | Yes — local JSONL session files | [claude-code.md](claude-code.md) |
| Claude Desktop | Yes — HTTP transport; Projects instructions for discipline | Yes — Claude.ai account export | [claude-desktop.md](claude-desktop.md) |
| ChatGPT Desktop | Yes — Developer Mode apps (Business / Enterprise / Edu, beta) | Yes — Data Controls export (`conversations.json`) | [chatgpt-desktop.md](chatgpt-desktop.md) |
| GitHub Copilot (VS Code) | Yes — `.vscode/mcp.json` or user profile config, Agent mode | No — Copilot Chat does not ship a conversation export | [github-copilot.md](github-copilot.md) |
| M365 Copilot | Yes — Copilot Studio declarative agent wiring `loom_think` / `loom_learn` as MCP tools | Yes — M365 compliance export of Copilot interactions | [m365-copilot.md](m365-copilot.md) |

All five clients are first-class. There is no "primary" target — pick the
one that matches where you already work, and add others as your workflow
spans surfaces.

## What every client guide covers

Each guide is structured the same way so you can skim for the path you
need:

1. **Overview** — what this client is and what Loom gives it.
2. **Live MCP capture** — how to register Loom as an MCP server, auth, and
   where the client writes the config.
3. **Discipline block** — the template that enforces the verbatim-content
   invariant ([ADR-005](../adr/005-verbatim-content-invariant.md)). Drop
   it into whatever system-prompt / custom-instructions / Projects field
   the client provides.
4. **Vendor import** — how to produce the export, what schema the
   bootstrap parser asserts, and how to run it.
5. **Known gaps** — things the client cannot do (no export, no hook, no
   namespace context) so you know what to work around.

## The ingestion-mode mapping

Each path maps to exactly one of the three ingestion modes. No client can
forge a mode — the server hardcodes `live_mcp_capture` on the MCP path
and validates mode explicitly on `/api/learn`.

| Path | Ingestion mode | Ranking coefficient |
|------|----------------|---------------------|
| MCP `loom_learn` (any client) | `live_mcp_capture` (server-hardcoded) | 1.0 |
| PostSession / live hook POSTing to `/api/learn` | `live_mcp_capture` (client-asserted, trusted) | 1.0 |
| Bootstrap parser POSTing vendor export | `vendor_import` (client-asserted, validated) | 0.6 |
| `cli/loom-seed.py` POSTing user markdown | `user_authored_seed` | 0.8 |

See [ADR-004](../adr/004-ingestion-modes.md) for the full taxonomy and
[ADR-005](../adr/005-verbatim-content-invariant.md) for the verbatim
content invariant every guide is enforcing.

## Adding a new client

If you want to wire a different surface, the checklist is:

1. Does it support MCP over HTTP? If yes, register `https://<loom>/mcp/`
   with a bearer token — that is the live-capture path.
2. Does it emit a structured export? If yes, write a bootstrap parser
   under `bootstrap/` that asserts a pinned schema, posts verbatim
   excerpts as `vendor_import`, and fails loud on drift. Model on
   [bootstrap/claude_code_parser.py](../../bootstrap/claude_code_parser.py).
3. Ship a discipline template under `templates/` that tells the model to
   pass verbatim content only — the verbatim invariant is what keeps the
   authority hierarchy honest across surfaces.
