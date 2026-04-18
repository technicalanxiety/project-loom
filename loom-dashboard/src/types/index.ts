// loom-dashboard/src/types/index.ts
// TypeScript types mirroring loom-engine Rust types.

// TODO: Implement full type definitions

export interface Episode {
  id: string;
  source: string;
  content: string;
  content_hash: string;
  occurred_at: string;
  namespace: string;
  processed: boolean;
}

export interface Entity {
  id: string;
  name: string;
  entity_type: string;
  namespace: string;
  properties: Record<string, unknown>;
}

export interface Fact {
  id: string;
  subject_id: string;
  predicate: string;
  object_id: string;
  namespace: string;
  valid_from: string;
  valid_until: string | null;
  evidence_status: string;
}

export interface AuditLogEntry {
  id: string;
  created_at: string;
  task_class: string;
  namespace: string;
  query_text: string;
  compiled_tokens: number;
  latency_total_ms: number;
}

export interface ResolutionConflict {
  id: string;
  entity_name: string;
  entity_type: string;
  namespace: string;
  candidates: unknown;
  resolved: boolean;
}

export interface PredicateCandidate {
  id: string;
  predicate: string;
  occurrences: number;
  mapped_to: string | null;
}
