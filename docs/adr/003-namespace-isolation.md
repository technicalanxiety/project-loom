# ADR-003: Hard Namespace Isolation

## Status

Accepted

## Context

Project Loom stores knowledge from multiple projects/contexts. The question is whether
entities and facts should be queryable across namespaces (e.g., "find all uses of Redis
across all my projects") or strictly isolated.

Cross-namespace queries are powerful but introduce:
- Ambiguity: Same entity name in different contexts means different things.
- Security: Namespace boundaries may represent trust boundaries.
- Complexity: Resolution, ranking, and compilation must handle multi-namespace results.

## Decision

Hard namespace isolation in MVP. No cross-namespace queries.

- Every entity, fact, episode, and procedure belongs to exactly one namespace.
- Queries are scoped to one namespace.
- If the same real-world entity appears in two namespaces, it exists as two separate records.
- Hot-tier content is namespace-scoped. No global hot tier.
- Idempotency keys are scoped to the namespace boundary. In particular,
  `source_event_id` dedup uses `(namespace, source, source_event_id)`;
  the same source event replayed into two namespaces represents two
  distinct episodes.

## Consequences

### Positive

- Simple, predictable query behavior.
- No accidental data leakage between contexts.
- Easier to reason about ranking and compilation within a single namespace.
- Clean deletion semantics — drop a namespace, drop all its data.
- Bootstrap and seed workflows can safely replay the same source export
  into a different namespace without being suppressed by a row in the
  original namespace.

### Negative

- Users cannot query across projects without switching namespaces.
- Duplicate entity records across namespaces (no shared knowledge).
- May frustrate users who want a "global search" capability.

### Neutral

- Track how often users hit the namespace wall during benchmarking.
- If >10% of queries show cross-namespace intent, revisit in Phase 2.
