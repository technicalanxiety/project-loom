# Claude Desktop — Loom Projects instructions

Paste the block below into the Projects "Instructions" field in Claude
Desktop (Projects → [your project] → Edit project). It is the reference
instruction set shipped with Loom and enforces the verbatim-content
invariant on the Desktop capture path.

If you edit this block, preserve the "Do not summarize, paraphrase, or
reconstruct" sentence — it is load-bearing for the authority model.

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
