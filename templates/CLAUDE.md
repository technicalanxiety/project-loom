# Loom context — project CLAUDE.md template

Drop this block into a project's `CLAUDE.md` (or the appropriate Claude Code
memory file) when that project has Loom registered as an MCP server. It
establishes the discipline the model needs to use `loom_learn` correctly.

Replace or extend the block below — keep the wording about verbatim content
intact, that's the load-bearing part.

---

## Loom Context

Call `loom_think` before complex tasks to retrieve professional context for
this project's namespace.

Call `loom_learn` only with verbatim content from the current session:
quoted user text, quoted tool output, raw file contents the user pointed at.
**Never summarize, reconstruct, or paraphrase** before calling `loom_learn`.
If the user says "save this" and points at content, the `content` argument
must be the exact text they pointed at.

The PostSession hook (`loom-capture.sh`) handles exhaustive session capture
automatically. Your in-session `loom_learn` calls are only for the user's
explicit "save this" moments — not for end-of-session summaries, not for
recaps, not for "here's what we discussed" blocks.

If you are tempted to call `loom_learn` with a summary of the conversation
so far, don't. The hook has the raw JSONL and will ingest it verbatim. Your
summary would be an LLM reconstruction, which Loom treats as a first-class
authority violation.

## Namespace

Use the project's canonical namespace for every `loom_think` and `loom_learn`
call in this repository. Memory is strictly isolated per namespace — there
is no cross-namespace retrieval.

<!-- Replace with your actual namespace: -->
Namespace: `your-project-namespace`
