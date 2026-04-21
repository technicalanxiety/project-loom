# Loom bootstrap

Vendor-export parsers that seed historical memory into Loom as Mode 2
(`vendor_import`) episodes. Each parser is a thin Python script that:

1. Asserts a pinned schema on its input export. On mismatch, it fails
   loudly with the specific field name. **No best-effort fallback.**
2. Emits one `POST /api/learn` per episode with:
   - `ingestion_mode: "vendor_import"`
   - `parser_version: "<name>@<semver>"`
   - `parser_source_schema: "<export_schema_name>"`
   - `content`: verbatim excerpt from the export (transcript message,
     conversation body, audit record, etc.) — never a rewritten summary.

The schema-assertion layer is deliberately strict. Vendor exports drift:
Claude.ai's account export has rotated field names between versions,
ChatGPT's export has added/dropped metadata blocks, Microsoft's Purview
audit schema has evolved through GA cadences. A parser that "tolerates
missing fields" quietly imports partial data and leaves the user with
an authority model full of silent gaps. Don't do that — fail loud,
surface the error, let the user decide to retry / skip / halt.

## Catalog

| Parser | Asserted schema | Supported client(s) |
|--------|-----------------|---------------------|
| [`claude_code_parser.py`](claude_code_parser.py) | `claude_code_jsonl_v1` | Claude Code (local session JSONL) |
| [`claude_ai_parser.py`](claude_ai_parser.py) | `claude_ai_export_v2` | Claude Desktop / Claude.ai (account data export) |
| [`chatgpt_parser.py`](chatgpt_parser.py) | `chatgpt_export_v1` | ChatGPT (Data Controls export `conversations.json`) |
| [`github_copilot_parser.py`](github_copilot_parser.py) | — (stub) | GitHub Copilot — no export available; stub exits with pointer to live-capture path |
| [`m365_copilot_parser.py`](m365_copilot_parser.py) | `m365_copilot_audit_v1` | Microsoft 365 Copilot (Purview Content Search / eDiscovery export) |

Claude Code is the reference because its local JSONL is the most stable
export surface — the file format is driven by the CLI's own write path,
not a cloud product's feature flags. The other parsers target cloud
export schemas and will need their pinned versions bumped when the
vendor rotates fields.

For the per-client integration context each parser belongs to, see
[`docs/clients/`](../docs/clients/).

## Degraded-mode contract

When a schema assertion fails, the parser:

1. Prints a single-line error naming the specific field (e.g.
   `claude_ai_export_v2: missing conversations[].chat_messages[].created_at`).
2. Exits non-zero.
3. Does not retry, does not silently skip, does not best-effort parse.

Previously-ingested episodes from the same export remain in Loom — the
`content_hash` idempotency check protects re-runs after a vendor
regenerates the export.

## Running a parser

Every parser reads `LOOM_URL` and `LOOM_TOKEN` from the environment and
takes `--namespace` as a required flag. Input flags are parser-specific
(`--session-dir`, `--export`, `--export-dir`).

```bash
export LOOM_URL="https://loom.yourdomain.com"
export LOOM_TOKEN="your-bearer-token"

# Claude Code local JSONL sessions
python3 bootstrap/claude_code_parser.py \
    --session-dir ~/.claude/projects \
    --namespace my-project

# Claude.ai / Claude Desktop account export
python3 bootstrap/claude_ai_parser.py \
    --export ~/Downloads/claude-data-export/conversations.json \
    --namespace my-project

# ChatGPT Data Controls export
python3 bootstrap/chatgpt_parser.py \
    --export ~/Downloads/chatgpt-data-export/conversations.json \
    --namespace my-project

# Microsoft 365 Copilot Purview export bundle
python3 bootstrap/m365_copilot_parser.py \
    --export-dir ~/Downloads/m365-purview-export/ \
    --namespace my-tenant
```

See each parser's `--help` for its specific flags.
