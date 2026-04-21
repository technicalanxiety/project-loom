#!/usr/bin/env bash
# loom-capture.sh — Claude Code PostSession hook for exhaustive live capture.
#
# Reads the raw session JSONL produced by Claude Code and POSTs it verbatim
# to Loom's /api/learn endpoint with ingestion_mode=live_mcp_capture. No
# summarization. No filtering. No LLM inference. The transcript is the
# transcript.
#
# ## Install
#
# 1. Copy this file to a known path, e.g. ~/.claude/hooks/loom-capture.sh.
# 2. chmod +x ~/.claude/hooks/loom-capture.sh
# 3. Add the hook to ~/.claude/settings.json:
#
#      {
#        "hooks": {
#          "PostSession": [
#            { "matcher": "", "command": "~/.claude/hooks/loom-capture.sh" }
#          ]
#        }
#      }
#
# 4. Export env:
#      export LOOM_URL="https://loom.yourdomain.com"
#      export LOOM_TOKEN="your-bearer-token"
#      export LOOM_NAMESPACE="your-project-namespace"  # or set per-project
#
# Claude Code passes the session JSONL path as its first argument on
# PostSession. If your harness passes it via env instead, adjust below.

set -euo pipefail

: "${LOOM_URL:?set LOOM_URL to your Loom base URL, e.g. https://loom.yourdomain.com}"
: "${LOOM_TOKEN:?set LOOM_TOKEN to your Loom bearer token}"
: "${LOOM_NAMESPACE:?set LOOM_NAMESPACE to the namespace for this session}"

SESSION_FILE="${1:-${CLAUDE_SESSION_JSONL:-}}"
if [[ -z "$SESSION_FILE" || ! -r "$SESSION_FILE" ]]; then
  echo "loom-capture: no readable session file (got '$SESSION_FILE')" >&2
  exit 0
fi

# Read the raw JSONL verbatim. Do not transform, do not summarize.
CONTENT="$(cat "$SESSION_FILE")"

# Derive a stable idempotency key from the file path + modification time so
# re-running the hook on the same session file is a no-op at the content-hash
# layer too.
SOURCE_EVENT_ID="claude-code:$(basename "$SESSION_FILE"):$(stat -f %m "$SESSION_FILE" 2>/dev/null || stat -c %Y "$SESSION_FILE")"

# POST the verbatim transcript. ingestion_mode is the load-bearing field —
# the server trusts us when we say live_mcp_capture here.
curl -sS -X POST "${LOOM_URL}/api/learn" \
  -H "Authorization: Bearer ${LOOM_TOKEN}" \
  -H "Content-Type: application/json" \
  -d "$(jq -n \
        --arg content "$CONTENT" \
        --arg source "claude-code" \
        --arg namespace "$LOOM_NAMESPACE" \
        --arg source_event_id "$SOURCE_EVENT_ID" \
        '{
          content: $content,
          source: $source,
          namespace: $namespace,
          ingestion_mode: "live_mcp_capture",
          source_event_id: $source_event_id
        }')" \
  > /dev/null

echo "loom-capture: posted $(wc -c < "$SESSION_FILE") bytes from $SESSION_FILE"
