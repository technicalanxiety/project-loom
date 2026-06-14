# ADR-012: Memory Consolidation and Active Forgetting Pipeline

## Status

Accepted

## Context

As loom accumulates extracted facts over time, two problems compound:

1. **Fact sprawl.** A single entity can accumulate dozens of stable,
   non-contradicting facts from multiple ingestion sessions. Querying these
   facts individually is verbose and expensive at compile time — the context
   package grows linearly with fact count for heavily-referenced entities.

2. **Stale artifact accumulation.** Procedures that haven't matched any
   episode in months, and resolution conflicts that no human has reviewed,
   silently occupy the hot tier and inflate context packages without
   contributing useful signal.

Both problems erode the precision of the retrieval layer over time: more
tokens consumed, lower signal-to-noise, slower compilation. The four-dimension
ranker (ADR-001, ADR-009) filters well at retrieval time but does not reduce
the underlying volume.

What was needed was a background lifecycle for memory: a **consolidation**
phase that synthesizes clusters of stable facts into higher-order summaries,
and a **pruning** phase that removes artifacts that have aged out of
usefulness.

## Decision

Add two new background phases to the offline pipeline, each running on a
configurable schedule (default: nightly at 02:00 per namespace).

### Consolidation phase (`worker/consolidator.rs`)

1. **Cluster identification.** For each namespace, query entities with
   ≥ `consolidation_min_cluster` stable, non-superseded facts older than 48
   hours. Cap at 20 entities per run to bound LLM load. Default minimum
   cluster size is 5 facts.

2. **Synthesis.** For each cluster, call the offline LLM with a structured
   prompt (`prompts/consolidation.txt`) that produces a single coherent
   paragraph plus a coverage map — every claim in the summary must cite at
   least one source fact ID. The prompt also requests a `conflicts_detected`
   array; detected conflicts are recorded on the summary rather than silently
   resolved.

3. **Hallucination guard.** After synthesis, every fact UUID cited in the
   coverage map is cross-checked against the cluster's source fact IDs. Any
   UUID not in the cluster causes the summary to be rejected with
   `ConsolidationError::HallucinatedFactReference` — the run logs the failure
   and moves to the next cluster.

4. **Storage.** Accepted summaries are upserted into `loom_summaries`
   (migration 018) with full provenance: `source_facts` UUID array,
   `synthesis_model`, `synthesis_prompt_ver`, `contains_sole_source` flag, and
   `invalidated_at` (null until a source fact is superseded). A companion
   `loom_summary_state` table carries the 768-dimensional embedding and access
   tracking for warm-tier retrieval.

5. **Invalidation.** When a source fact is superseded during online
   extraction, any `loom_summaries` row that references that fact's UUID in
   its `source_facts` array has `invalidated_at` stamped. Invalidated summaries
   are excluded from retrieval but retained for the TTL window (default 30
   days) before pruning phase soft-deletes them.

### Pruning phase (`worker/consolidator.rs`)

Runs in the same daily cycle immediately after consolidation:

- **Stale procedures.** Soft-delete procedures where
  `last_matched_at + pruning_procedure_ttl_days` (default 90 days) has
  elapsed. The new `last_matched_at` and `decay_eligible_at` columns
  (migration 020) track this without touching the existing procedures table
  structure.

- **Auto-resolved conflicts.** Resolution conflicts older than
  `pruning_conflict_ttl_days` (default 60 days) that are still unresolved
  are auto-resolved by the pruning phase. The intent is that very old
  conflicts are almost certainly stale (the entity or fact that caused them
  has likely been superseded), and leaving them unresolved indefinitely
  pollutes the conflict queue.

- **Invalidated summaries.** Summaries where
  `invalidated_at + summary_invalidation_ttl_days` (default 30 days) has
  elapsed are soft-deleted.

### Scheduling

