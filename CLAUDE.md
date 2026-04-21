# CLAUDE.md — Project Loom Integration Guide for Claude Code

This file documents how to register Project Loom as an MCP server in Claude Code,
configure namespace resolution, use manual overrides, and interact with the memory
system during development sessions.

## MCP Server Registration

### Quick Setup

Register Loom as an MCP server in Claude Code:

```bash
# Set your bearer token (from .env)
export LOOM_BEARER_TOKEN="your-token-here"

# Add Loom as an MCP server
claude mcp add loom-memory \
  --transport http \
  --url https://localhost/mcp/ \
  --header "Authorization: Bearer $LOOM_BEARER_TOKEN"
```

### Manual Configuration

If you prefer to edit the MCP config file directly, add this to your Claude Code
MCP settings (typically `~/.claude/mcp_servers.json` or project-level `.claude/mcp_servers.json`):

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

### Verify Connection

After registration, verify Loom is reachable:

```bash
# Health check (no auth required)
curl -s https://localhost/api/health | jq '.status'
# Expected: "ok"

# Test MCP endpoint (auth required)
curl -s -X POST https://localhost/mcp/loom_recall \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOOM_BEARER_TOKEN" \
  -d '{"entity_names": ["test"], "namespace": "default"}' | jq
```

---

## Available MCP Tools

Loom exposes three tools to Claude Code:

| Tool | Purpose | Latency |
|------|---------|---------|
| `loom_learn` | Ingest an episode (async — returns immediately) | < 50ms |
| `loom_think` | Compile a context package for a query | < 500ms p95 |
| `loom_recall` | Direct fact lookup for named entities | < 100ms |

---

## Namespace Resolution

Namespaces provide strict memory isolation. Every episode, entity, and fact belongs
to exactly one namespace. There is no cross-namespace retrieval.

### How Namespaces Work

- Each `loom_learn` and `loom_think` call requires a `namespace` parameter.
- Memory stored in namespace `project-a` is invisible to queries in namespace `project-b`.
- A `default` namespace exists for general knowledge not tied to a specific project.
- GitHub webhook episodes use the repository `full_name` (e.g. `owner/repo`) as the namespace.

### Choosing a Namespace

Use the project or repository name as the namespace. Keep it consistent across sessions:

```
my-project          # Simple project name
owner/repo          # GitHub repository (auto-set by webhook ingestion)
team/service-name   # Team-scoped service
default             # General knowledge
```

### Per-Namespace Configuration

Each namespace can have its own:
- **Hot tier token budget** (default: 500 tokens) — always-injected memory
- **Warm tier token budget** (default: 3000 tokens) — per-query retrieved memory
- **Predicate packs** (default: `["core"]`) — which domain vocabularies are active

Configure via the dashboard at `https://localhost` → Namespaces.

---

## Manual Overrides

### task_class_override

By default, `loom_think` classifies your query into one of five task classes to
select the right retrieval strategy. You can override this when you know what you need:

| Task Class | When to Use | Retrieval Profiles |
|------------|-------------|-------------------|
| `debug` | Troubleshooting errors, investigating issues | graph_neighborhood, episode_recall |
| `architecture` | Understanding system design, component relationships | fact_lookup, graph_neighborhood |
| `compliance` | Audit trails, policy verification, evidence gathering | episode_recall, fact_lookup |
| `writing` | Documentation, code generation | fact_lookup |
| `chat` | General conversation, unclear intent | fact_lookup |

**Example — force architecture retrieval for a design question:**

```json
{
  "query": "How does the payment service connect to the notification system?",
  "namespace": "my-project",
  "task_class_override": "architecture"
}
```

**Example — force debug retrieval when investigating an error:**

```json
{
  "query": "What changed in the auth flow recently?",
  "namespace": "my-project",
  "task_class_override": "debug"
}
```

### target_model

Controls the output format of the compiled context package:

| target_model | Output Format | Best For |
|-------------|---------------|----------|
| `claude` (default) | XML structured (`<loom>` tags) | Claude, Claude Code |
| `gpt-4` | JSON compact | GPT models, local models |
| Any string with "claude" | XML structured | Claude variants |
| Anything else | JSON compact | Other models |

**Example — get JSON output for a local model:**

```json
{
  "query": "What do we know about the deployment pipeline?",
  "namespace": "my-project",
  "target_model": "llama3"
}
```

