# ADR-005: Verbatim Episode Content Invariant

## Status

Accepted

## Context

ADR-004 establishes three ingestion modes and rejects a fourth (LLM reconstruction). But the mode enum alone cannot prevent contamination: a client could call `loom_learn` with summarized content and claim `live_mcp_capture`, and Loom — having no reliable way to detect LLM-generated text post-hoc — would accept it as ground truth.

The failure mode this ADR addresses is specific and compounding:

1. User asks Claude to "summarize what we discussed about X over the last six months."
2. Claude, with no actual memory of past conversations, generates a confident-sounding summary that includes specifics that did not happen, conflates topics, invents decisions.
3. The user (or the model, at the end of a Desktop session) pipes that summary into `loom_learn`.
4. That content becomes an episode. Fact extraction runs on it. Facts derived from fabrication enter the authority hierarchy.
5. Next time the user queries the seeded-but-never-captured-live topic, Loom returns fabricated facts with no way for the user to detect the drift — ranking's provenance coefficient only helps when there are competing sources, and seeded-only topics have none.

In a multi-user product, this kind of contamination is isolated to one user and resettable. In a personal memory system used daily for years, it compounds: every fabricated episode becomes substrate for extractions, which become substrate for retrievals, which inform future conversations, which get captured back. The rot is silent and accumulating.

## Decision

Add a hard invariant to the Loom design: **the `content` field of every episode must be verbatim.**

Acceptable sources are exactly:

- A transcript or transcript excerpt from a conversation (Mode 3 live capture).
- An excerpt from a vendor export (Mode 2 bootstrap import).
- Human-authored prose written by a user for the purpose of seeding (Mode 1 seed).

Unacceptable sources include, but are not limited to: LLM summaries, LLM paraphrases, LLM "here's what we discussed" reconstructions, model-generated recaps at the end of sessions, synthetic episodes meant to bootstrap missing history.

This invariant is **trust-based** and cannot be enforced at runtime. LLM-generated text is not reliably detectable, and any detection heuristic would produce false positives on legitimate content. Enforcement is structural, not automatic:

- **MCP server**: hardcodes `ingestion_mode = live_mcp_capture` regardless of client input, so a client cannot launder LLM-reconstructed content through a mode claim. But the `content` field itself the server must accept as-given.
- **Shipped templates**: `templates/CLAUDE.md`, `templates/claude_desktop_projects_instructions.md`, and `templates/loom-capture.sh` all include bolded instructions to pass verbatim content only. The PostSession hook specifically reads raw JSONL from disk with no LLM in the loop.
- **Bootstrap parsers**: fail loud on schema drift; the `content` they post is a verbatim excerpt from the source export with no transformation step.
- **CLI seed tool**: treats the user's markdown files as opaque verbatim content — no preprocessing, no summarization.
- **User guide**: documents the invariant prominently, explains the failure mode in bold, describes the acceptable interview-style drafting pattern (user authors, Claude polishes, user reviews and approves) versus the unacceptable "Claude summarizes history" pattern.

This is the only defense. Documented rules plus client-side templates plus user discipline. There is no code to enforce it at ingestion time. The rule holds because every entry point (MCP, REST, bootstrap, CLI) is shipped with templates and tooling that make the verbatim path the default and the summary path hard.

## Consequences

### Positive

- The authority hierarchy (Episodes > Facts > Procedures) rests on evidence that actually happened. Facts extracted by the offline pipeline operate on primary sources, not model inference.
- Users who read their own episode content can audit the invariant directly. Grep, diff, pattern-match — the content is text, and the rule is "does this look like something someone or something actually said or wrote."
- The rejection of `llm_reconstruction` as an ingestion mode (ADR-004) and this verbatim invariant reinforce each other: there is no door for LLM-reconstructed content, even if a user tries to force one open.

### Negative

- The invariant cannot be enforced automatically. A determined or careless user can violate it, and Loom will accept the violation. Detection is possible only by reading the content manually.
- Users who expected to "seed Loom with what we discussed" via Claude Desktop summarization will find the documented pattern frustrating at first. The user guide explains why, but the friction is real.
- Shipped templates are a maintenance surface: if Claude Desktop or Claude Code changes its memory-file format, the templates need updating. Accepted cost.

### Neutral

- Interview-style drafting (user asks Claude to help them write a seed document through guided prompting, reviews and edits, then runs `loom-seed.py`) is explicitly allowed. The boundary is "who authored the content that went to `/api/learn`" — the user, after review, with Claude as scribe, is fine; Claude generating summaries the user pipes through unchanged is not.
- GitHub webhook episodes (PR comments, issue comments) are verbatim by construction — the webhook payload is the comment body, untransformed. No special handling needed.
- Extraction metrics' `extraction_model` field identifies which model processed an episode downstream. That is distinct from this ADR's invariant, which concerns the episode's input, not its extracted output.
