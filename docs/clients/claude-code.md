# Claude Code + Loom

Claude Code is Anthropic's terminal / IDE harness. It ships native MCP
support, can run shell hooks on session lifecycle events, and writes
session transcripts to local JSONL — all three of which Loom uses.

## Live MCP capture

Register Loom as an MCP server in Claude Code:

```bash
export LOOM_BEARER_TOKEN="your-token-here"

claude mcp add loom-memory \
  --transport http \
  --url https://localhost/mcp/ \
  --header "Authorization: Bearer $LOOM_BEARER_TOKEN"
```

Or edit `~/.claude/mcp_servers.json` (or the project-level
`.claude/mcp_servers.json`) directly:

```json
{
  "loom-memory": {
    "transport": "http",
    "url": "https://localhost/mcp/",
    "headers": {
      "Authorization": "Bearer your-token-here",
      "Content-Type": "application/json"
    }
  }
}
```

Verify:

```bash
curl -s https://localhost/api/health | jq '.status'   # "ok"

curl -s -X POST https://localhost/mcp/loom_recall \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOOM_BEARER_TOKEN" \
  -d '{"entity_names": ["test"], "namespace": "default"}' | jq
```

The server hardcodes `ingestion_mode = live_mcp_capture` at the MCP
boundary. A client cannot claim any other mode through this transport.

## Discipline block — project CLAUDE.md

Drop [templates/CLAUDE.md](../../templates/CLAUDE.md) into the project's
`CLAUDE.md` (or append to it). It establishes the rule Claude needs to
follow on this transport: `loom_learn` is only for verbatim content, and
exhaustive capture is the hook's job, not the model's.

## Exhaustive live capture — PostSession hook

Claude Code can run a shell hook after every session. The shipped hook
at [templates/loom-capture.sh](../../templates/loom-capture.sh) reads
the raw session JSONL from disk and POSTs it to `/api/learn` verbatim
with `ingestion_mode = live_mcp_capture`. No LLM inference at the
boundary — the transcript is the transcript.

Install:

```bash
cp templates/loom-capture.sh ~/.claude/hooks/loom-capture.sh
chmod +x ~/.claude/hooks/loom-capture.sh
```

Add to `~/.claude/settings.json`:

```json
{
  "hooks": {
    "PostSession": [
      { "matcher": "", "command": "~/.claude/hooks/loom-capture.sh" }
    ]
  }
}
```

Export the env vars (the hook refuses to run without them):

```bash
export LOOM_URL="https://localhost"
export LOOM_TOKEN="your-bearer-token"
export LOOM_NAMESPACE="your-project-namespace"
```

The hook derives its `source_event_id` from the session file's path +
mtime, so re-running it on the same session is a no-op at the
content-hash layer.

## Vendor import — bootstrap from local JSONL

If you have Claude Code sessions on disk that predate the hook, backfill
them with [bootstrap/claude_code_parser.py](../../bootstrap/claude_code_parser.py).
It walks `~/.claude/projects/*.jsonl`, asserts the `claude_code_jsonl_v1`
schema on each record, and POSTs each session as one `vendor_import`
episode.

```bash
export LOOM_URL="https://localhost"
export LOOM_TOKEN="your-bearer-token"

python3 bootstrap/claude_code_parser.py \
    --session-dir ~/.claude/projects \
    --namespace my-project
```

Sessions are deduped by content hash + `source_event_id`, so this is
safe to re-run — the hook-captured episodes and the backfilled
episodes coexist.

## Namespaces

Pick a consistent namespace per project (usually the repo name) and use
it across all `loom_learn` / `loom_think` / `loom_recall` calls.
Namespace isolation is strict — memory in `namespace=a` is invisible to
queries in `namespace=b`.

## Known gaps

- Claude Code's per-project namespace is set by your discipline, not
  derived automatically from the working directory. Set `LOOM_NAMESPACE`
  per-project in your shell rc or a direnv file.
- The hook fires after the session ends — in-session `loom_learn` calls
  during the conversation still need discipline to pass verbatim content.
  See [ADR-005](../adr/005-verbatim-content-invariant.md).