### include_historical (loom_recall)

By default, `loom_recall` returns only current facts (where `valid_until IS NULL`).
Set `include_historical: true` to see superseded facts and trace how knowledge evolved:

```json
{
  "entity_names": ["Auth Service"],
  "namespace": "my-project",
  "include_historical": true
}
```

---

## Usage Examples

### Example 1: Learning from a Development Session

After a productive coding session, teach Loom what happened:

```bash
curl -s -X POST https://localhost/mcp/loom_learn \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOOM_BEARER_TOKEN" \
  -d '{
    "content": "Refactored the payment service to use event-driven architecture. Replaced direct HTTP calls between payment-service and notification-service with Azure Service Bus topics. The payment-completed topic triggers email and SMS notifications independently. Decided to use managed identity for Service Bus auth instead of connection strings.",
    "source": "claude-code",
    "namespace": "my-project",
    "participants": ["jason", "claude"]
  }'
```

Loom will:
1. Store the episode immediately (returns `"status": "queued"`)
2. Asynchronously extract entities: `payment-service`, `notification-service`, `Azure Service Bus`
3. Extract facts: `payment-service → uses → Azure Service Bus`, `payment-service → replaced_by → event-driven architecture`
4. Resolve entities against existing knowledge (3-pass: exact → alias → semantic)
5. Track temporal validity and supersede outdated facts

### Example 2: Getting Context for a New Task

When starting work on a related feature, ask Loom for context:

```bash
curl -s -X POST https://localhost/mcp/loom_think \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOOM_BEARER_TOKEN" \
  -d '{
    "query": "I need to add retry logic to the payment notification flow. What do I need to know?",
    "namespace": "my-project"
  }'
```

Loom will:
1. Classify intent as `debug` or `architecture` based on the query
2. Select retrieval profiles (e.g., graph_neighborhood + fact_lookup)
3. Find relevant entities and facts about the payment/notification flow
4. Compile a context package with hot tier memory + relevant warm tier results
5. Return XML structured output (default for Claude)

### Example 3: Looking Up Specific Entities

When you need facts about specific things without the full compilation pipeline:

```bash
curl -s -X POST https://localhost/mcp/loom_recall \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOOM_BEARER_TOKEN" \
  -d '{
    "entity_names": ["payment-service", "Azure Service Bus"],
    "namespace": "my-project"
  }'
```

Returns raw facts without classification or ranking — useful for quick lookups.

### Example 4: Compliance Query with Override

When gathering evidence for an audit, force the compliance retrieval strategy:

```bash
curl -s -X POST https://localhost/mcp/loom_think \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOOM_BEARER_TOKEN" \
  -d '{
    "query": "What authentication decisions have been made and what evidence supports them?",
    "namespace": "my-project",
    "task_class_override": "compliance"
  }'
```

The compliance profile prioritizes episode_recall (raw evidence) and fact_lookup,
and excludes procedural memory (weight 0.0) to focus on documented decisions.

### Example 5: Tracing Knowledge Evolution

To understand how knowledge about an entity changed over time:

```bash
curl -s -X POST https://localhost/mcp/loom_recall \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $LOOM_BEARER_TOKEN" \
  -d '{
    "entity_names": ["Auth Service"],
    "namespace": "my-project",
    "include_historical": true
  }'
```

This returns both current and superseded facts, showing the full timeline:
- `Auth Service → uses → JWT` (valid_from: 2024-06, valid_until: 2025-01)
- `Auth Service → uses → OAuth2` (valid_from: 2025-01, valid_until: null) ← current

---

## Tips

- **Be consistent with namespaces.** Use the same namespace string across all sessions
  for a project. Memory is strictly isolated per namespace.
- **Use `loom_learn` liberally.** Ingestion is async and idempotent. Duplicate content
  is detected by SHA-256 hash and skipped automatically.
- **Let classification work.** The intent classifier handles most queries well. Only use
  `task_class_override` when you know the default classification is wrong for your use case.
- **Check the dashboard.** The compilation trace viewer at `https://localhost` shows exactly
  what Loom retrieved, what it rejected, and why — useful for understanding retrieval quality.
- **Hot tier for critical knowledge.** Pin frequently-needed facts via the dashboard to
  ensure they're always included in context packages without consuming retrieval budget.
