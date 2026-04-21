# Loom client templates

Templates users drop into their own environment to wire Loom into a client
surface. Each file is self-contained; copy the one you need and edit the
placeholder values (`LOOM_TOKEN`, `https://loom.yourdomain.com`, etc.).

## Contents

| File | Purpose |
|------|---------|
| `CLAUDE.md` | Discipline block for users running Claude Code against Loom. Drop into a project's `CLAUDE.md`. |
| `claude_desktop_projects_instructions.md` | Projects instructions for Claude Desktop — tells the model to call `loom_learn` only with verbatim content. |
| `claude_desktop_config.example.json` | Reference `claude_desktop_config.json` snippet for MCP registration. |
| `loom-capture.sh` | PostSession hook for Claude Code that POSTs each session's raw JSONL to `/api/learn` with `ingestion_mode=live_mcp_capture`. |

## The verbatim content invariant

Every template below enforces the same architectural rule, by documentation:

> The `content` parameter of `loom_learn` (and every episode posted to
> `/api/learn`) must be verbatim. Transcript excerpts, vendor export
> excerpts, user-authored prose. **Never** LLM summarization, paraphrase,
> or reconstruction.

Loom cannot detect violations at runtime. Enforcement lives in these
templates, the MCP server's hardcoded `live_mcp_capture` mode, and user
discipline. See `docs/adr/005-verbatim-content-invariant.md` for the full
rationale.
