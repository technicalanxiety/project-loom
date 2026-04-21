# Microsoft 365 Copilot + Loom

M365 Copilot reaches MCP servers via **Copilot Studio declarative
agents**. The standalone Copilot chat experience does not talk to MCP
directly — you build a declarative agent that wires Loom's tools
(`loom_learn`, `loom_think`, `loom_recall`) as MCP actions, publish that
agent to the tenant, and users invoke it from the Copilot surface.

MCP in Copilot Studio went GA in Q3 2025 and MCP Apps (interactive UI
returns) shipped through 2026. Federated Copilot connectors reached GA
in April 2026 — those use MCP under the hood but target real-time
third-party data, not memory systems, so they're not the Loom integration
point.

## Live MCP capture — declarative agent via Copilot Studio

High-level flow:

1. Install the **Microsoft 365 Agents Toolkit** extension in VS Code.
2. Scaffold a new declarative agent. Pick **Start with an MCP server**.
3. Enter `https://loom.yourdomain.com/mcp/` as the MCP URL. The toolkit
   fetches the tool list (`loom_learn`, `loom_think`, `loom_recall`) and
   generates the plugin spec.
4. Choose which tools to expose (all three, for the full Loom surface).
5. Configure auth. Loom uses bearer-token auth; in Copilot Studio that
   maps to **API key authentication** (simpler) or OAuth 2.0 with DCR
   (if you want per-user auth). See
   [templates/m365_copilot_agent_manifest.example.json](../../templates/m365_copilot_agent_manifest.example.json)
   for the reference manifest skeleton.
6. Deploy the declarative agent to the tenant.
7. Users invoke it by `@`-mentioning the agent in Microsoft 365 Copilot
   chat, or by selecting it from the agent picker.

The server hardcodes `ingestion_mode = live_mcp_capture` at the MCP
boundary — identical contract to every other transport.

### Licensing & cost

Every MCP tool call from a Copilot Studio agent bills as an **Agent
Action** at a fixed rate (Microsoft abstracts the LLM orchestration +
tool execution into a single unit). Heavy `loom_learn` traffic from
M365 Copilot costs Agent Actions. Plan for it — or route bulk ingestion
through the REST endpoint outside the agent, where Agent Actions don't
apply.

## Discipline block — agent instructions

The declarative agent manifest includes an `instructions` field. Paste
[templates/m365_copilot_instructions.md](../../templates/m365_copilot_instructions.md)
into it.

Same verbatim-content invariant: the agent may call `loom_learn` only
with verbatim content — raw user text, quoted source material. Never a
summary of the Copilot session. See
[ADR-005](../adr/005-verbatim-content-invariant.md).

## Vendor import — M365 compliance export

Microsoft 365 Copilot interactions are logged in the Purview audit log
and can be exported via **eDiscovery** or the **Microsoft Purview
Content Search** APIs. The export format is XML-structured M365
compliance bundles containing Copilot chat records.

The [bootstrap/m365_copilot_parser.py](../../bootstrap/m365_copilot_parser.py)
parser asserts the `m365_copilot_audit_v1` schema against the Purview
export bundle:

```bash
export LOOM_URL="https://loom.yourdomain.com"
export LOOM_TOKEN="your-bearer-token"

python3 bootstrap/m365_copilot_parser.py \
    --export-dir ~/Downloads/m365-purview-export/ \
    --namespace my-tenant
```

Because Purview exports are tenant-admin-gated, this path is typically
run by an admin once per compliance cycle, not by end users. The parser
dedups by the M365 `InteractionId` field + content hash so re-running
after re-exporting is safe.

> **Schema drift warning.** Purview export schemas evolve. The parser
> pins `m365_copilot_audit_v1` and fails loud on any divergence — see
> the [degraded-mode contract](../../bootstrap/README.md#degraded-mode-contract).

## Namespaces

Declarative agents are tenant-scoped by default. Use the tenant name
or a department-scoped namespace consistently. The agent's instructions
can pin a namespace per-agent, so one agent per namespace is a cleaner
pattern than letting the model pick.

## Known gaps

- **Agent surface only.** M365 Copilot chat itself does not hold an MCP
  session — only declarative agents do. Users must invoke the agent
  (via `@` or picker) for Loom to see the turn.
- **Agent Action billing.** Tool calls cost Agent Actions. Watch this on
  high-traffic namespaces.
- **No PostSession equivalent.** M365 Copilot has no hook analogous to
  Claude Code's. Live capture is what the agent calls `loom_learn` with
  during a turn; anything else has to come from the Purview export.
- **Purview export requires admin.** End users can't self-serve the
  vendor import path; an admin has to run eDiscovery / Content Search
  and hand off the export bundle.
- **Tenant data boundary.** Loom is deployed on your infrastructure; the
  M365 Copilot MCP call crosses the Microsoft tenant boundary to reach
  it. Review the data egress posture with your compliance team before
  turning this on for sensitive namespaces.
