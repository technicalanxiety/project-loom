// loom-dashboard/src/api/client.ts
// Typed fetch wrapper for loom-engine dashboard API.

// TODO: Implement
// - fetchJson<T>(path, options?) -> Promise<T>
// - getPipelineHealth() -> Promise<PipelineHealth>
// - getEntities(namespace) -> Promise<Entity[]>
// - getFacts(namespace) -> Promise<Fact[]>
// - getTraces(namespace, limit, offset) -> Promise<AuditLogEntry[]>
// - getConflicts() -> Promise<ResolutionConflict[]>
// - getCandidates() -> Promise<PredicateCandidate[]>
// - getMetrics(namespace) -> Promise<RetrievalMetrics>
// - resolveConflict(id, resolution) -> Promise<void>
// - resolveCandidate(id, resolution) -> Promise<void>

const BASE_URL = "/dashboard/api";

export async function fetchJson<T>(path: string): Promise<T> {
  const response = await fetch(`${BASE_URL}${path}`);
  if (!response.ok) {
    throw new Error(`API error: ${response.status} ${response.statusText}`);
  }
  return response.json() as Promise<T>;
}
