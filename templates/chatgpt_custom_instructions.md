# ChatGPT — Loom Custom Instructions

Paste the block below into ChatGPT **Settings → Personalization → Custom
instructions** (global), or into **Projects → [your project] → Instructions**
(project-scoped, preferred if you have Loom scoped per Project).

This block enforces the verbatim-content invariant on the ChatGPT capture
path. If you edit it, preserve the "Do not summarize, paraphrase, or
reconstruct" sentence — it is load-bearing for the authority model.

On paid ChatGPT workspaces with **Developer Mode** enabled, Loom is
registered as a custom MCP app and these instructions govern how the
model calls it. On Free / Plus / Pro (no Developer Mode), Loom tools
are not available — use the vendor import path instead.

---

## Loom Context Integration

Call `loom_think` before responding to substantive questions about
architecture, debugging, compliance, or ongoing projects in my namespaces.

Call `loom_learn` when I explicitly ask you to save something, or when I
say "remember this," "save that," "capture this," or similar. Pass the
verbatim text I am pointing at as the `content` parameter.

Do not summarize, paraphrase, or reconstruct. Do not pass your own
descriptions of what was discussed. Only pass raw content authored by me
or quoted verbatim from source material I provided.

Do not call `loom_learn` for casual exchanges, clarifications, small talk,
or end-of-conversation summaries.

When invoking `loom_learn`, omit the `ingestion_mode` field — the Loom
server will hardcode it to `live_mcp_capture` at the MCP boundary.

If a tool call is a write (modifying state on the Loom server), ChatGPT
will show a confirmation dialog. `loom_learn` is a write; `loom_think`
and `loom_recall` are reads.

<!-- Replace with your actual namespace: -->
Namespace: `your-project-namespace`
