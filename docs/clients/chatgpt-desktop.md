# ChatGPT Desktop + Loom

ChatGPT (Desktop and web) added MCP client support in ChatGPT Developer
Mode, rolling out through 2025–2026 on Business, Enterprise, and Edu
plans. On ChatGPT, MCP servers are surfaced as **apps** (renamed from
"connectors" on 2025-12-17).

Loom can be registered as a custom MCP app for live capture, and the
ChatGPT **Data Controls export** (`conversations.json`) is the path for
backfilling historical chats.

## Live MCP capture — Developer Mode apps

ChatGPT's MCP support lives behind Developer Mode.

1. Workspace admin / owner enables **Developer Mode** in ChatGPT
   workspace settings (Business / Enterprise / Edu only, at time of
   writing — beta).
2. In **Settings → Apps & connectors → Build**, create a new custom app
   with the MCP endpoint URL `https://loom.yourdomain.com/mcp/` and the
   `Authorization: Bearer <LOOM_TOKEN>` header.
3. Publish it for your workspace (or just yourself).
4. In a conversation, open the **Plus menu → Developer mode** and
   select the Loom app.

ChatGPT shows a confirmation dialog before any write (non-read-only)
tool call. `loom_learn` is a write; `loom_think` and `loom_recall` are
reads. Loom declares the `readOnlyHint` tool annotation so ChatGPT can
distinguish them.

The server hardcodes `ingestion_mode = live_mcp_capture` at the MCP
boundary.

> **Consumer ChatGPT:** custom MCP apps are not available on Free, Plus,
> or Pro at time of writing. Use the vendor import path (below) for
> those plans.

## Discipline block — Custom Instructions / Projects

ChatGPT Projects has a "Custom instructions" field, and the account also
has global custom instructions. Paste
[templates/chatgpt_custom_instructions.md](../../templates/chatgpt_custom_instructions.md)
into whichever surface covers the chats you want disciplined.

Same load-bearing sentence as the other clients: verbatim content only,
no summaries, no reconstructions. See
[ADR-005](../adr/005-verbatim-content-invariant.md).

## Vendor import — Data Controls export

ChatGPT ships a per-user data export at **Settings → Data Controls →
Export data**. You receive an email with a zip; inside is
`conversations.json` — a tree-structured object with:

- `title`, `create_time`, `update_time` at the conversation level
- A `mapping` object keyed by message ID
- A `current_node` pointer, with `parent` links forming the canonical
  path through the tree (and sibling branches for edits /
  regenerations)

The [bootstrap/chatgpt_parser.py](../../bootstrap/chatgpt_parser.py)
parser asserts the `chatgpt_export_v1` schema, walks each conversation's
current-node chain to extract the canonical linear transcript, and POSTs
one episode per conversation:

```bash
export LOOM_URL="https://loom.yourdomain.com"
export LOOM_TOKEN="your-bearer-token"

python3 bootstrap/chatgpt_parser.py \
    --export ~/Downloads/chatgpt-data-export/conversations.json \
    --namespace my-project
```

Branch handling is deliberate: we follow `current_node` only. Edit /
regeneration branches are not ingested — they represent paths not taken,
and entity / fact extraction on them would attribute "decisions" that
never actually happened. If you want the branches, re-export after
switching the active branch in the ChatGPT UI.

Fail-loud on schema drift applies here too. OpenAI has rotated field
names on this export historically; the parser pins `chatgpt_export_v1`
and exits non-zero if the fields don't match.

## Namespaces

ChatGPT has no project-root / working-directory concept visible to
tools. Use Projects + a Projects-scoped discipline block to carry a
namespace, or have the user include it in their message when they ask
you to save something.

## Known gaps

- **MCP only on paid workspaces.** Plus / Pro / Free can't register
  Loom as an app. Use vendor import for those, or run ChatGPT sessions
  through a paid workspace.
- **Developer Mode is beta.** UI, permissions, and app shape change
  quarterly. Re-check the setup if registration breaks.
- **Export cadence is user-driven.** Like every account-export path, you
  re-export on demand. Dedup by content hash makes this free to re-run.
- **Branches are dropped.** Only the active branch at export time is
  ingested. Document this if branch history matters to your
  investigations.