The scheduler (`worker/scheduler.rs`) was extended to spawn a fourth
background job — `daily_consolidation` — on a 24-hour interval alongside the
existing snapshot, tier management, and entity health check jobs. The cycle
iterates over all configured namespaces, reading per-namespace consolidation
settings from `loom_namespace_config` (migration 020 adds
`consolidation_min_cluster`, `consolidation_schedule`,
`pruning_procedure_ttl_days`, `pruning_conflict_ttl_days`,
`pruning_summary_invalidation_ttl_days`).

### Dashboard

A new **Consolidation** page (`loom-dashboard/src/pages/ConsolidationPage.tsx`)
surfaces:
- KPIs: active summaries, invalidated summaries, latest run timestamps.
- Recent run history table (type, status, duration, per-phase details).
- A **"Run consolidation now"** button that triggers an immediate
  consolidation + pruning cycle via
  `POST /dashboard/api/consolidation/run/{namespace}`.

The consolidation health endpoint is
`GET /dashboard/api/consolidation/health/{namespace}`.

Telemetry is logged to `loom_consolidation_log` (migration 019) with separate
rows for `consolidation` and `pruning` run types, each carrying a status
(`running` | `completed` | `failed`), durations, and per-phase counters.

### LLM model selection

The synthesis model is selected from `loom_namespace_config.hot_tier_budget`
using the same heuristic as ADR-009: namespaces with `hot_tier_budget > 1000`
use `qwen2.5:32b`; others use `qwen2.5:14b`. The consolidation prompt is
schema-constrained JSON (consistent with ADR-011).

## Alternatives considered

1. **Summarize at extraction time.** Rejected: violates the verbatim content
   invariant (ADR-005). Summaries are derived artifacts, not episode content.
   They must be produced offline from extracted facts, not inlined into
   ingestion.

2. **No pruning — rely purely on tier demotion.** Tier demotion (ADR-001,
   tier management scheduler) moves stale facts and entities to warm, but does
   not remove them. Without pruning, the warm tier grows unboundedly. For a
   personal infrastructure system with multi-year lifetimes, unbounded growth
   is a real problem.

3. **Manual consolidation only.** The dashboard "run now" button exists, but
   relying on manual operation means consolidation never happens if the
   operator forgets. The nightly automatic cycle is the default; manual is the
   escape hatch.

4. **Merge consolidated facts back into `loom_facts`.** Rejected: summaries
   are derived artifacts and must be traceable to their source facts. Merging
   them into the fact table would destroy provenance and mix extraction-quality
   facts with synthesis-quality summaries. The separate `loom_summaries` table
   maintains the distinction.

## Consequences

### Positive

- Context packages can reference a single summary instead of N facts for a
  well-covered entity, reducing token consumption.
- Stale procedures and ancient unresolved conflicts are automatically removed,
  keeping the hot tier free of useless weight.
- The hallucination guard ensures summaries never introduce entity relationships
  or facts that didn't come from the source cluster.
- All consolidation activity is auditable: `loom_consolidation_log` records
  every run with counters, duration, and error detail.

### Negative

- Summaries are an additional artifact type that retrieval must account for.
  Warm-tier retrieval now searches `loom_summary_state.embedding` alongside
  `loom_fact_state.embedding` and `loom_entity_state.embedding`.
- Synthesis calls the LLM for each cluster — up to 20 times per namespace per
  night. On local Ollama this is low cost; on cloud LLMs it adds billing
  exposure proportional to namespace activity.
- Auto-resolution of stale conflicts discards human review. The 60-day default
  is conservative enough that most auto-resolved conflicts will genuinely be
  stale, but operators working on slow-moving data should raise the TTL.

### Neutral

- Three new migrations (018–020) and three new tables (`loom_summaries`,
  `loom_summary_state`, `loom_consolidation_log`). Existing migrations and
  tables are not altered except for the additive columns in migration 020.
- The consolidation prompt version is pinned to `"consolidation_v1"` in the
  `synthesis_prompt_ver` column. Prompt changes should bump this string so
  historical summaries can be distinguished from re-synthesized ones.
- Consolidation only targets facts older than 48 hours to avoid synthesizing a
  cluster that is still actively receiving new facts from ongoing extraction.
