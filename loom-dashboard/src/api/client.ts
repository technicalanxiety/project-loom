/**
 * Typed fetch wrapper for the loom-engine dashboard API.
 *
 * All dashboard views call these functions instead of using `fetch` directly.
 * The base URL is `/dashboard/api` which Vite proxies to loom-engine in dev
 * and Caddy routes in production.
 */
import type {
  ActivePredicatesResponse,
  BenchmarkComparison,
  BenchmarkRun,
  CancelBenchmarkResponse,
  ClassificationMetrics,
  CompilationDetail,
  CompilationSummary,
  ConflictSummary,
  EntityDetail,
  EntitySummary,
  ExtractionMetrics,
  FactSummary,
  GraphResponse,
  HotTierMetrics,
  IngestionDistributionMetrics,
  NamespaceInfo,
  PackDetail,
  PackSummary,
  ParserHealthMetrics,
  PipelineHealthResponse,
  PredicateCandidateSummary,
  RequeueAllFailedResponse,
  ResolveConflictRequest,
  ResolveConflictResponse,
  ResolvePredicateCandidateRequest,
  ResolvePredicateCandidateResponse,
  RetrievalMetrics,
  SeedSummary,
} from '../types';

const BASE_URL = '/dashboard/api';

// ---------------------------------------------------------------------------
// Generic helpers
// ---------------------------------------------------------------------------

/** Error thrown when the dashboard API returns a non-2xx status. */
export class ApiError extends Error {
  constructor(
    public readonly status: number,
    public readonly statusText: string,
    public readonly body?: string,
  ) {
    super(`API error: ${status} ${statusText}`);
    this.name = 'ApiError';
  }
}

/**
 * Fetch JSON from the dashboard API.
 *
 * @param path - Path relative to `/dashboard/api` (e.g. `/health`).
 * @param options - Optional `RequestInit` overrides.
 * @returns Parsed JSON response typed as `T`.
 * @throws {ApiError} When the response status is not ok.
 */
export async function fetchJson<T>(path: string, options?: RequestInit): Promise<T> {
  const response = await fetch(`${BASE_URL}${path}`, options);
  if (!response.ok) {
    const body = await response.text().catch(() => undefined);
    throw new ApiError(response.status, response.statusText, body);
  }
  return response.json() as Promise<T>;
}

/**
 * POST JSON to the dashboard API.
 *
 * @param path - Path relative to `/dashboard/api`.
 * @param body - Request body to serialize as JSON.
 * @returns Parsed JSON response typed as `T`.
 * @throws {ApiError} When the response status is not ok.
 */
