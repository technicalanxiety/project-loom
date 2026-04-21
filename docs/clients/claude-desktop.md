# Claude Desktop + Loom

Claude Desktop is Anthropic's consumer + Pro + Team app. It supports
MCP over HTTP via `claude_desktop_config.json`, and its account data
export is the path for backfilling historical conversations.

Unlike Claude Code, Desktop has no session hook — there is no way to
capture every conversation automatically at its close. Live capture on
this client is selective: the model calls `loom_learn` when the user
asks it to, following the Projects instructions.

## Live MCP capture

Copy [templates/claude_desktop_config.example.json](../../templates/claude_desktop_config.example.json)
into the client's config file:

- **macOS:** `~/Library/Application Support/Claude/claude_desktop_config.json`
- **Windows:** `%APPDATA%\Claude\claude_desktop_config.json`
- **Linux:** `~/.config/Claude/claude_desktop_config.json`

```json
{
  "mcpServers": {
    "loom": {
      "transport": {
        "type": "http",
        "url": "https://loom.yourdomain.com/mcp",
        "headers": {
          "Authorization": "Bearer ${LOOM_TOKEN}"
        }
      }
    }
  }
}
```

Replace `LOOM_TOKEN` and the URL placeholder. Restart Claude Desktop —
it rereads the config at launch.

The server hardcodes `ingestion_mode = live_mcp_capture` at the MCP
boundary, same as every other MCP transport.

## Discipline block — Projects instructions

Claude Desktop Projects has an "Instructions" field that the model
treats as a system prompt for every chat in that project. Paste
[templates/claude_desktop_projects_instructions.md](../../templates/claude_desktop_projects_instructions.md)
into it.

The key line is "Do not summarize, paraphrase, or reconstruct. Do not
pass your own descriptions of what was discussed. Only pass raw content
authored by me or quoted verbatim from source material I provided."
That sentence is load-bearing — it's the only thing standing between
Desktop's "let me summarize our conversation" tendency and the authority
hierarchy. See [ADR-005](../adr/005-verbatim-content-invariant.md).

## Vendor import — Claude.ai account export

Claude.ai (the web app backing Desktop conversations) has an account
data export under **Settings → Privacy → Export data**. You receive a
zip containing `conversations.json` after a short delay.

The [bootstrap/claude_ai_parser.py](../../bootstrap/claude_ai_parser.py)
parser asserts the `claude_ai_export_v2` schema on each conversation,
emits one episode per conversation (not per message — keeping causal
order for the extractor), and fails loud on any schema drift.

```bash
export LOOM_URL="https://loom.yourdomain.com"
export LOOM_TOKEN="your-bearer-token"

python3 bootstrap/claude_ai_parser.py \
    --export ~/Downloads/claude-data-export/conversations.json \
    --namespace my-project
```

Schema drift (Claude.ai rotates field names periodically) causes the
parser to exit non-zero with the specific failing field named. No
best-effort fallback — see the [degraded-mode contract](../../bootstrap/README.md#degraded-mode-contract).

## Namespaces

Claude Desktop has no working-directory concept. Pick a namespace per
Project (or per topical area) and let the Projects instructions carry
it — the template reserves a `Namespace:` line for you to fill in.

## Known gaps

- **No PostSession hook.** Live capture depends on the model's
  discipline to call `loom_learn` when you ask. Desktop sessions that
  ended without a `loom_learn` call are lost to live capture; they only
  enter Loom via the account export, if at all.
- **Selective capture is lossy by design.** The Desktop path is "save
  the things I pointed at." Everything else goes through the
  exhaustive-capture gap. If you need every turn, use Claude Code with
  the PostSession hook instead.
- **Vendor export cadence is user-driven.** You have to re-request the
  export when you want new conversations backfilled. The parser dedups
  by content hash, so re-ingesting the overlap is free.
