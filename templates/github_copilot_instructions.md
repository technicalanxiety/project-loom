# GitHub Copilot — Loom instructions

Drop this file into `.github/copilot-instructions.md` in your repo.
Copilot loads it on every Agent-mode turn in this workspace. If you
already have a `copilot-instructions.md`, merge the Loom block into it —
the verbatim-content sentence is the load-bearing part.

Loom is registered as an MCP server via `.vscode/mcp.json` (see
[vscode_mcp.example.json](vscode_mcp.example.json)) or the user profile
config. MCP tools are only visible in Copilot **Agent mode** — they do
not appear in Ask or Edit mode.

---

## Loom Context

Call `loom_think` before complex tasks in this repo to retrieve
professional context for this workspace's namespace.

Call `loom_learn` only with **verbatim content** from the current
session: quoted user text, quoted tool output, raw file contents the
user pointed at. **Never summarize, reconstruct, or paraphrase** before
calling `loom_learn`. If the user says "save this" and points at
content, the `content` argument must be the exact text they pointed at.

Do not call `loom_learn` for casual exchanges, clarifications, or
end-of-turn summaries. Copilot Chat has no PostSession equivalent, so
there is no automatic exhaustive-capture fallback on this client — if
the user asks you to capture something, capture exactly what they
pointed at, nothing extra.

Copilot prompts for confirmation before every MCP write. `loom_learn`
is a write; `loom_think` and `loom_recall` are reads.

## Namespace

Use the namespace below for every `loom_think` / `loom_learn` /
`loom_recall` call in this workspace. Memory is strictly isolated per
namespace.

<!-- Replace with your actual namespace: -->
Namespace: `your-project-namespace`