export async function postJson<T>(path: string, body: unknown): Promise<T> {
  return fetchJson<T>(path, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
}

// ---------------------------------------------------------------------------
// Query-string builder
// ---------------------------------------------------------------------------

/** Build a query string from an object, omitting undefined/null values. */
function qs(params: Record<string, string | number | undefined | null>): string {
  const entries = Object.entries(params).filter(
    (entry): entry is [string, string | number] => entry[1] != null,
  );
  if (entries.length === 0) return '';
  return `?${new URLSearchParams(entries.map(([k, v]) => [k, String(v)])).toString()}`;
}

// ---------------------------------------------------------------------------
// GET endpoints
// ---------------------------------------------------------------------------

/** Fetch pipeline health overview. */
export async function getPipelineHealth(): Promise<PipelineHealthResponse> {
  return fetchJson<PipelineHealthResponse>('/health');
}

/** Fetch all configured namespaces. */
export async function getNamespaces(): Promise<NamespaceInfo[]> {
  return fetchJson<NamespaceInfo[]>('/namespaces');
}

/** Fetch paginated compilation summaries. */
export async function getCompilations(params?: {
  namespace?: string;
  limit?: number;
  offset?: number;
}): Promise<CompilationSummary[]> {
  return fetchJson<CompilationSummary[]>(`/compilations${qs(params ?? {})}`);
}

/** Fetch full detail for a single compilation trace. */
export async function getCompilationDetail(id: string): Promise<CompilationDetail> {
  return fetchJson<CompilationDetail>(`/compilations/${encodeURIComponent(id)}`);
}

/** Fetch paginated entity summaries with optional filters. */
export async function getEntities(params?: {
  namespace?: string;
  entity_type?: string;
  q?: string;
  limit?: number;
  offset?: number;
}): Promise<EntitySummary[]> {
  return fetchJson<EntitySummary[]>(`/entities${qs(params ?? {})}`);
}

/** Fetch full detail for a single entity. */
export async function getEntityDetail(id: string): Promise<EntityDetail> {
  return fetchJson<EntityDetail>(`/entities/${encodeURIComponent(id)}`);
}

/** Fetch the graph neighborhood for an entity. */
export async function getEntityGraph(id: string): Promise<GraphResponse> {
  return fetchJson<GraphResponse>(`/entities/${encodeURIComponent(id)}/graph`);
}

/** Fetch paginated fact summaries with optional filters. */
export async function getFacts(params?: {
  namespace?: string;
  predicate?: string;
  evidence_status?: string;
  limit?: number;
  offset?: number;
}): Promise<FactSummary[]> {
  return fetchJson<FactSummary[]>(`/facts${qs(params ?? {})}`);
}

/** Fetch all unresolved entity conflicts. */
export async function getConflicts(): Promise<ConflictSummary[]> {
  return fetchJson<ConflictSummary[]>('/conflicts');
}

/** Fetch all predicate candidates. */
export async function getPredicateCandidates(): Promise<PredicateCandidateSummary[]> {
  return fetchJson<PredicateCandidateSummary[]>('/predicates/candidates');
}

/** Fetch all predicate pack summaries. */
export async function getPredicatePacks(): Promise<PackSummary[]> {
  return fetchJson<PackSummary[]>('/predicates/packs');
}

/** Fetch full detail for a predicate pack. */
export async function getPredicatePackDetail(pack: string): Promise<PackDetail> {
  return fetchJson<PackDetail>(`/predicates/packs/${encodeURIComponent(pack)}`);
}

/** Fetch active predicates for a namespace. */
export async function getActivePredicates(namespace: string): Promise<ActivePredicatesResponse> {
  return fetchJson<ActivePredicatesResponse>(`/predicates/active/${encodeURIComponent(namespace)}`);
}

/** Fetch retrieval quality metrics. */
export async function getRetrievalMetrics(): Promise<RetrievalMetrics> {
  return fetchJson<RetrievalMetrics>('/metrics/retrieval');
}

/** Fetch extraction pipeline metrics. */
export async function getExtractionMetrics(): Promise<ExtractionMetrics> {
  return fetchJson<ExtractionMetrics>('/metrics/extraction');
}

/** Fetch classification confidence distribution metrics. */
export async function getClassificationMetrics(): Promise<ClassificationMetrics> {
  return fetchJson<ClassificationMetrics>('/metrics/classification');
}

/** Fetch hot-tier utilization metrics. */
export async function getHotTierMetrics(): Promise<HotTierMetrics> {
  return fetchJson<HotTierMetrics>('/metrics/hot-tier');
}

/** Fetch bootstrap parser health metrics. */
export async function getParserHealthMetrics(): Promise<ParserHealthMetrics> {
  return fetchJson<ParserHealthMetrics>('/metrics/parser-health');
}

/** Fetch ingestion-mode distribution per namespace. */
export async function getIngestionDistributionMetrics(): Promise<IngestionDistributionMetrics> {
  return fetchJson<IngestionDistributionMetrics>('/metrics/ingestion-distribution');
}

// ---------------------------------------------------------------------------
// POST endpoints (mutations)
// ---------------------------------------------------------------------------

/** Resolve an entity conflict. */
export async function resolveConflict(
  id: string,
  request: ResolveConflictRequest,
): Promise<ResolveConflictResponse> {
  return postJson<ResolveConflictResponse>(`/conflicts/${encodeURIComponent(id)}/resolve`, request);
}

/** Resolve a predicate candidate. */
export async function resolvePredicateCandidate(
  id: string,
  request: ResolvePredicateCandidateRequest,
): Promise<ResolvePredicateCandidateResponse> {
  return postJson<ResolvePredicateCandidateResponse>(
    `/predicates/candidates/${encodeURIComponent(id)}/resolve`,
    request,
  );
}

/**
 * Bulk-reset every episode currently in `failed` state back to `pending`.
 *
 * Used by the Runtime page's "Retry failed" button after the operator has
 * fixed the root cause of a failure class. Idempotent — when there are no
 * failures, returns `{requeued: 0}`.
 */
export async function requeueAllFailedEpisodes(): Promise<RequeueAllFailedResponse> {
  return postJson<RequeueAllFailedResponse>('/episodes/failed/requeue-all', {});
}

// ---------------------------------------------------------------------------
// Benchmark endpoints
// ---------------------------------------------------------------------------

/** Fetch all benchmark runs ordered by most recent first. */
export async function getBenchmarkRuns(): Promise<BenchmarkRun[]> {
  return fetchJson<BenchmarkRun[]>('/benchmarks');
}

/** Fetch full benchmark comparison detail for a specific run. */
export async function getBenchmarkDetail(id: string): Promise<BenchmarkComparison> {
  return fetchJson<BenchmarkComparison>(`/benchmarks/${encodeURIComponent(id)}`);
}

/** Trigger a new benchmark run across all A/B/C conditions. */
export async function runBenchmark(): Promise<BenchmarkRun> {
  return postJson<BenchmarkRun>('/benchmarks/run', {});
}

/** Seed the `benchmark` namespace with the engine's embedded corpus.
 * Idempotent — calling twice on a populated namespace returns
 * `{inserted: 0, duplicates: <total>}`. Extraction runs asynchronously after
 * seeding, so the operator must wait before running a benchmark. */
export async function seedBenchmark(): Promise<SeedSummary> {
  return postJson<SeedSummary>('/benchmarks/seed', {});
}

/** Cancel a running benchmark by flipping the row to `failed`. The pipeline
 * keeps executing in the background — partial results that already landed
 * are preserved — but the dashboard's spinner stops. Returns
 * `{cancelled: false}` if the run was already in a terminal state. */
export async function cancelBenchmark(id: string): Promise<CancelBenchmarkResponse> {
  return postJson<CancelBenchmarkResponse>(`/benchmarks/${encodeURIComponent(id)}/cancel`, {});
}
