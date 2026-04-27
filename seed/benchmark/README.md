# Benchmark namespace seed corpus

The benchmark suite in `loom-engine/src/pipeline/benchmark.rs` runs ten
queries against the `benchmark` namespace. Each task declares the
entity names and predicate identifiers it expects to see in retrieved
context, and the dashboard's A/B/C cards score how well the LLM uses
that context to answer the query.

If the namespace is empty, condition B and C compile an empty
context and the LLM has nothing useful to work with — the cards will
look identical to condition A. **Seed this corpus before reading the
benchmark numbers.**

## Seeding

Each `*.md` file in this directory is a verbatim episode. Post them
through `loom-seed.py`, which uses the `user_authored_seed` ingestion
mode (provenance coefficient 0.8):

```bash
export LOOM_URL="https://loom.yourdomain.com"
export LOOM_TOKEN="your-bearer-token"

cli/loom-seed.py --namespace benchmark seed/benchmark/
```

The worker will extract entities and facts in the background. Check
the dashboard's *Compilations* or *Entities* page (or `GET
/dashboard/api/episodes?namespace=benchmark`) to confirm extraction
finished before triggering a benchmark run — episodic retrieval
(condition B) works as soon as episodes land, but graph and fact
retrieval (condition C) needs extraction to complete first.

## Verbatim content invariant

These files are user-authored prose, not LLM summaries. See
[ADR-005](../../docs/adr/005-verbatim-content-invariant.md). Edit
them by hand if you want to add or rebalance ground-truth coverage.

## Coverage

| File | Task | Expected entities | Expected predicates |
|------|------|-------------------|---------------------|
| `01-debug-auth-failure.md` | `debug_auth_failure` | APIM, Auth Service | uses, deployed_to |
| `02-debug-memory-leak.md` | `debug_memory_leak` | Worker Service, Redis Cache | depends_on, monitors |
| `03-arch-service-topology.md` | `arch_service_topology` | Payment Service, Stripe Gateway | communicates_with, owns |
| `04-arch-data-flow.md` | `arch_data_flow` | Data Pipeline, Analytics Dashboard | produces, consumes |
| `05-compliance-gdpr.md` | `compliance_gdpr_audit` | User Data Store, GDPR Policy | complies_with, governs |
| `06-compliance-access-review.md` | `compliance_access_review` | Production DB, IAM Policy | authorized_by, restricts |
| `07-writing-api-docs.md` | `writing_api_docs` | Notification Service, REST API | exposes, implements |
| `08-writing-runbook.md` | `writing_runbook` | CI/CD Pipeline, Kubernetes Cluster | deploys_to, manages |
| `09-chat-project-status.md` | `chat_project_status` | Project Sentinel | has_status |
| `10-chat-team-ownership.md` | `chat_team_ownership` | Billing Module, Platform Team | owns, maintains |
