# Loom client templates

Templates users drop into their own environment to wire Loom into a
client surface. Each file is self-contained; copy the one you need and
edit the placeholder values (`LOOM_TOKEN`, `https://loom.yourdomain.com`,
namespace, etc.).

For the per-client integration walkthroughs that reference these
templates, see [docs/clients/](../docs/clients/).

## Claude Code

| File | Purpose |
|------|---------|
| `CLAUDE.md` | Discipline block for project-scoped `CLAUDE.md`. Enforces the verbatim-content invariant on in-session `loom_learn` calls. |
| `loom-capture.sh` | PostSession hook. Reads raw session JSONL and POSTs it verbatim to `/api/learn` with `ingestion_mode=live_mcp_capture`. |

## Claude Desktop

| File | Purpose |
|------|---------|
| `claude_desktop_config.example.json` | Reference `claude_desktop_config.json` snippet for MCP registration. |
| `claude_desktop_projects_instructions.md` | Projects instructions block. Tells the model to call `loom_learn` only with verbatim content. |

## ChatGPT (Desktop / web, paid workspaces with Developer Mode)

| File | Purpose |
|------|---------|
| `chatgpt_custom_instructions.md` | Custom instructions block (global or Projects-scoped). Mirrors the verbatim-content discipline. |

## GitHub Copilot (VS Code)

| File | Purpose |
|------|---------|
| `vscode_mcp.example.json` | Reference `.vscode/mcp.json` (or user profile config) for Copilot MCP registration. Note the root key is `servers`, not `mcpServers`. |
| `github_copilot_instructions.md` | Block for `.github/copilot-instructions.md`. Copilot loads this on every Agent-mode turn. |

## Microsoft 365 Copilot

| File | Purpose |
|------|---------|
| `m365_copilot_agent_manifest.example.json` | Declarative-agent manifest skeleton wiring Loom as an MCP action. Use with the Microsoft 365 Agents Toolkit. |
| `m365_copilot_instructions.md` | Agent `instructions` block. The declarative agent loads this on every invocation. |

## The verbatim-content invariant

Every template enforces the same architectural rule, by documentation:

> The `content` parameter of `loom_learn` (and every episode posted to
> `/api/learn`) must be verbatim. Transcript excerpts, vendor export
> excerpts, user-authored prose. **Never** LLM summarization, paraphrase,
> or reconstruction.

Loom cannot detect violations at runtime. Enforcement lives in these
templates, the MCP server's hardcoded `live_mcp_capture` mode, and user
discipline. See [docs/adr/005-verbatim-content-invariant.md](../docs/adr/005-verbatim-content-invariant.md)
for the full rationale.
