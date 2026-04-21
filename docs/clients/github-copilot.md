# GitHub Copilot (VS Code) + Loom

GitHub Copilot Chat in VS Code became GA for MCP in VS Code 1.102 (July
2025) and is the most enterprise-ready MCP client at time of writing —
OAuth, sandboxing, Settings Sync, a curated MCP marketplace. Loom
registers as an MCP server in `.vscode/mcp.json` (workspace) or the user
profile config.

Copilot Chat does **not** ship a conversation export. There is no
`conversations.json` equivalent for backfilling chats. Live MCP capture
is the only path on this client.

## Live MCP capture

Copilot's MCP config key is **`servers`**, not `mcpServers`. This is
the number-one setup mistake when copy-pasting from Claude Desktop
configs.

Workspace config — commit this into your repo to share with the team:

```json
// .vscode/mcp.json
{
  "servers": {
    "loom": {
      "type": "http",
      "url": "https://loom.yourdomain.com/mcp/",
      "headers": {
        "Authorization": "Bearer ${input:loom_token}"
      }
    }
  },
  "inputs": [
    {
      "type": "promptString",
      "id": "loom_token",
      "description": "Loom bearer token",
      "password": true
    }
  ]
}
```

A reference copy lives at [templates/vscode_mcp.example.json](../../templates/vscode_mcp.example.json).

User-scoped config (applies across all workspaces): Command Palette →
**MCP: Open User Configuration** → paste the same `servers` block.

Then:

- **Agent mode is required.** MCP tools are invisible in Ask or Edit
  mode. Open Copilot Chat, click the mode dropdown, select **Agent**.
- Run **MCP: List Servers** from the Command Palette to verify `loom`
  is running.
- Copilot prompts for confirmation before calling any MCP tool. Click
  **Allow** (once per tool, or "Always" to persist).

The server hardcodes `ingestion_mode = live_mcp_capture` at the MCP
boundary, same as any other transport.

### Organizations

Enterprises / organizations can enable or disable MCP for members
globally via the **MCP servers in Copilot** policy. It's disabled by
default — if Loom doesn't appear, check the org policy.

## Discipline block — Copilot custom instructions

VS Code Copilot supports per-workspace custom instructions via
`.github/copilot-instructions.md`. Paste the discipline block from
[templates/github_copilot_instructions.md](../../templates/github_copilot_instructions.md)
into it. Copilot loads this file on every Agent-mode turn.

Same verbatim-content invariant as every other client: no summaries, no
paraphrases, no "here's what we discussed." See
[ADR-005](../adr/005-verbatim-content-invariant.md).

## Vendor import

**There is no vendor import path for Copilot Chat.** GitHub does not
publish a conversation export for Copilot Chat in VS Code, and the chat
state is held in the editor's extension storage with no stable export
schema to assert against.

If you need to backfill historical coding work, the two usable
substitutes are:

1. Run the same sessions through Claude Code (which does have a JSONL
   export path). The Claude Code parser is the analogue.
2. Seed a namespace with user-authored markdown describing the
   decisions / patterns / people from the historical work, via
   [cli/loom-seed.py](../../cli/loom-seed.py). This is Mode 1
   (`user_authored_seed`), explicitly authored by you — not an LLM
   summary of past Copilot chats.

A stub at [bootstrap/github_copilot_parser.py](../../bootstrap/github_copilot_parser.py)
exists as a placeholder; it exits non-zero with a message pointing at
this guide. If GitHub ships a Copilot Chat export in the future, the
parser will be filled in.

## Namespaces

Copilot Chat in VS Code has access to the workspace folder. Bake the
namespace into the MCP config's `url` query string if you want it
fixed per-workspace, or into `.github/copilot-instructions.md` so the
model includes it in `loom_learn` / `loom_think` calls.

## Known gaps

- **No export, no hook.** Live capture is "what the model calls
  `loom_learn` with during the conversation." There is no exhaustive
  fallback like the Claude Code PostSession hook.
- **Agent mode only.** Users in Ask / Edit mode see no Loom tools.
- **Tool confirmation dialogs.** Copilot prompts before every write.
  Expected and appreciated — this is the enterprise posture — but it
  does mean drive-by `loom_learn` calls cost a click.
- **Org policy gate.** Enterprise Copilot disables MCP by default.
