# ADR-004: Three-Mode Ingestion Taxonomy

## Status

Accepted

## Context

Every episode in Loom needs a provenance class. Without one, Stage 5 ranking cannot distinguish live-captured evidence from historical bootstrap from user-authored seed, and Stage 6 compilation cannot warn readers when a fact is backed only by seeded content. Worse, an implicit fourth category — LLM-generated summaries of past conversations — would, if allowed in through any door, poison the authority hierarchy at its root: facts derived from fabricated episodes become the only authority Loom has on topics that were seeded but never corroborated.

The amendment that motivated this ADR called out three real ingestion paths and one rejected path:

1. **Mode 1 — user-authored seed.** Markdown the user writes describing their domain. Ingested via a CLI. An LLM may help draft, but the user is the author.
2. **Mode 2 — vendor export import.** Parsed content from published vendor exports. Current parsers cover Claude Code local JSONL, Claude.ai / Claude Desktop account export, ChatGPT Data Controls export, and Microsoft 365 Copilot Purview audit export. Historical backfill where export fidelity allows. Surfaces without a published export (currently GitHub Copilot Chat) have no Mode 2 path and rely on Mode 3 live capture only.
3. **Mode 3 — live MCP capture.** Real-time verbatim capture via MCP-aware clients — Claude Code, Claude Desktop, ChatGPT Desktop (Developer Mode), GitHub Copilot (VS Code Agent mode), and M365 Copilot (declarative agents) — plus the Claude Code PostSession hook for exhaustive capture on that surface.
4. **Rejected — LLM reconstruction.** Summaries, paraphrases, "what we discussed" recaps. Confident-sounding generation, not recall. No door in the architecture.

Pre-amendment, all episodes flowed through the same `/api/learn` surface with no provenance tag. Ranking knew only evidence status (user_asserted > observed > extracted > inferred) — a semantic property of the extracted fact, not the episode it came from. A fact sourced from a seed document and a fact sourced from a live transcript were indistinguishable downstream.

## Decision

Add `ingestion_mode` to `loom_episodes` with a CHECK constraint pinning it to exactly three values: `user_authored_seed`, `vendor_import`, `live_mcp_capture`. Every episode carries its mode for the life of the row. Enforcement is layered:

- **Database**: CHECK constraint rejects invalid values. Separate constraint `chk_parser_fields_vendor_import` couples `parser_version` and `parser_source_schema` to vendor_import mode and forbids them elsewhere.
- **MCP server**: hardcodes `ingestion_mode = live_mcp_capture` at the handler boundary regardless of what the client sent. Clients cannot claim any other mode through MCP.
- **REST /api/learn**: requires `ingestion_mode` on every request (HTTP 400 if absent); validates parser-metadata coupling before insert.
- **Writers**: the CLI seed tool sets `user_authored_seed`, bootstrap parsers set `vendor_import` + their parser metadata, the PostSession hook sets `live_mcp_capture`.

The mode is read back at ranking time (via a provenance coefficient: live=1.0, seed=0.8, vendor=0.6) and at compilation time (via the sole-source flag — true iff the fact's only provenance is seed). For facts with multiple source episodes, retrieval pre-computes the MAX mode so one live-captured supporter wins.

No `llm_reconstruction` mode is added. That absence is load-bearing: it means there is no official door for LLM-generated content, and a future migration cannot accidentally (or expediently) open one without revisiting this ADR.

## Consequences

### Positive

- Ranking can finally express the authority hierarchy the system claims: live capture ground truth, seed authoritative but unverified, vendor import acknowledged as incomplete.
- The sole-source flag gives users a per-fact warning when compiled output rests on seed content alone — the only defense against the "seeded but never corroborated" failure mode.
- Dashboard views can show which namespaces are seed-only, making the authority gap visible rather than latent.
- The rejected fourth mode is called out explicitly rather than left as an implicit possibility; future contributors (and future-me) see it as a choice, not an oversight.

### Negative

- Writers must specify ingestion_mode explicitly — no silent default. Forgetting it means a 400. Intentional, but adds friction.
- The MAX-across-source-episodes computation requires a subquery in the fact-lookup retrieval path. Small cost; measurable on large facts.
- Schema CHECK constraints cannot be changed without a migration; adding a legitimate fourth mode later requires `ALTER TABLE ... DROP CONSTRAINT ... ADD CONSTRAINT`. This is the correct friction for a decision this load-bearing.

### Neutral

- GitHub webhook episodes are classified as `live_mcp_capture` because they match the semantic — verbatim real-time capture of an external event — even though no MCP client is involved. The name of the mode is about the semantic, not the transport.
- The `ingestion_mode` column does not enforce the verbatim-content invariant. That is a trust-based contract, documented in ADR-005, separately enforced via shipped templates.
