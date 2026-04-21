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
     conversation body, etc.) — never a rewritten summary.

The schema-assertion layer is deliberately strict. Vendor exports drift:
Claude.ai's account export has rotated field names between versions,
ChatGPT's export has added/dropped metadata blocks, Codex CLI rollouts
have changed delimiter formats. A parser that "tolerates missing fields"
quietly imports partial data and leaves the user with an authority model
full of silent gaps. Don't do that — fail loud, surface the error, let the
user decide to retry / skip / halt.

## Catalog

| Parser | Status | Asserted schema |
|--------|--------|-----------------|
| `claude_code_parser.py` | Reference implementation | `claude_code_jsonl_v1` |
| `claude_ai_parser.py` | TODO — see stub | `claude_ai_export_v2` |
| `chatgpt_parser.py`    | TODO — see stub | `chatgpt_export_v1` |
| `codex_cli_parser.py`  | TODO — see stub | `codex_cli_rollout_v1` |

Claude Code is the reference because its local JSONL is the most stable
export surface — the file format is driven by the CLI's own write path,
not a cloud product's feature flags.

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

```bash
export LOOM_URL="https://loom.yourdomain.com"
export LOOM_TOKEN="your-bearer-token"

python3 bootstrap/claude_code_parser.py \
    --session-dir ~/.claude/projects \
    --namespace my-project
```

See each parser's `--help` for its specific flags.
