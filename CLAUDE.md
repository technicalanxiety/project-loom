# CLAUDE.md — project context for Claude Code sessions

This file is auto-loaded by Claude Code when working in this repo. It
documents the invariants and conventions you need to respect when making
changes.

For **integration setup** (how to wire Loom to Claude Code, Claude
Desktop, ChatGPT, GitHub Copilot, or M365 Copilot as an MCP memory
server), see [docs/clients/](docs/clients/). Each client has its own
guide.

---

## What Loom is

Project Loom is a PostgreSQL-native memory compiler for AI workflows.
It ingests episodes (verbatim interaction records), extracts entities
and facts, and compiles context packages for AI queries. It runs as
five Docker containers: loom-engine (Rust / axum / sqlx), loom-dashboard
(React SPA), PostgreSQL 17 with pgvector, Ollama for local LLM
inference, and Caddy as the reverse proxy.

This is **personal infrastructure**, not a maintained product. No PRs
reviewed, no issues answered. See [PROJECT-STANCE.md](PROJECT-STANCE.md).

## Load-bearing invariants — do not break

### Three-mode ingestion taxonomy

Every episode carries exactly one `ingestion_mode`:

| Mode | Path | Ranking coefficient |
|------|------|---------------------|
| `user_authored_seed` | `cli/loom-seed.py` posts user-authored markdown | 0.8 |
| `vendor_import` | `bootstrap/*.py` parsers post vendor-export excerpts with `parser_version` + `parser_source_schema` | 0.6 |
| `live_mcp_capture` | MCP `loom_learn` (server-hardcoded) or PostSession hook POST to `/api/learn` | 1.0 |

No `llm_reconstruction` mode exists. See
[ADR-004](docs/adr/004-ingestion-modes.md).

### Verbatim content invariant

Episode `content` must be verbatim — a transcript excerpt, a vendor
export excerpt, or user-authored prose. **Never** LLM summaries,
paraphrases, or reconstructions. The rule is not enforceable at runtime;
it's enforced by shipped templates, discipline, and the ADR. See
[ADR-005](docs/adr/005-verbatim-content-invariant.md). When editing
any template, docs, or client guide, preserve the "do not summarize,
paraphrase, or reconstruct" language — it's load-bearing.

### Namespace isolation

Memory in `namespace=a` is invisible to queries in `namespace=b`. No
cross-namespace retrieval. Every episode, entity, fact, and procedure
belongs to exactly one namespace.

### MCP server hardcodes `live_mcp_capture`

The MCP handler ignores whatever `ingestion_mode` a client sends and
sets it to `live_mcp_capture`. Clients cannot forge the mode through
this transport. Do not add an override path.

## Coding conventions for this repo

- **Rust**: stable, tokio + axum 0.8 + sqlx 0.8. The `ring` crypto
  provider is installed at startup (see `src/crypto.rs`) — do not
  pull in `aws-lc-sys`. Compile-time checked SQL via sqlx.
- **Dashboard**: React 19 + Vite 8 + TypeScript 6 + vitest 4 + biome 2.
- **No defensive coding beyond system boundaries.** Don't validate
  internal invariants that the type system or DB constraints already
  enforce. Validate user input at REST / MCP handlers and trust
  downstream.
- **No backwards-compat shims.** This is single-operator infrastructure;
  change the call sites, don't add compatibility layers.
- **Comments are the exception.** Only when the "why" is non-obvious —
  a hidden constraint, a subtle invariant, a workaround for a specific
  bug. No comments restating what the code does.

## Testing

```bash
# Unit + property tests
cd loom-engine && cargo nextest run

# Integration tests (need the test DB running)
docker compose -f docker-compose.test.yml up -d postgres-test
cd loom-engine && DATABASE_URL_TEST=postgres://loom_test:loom_test@localhost:5433/loom_test \
  cargo nextest run --profile integration

# Dashboard
cd loom-dashboard && npm test

# Lint
cd loom-engine && cargo clippy -- -D warnings && cargo fmt --check
cd loom-dashboard && npx biome check src/
```

Integration tests hit a real PostgreSQL — do not mock the DB. We got
burned once by mocked tests passing while a prod migration was broken.

## Where the pieces live

| Thing | Location |
|-------|----------|
| Engine source | `loom-engine/src/` |
| DB migrations | `loom-engine/migrations/` (001–013+) |
| Dashboard SPA | `loom-dashboard/src/` |
| Per-client integration guides | `docs/clients/` |
| Architecture Decision Records | `docs/adr/` |
| Bootstrap parsers (vendor imports) | `bootstrap/` |
| Client templates (configs, hooks, instructions) | `templates/` |
| Seed CLI | `cli/loom-seed.py` |
| Specs | `.kiro/specs/loom-memory-compiler/` |

## When in doubt

- **Integrating a new client?** Read [docs/clients/README.md](docs/clients/README.md)
  for the checklist. Every client needs a guide, a template, and
  (ideally) a bootstrap parser.
- **Touching ingestion?** Re-read [ADR-004](docs/adr/004-ingestion-modes.md)
  and [ADR-005](docs/adr/005-verbatim-content-invariant.md) before
  making changes.
- **Touching retrieval?** The four-dimension ranker (relevance 0.40,
  recency 0.25, stability 0.20, provenance 0.15) and the provenance
  coefficient (live 1.0, seed 0.8, vendor 0.6) are tuned. Don't
  retune without benchmark evidence.
