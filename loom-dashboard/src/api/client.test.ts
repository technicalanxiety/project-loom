/**
 * Unit tests for the typed dashboard API client.
 *
 * Mocks the global `fetch` to verify request construction,
 * error handling, and response parsing.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  ApiError,
  fetchJson,
  getCompilationDetail,
  getCompilations,
  getConflicts,
  getEntities,
  getEntityDetail,
  getEntityGraph,
  getPipelineHealth,
  getPredicateCandidates,
  getPredicatePacks,
  getRetrievalMetrics,
  postJson,
  resolveConflict,
  resolvePredicateCandidate,
} from './client';

/** Create a mock Response with JSON body. */
function mockResponse(body: unknown, status = 200, statusText = 'OK'): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText,
    json: () => Promise.resolve(body),
    text: () => Promise.resolve(JSON.stringify(body)),
    headers: new Headers(),
    redirected: false,
    type: 'basic',
    url: '',
    clone: () => mockResponse(body, status, statusText),
    body: null,
    bodyUsed: false,
    arrayBuffer: () => Promise.resolve(new ArrayBuffer(0)),
    blob: () => Promise.resolve(new Blob()),
    formData: () => Promise.resolve(new FormData()),
    bytes: () => Promise.resolve(new Uint8Array()),
  } as Response;
}

describe('API client', () => {
  const fetchSpy = vi.fn<(input: RequestInfo | URL, init?: RequestInit) => Promise<Response>>();

  beforeEach(() => {
    vi.stubGlobal('fetch', fetchSpy);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe('fetchJson', () => {
    it('returns parsed JSON on success', async () => {
      const payload = { hello: 'world' };
      fetchSpy.mockResolvedValueOnce(mockResponse(payload));
      const result = await fetchJson<{ hello: string }>('/test');
      expect(result).toEqual(payload);
      expect(fetchSpy).toHaveBeenCalledWith('/dashboard/api/test', undefined);
    });

    it('throws ApiError on non-2xx response', async () => {
      fetchSpy.mockResolvedValueOnce(mockResponse('not found', 404, 'Not Found'));
      await expect(fetchJson('/missing')).rejects.toThrow(ApiError);
    });

    it('includes status in ApiError', async () => {
      fetchSpy.mockResolvedValueOnce(mockResponse({ detail: 'bad' }, 400, 'Bad Request'));
      try {
        await fetchJson('/bad');
        expect.fail('should have thrown');
      } catch (err) {
        expect(err).toBeInstanceOf(ApiError);
        expect((err as ApiError).status).toBe(400);
      }
    });
  });

  describe('postJson', () => {
    it('sends POST with JSON body', async () => {
      const response = { id: '123', resolved: true };
      fetchSpy.mockResolvedValueOnce(mockResponse(response));
      const result = await postJson<typeof response>('/test', { action: 'do' });
      expect(result).toEqual(response);
      expect(fetchSpy).toHaveBeenCalledWith('/dashboard/api/test', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action: 'do' }),
      });
    });
  });

  describe('GET endpoints', () => {
    it('getPipelineHealth calls /health', async () => {
      const health = {
        episodes_by_source: [],
        episodes_by_namespace: [],
        entities_by_type: [],
        facts_current: 10,
        facts_superseded: 2,
        queue_depth: 0,
        extraction_model: 'gemma4:26b',
        classification_model: 'gemma4:e4b',
      };
      fetchSpy.mockResolvedValueOnce(mockResponse(health));
      const result = await getPipelineHealth();
      expect(result.facts_current).toBe(10);
      expect(fetchSpy).toHaveBeenCalledWith('/dashboard/api/health', undefined);
    });

    it('getCompilations builds query string', async () => {
      fetchSpy.mockResolvedValueOnce(mockResponse([]));
      await getCompilations({ namespace: 'test', limit: 10 });
      const url = fetchSpy.mock.calls[0][0] as string;
      expect(url).toContain('/dashboard/api/compilations?');
      expect(url).toContain('namespace=test');
      expect(url).toContain('limit=10');
    });

    it('getCompilationDetail encodes id', async () => {
      fetchSpy.mockResolvedValueOnce(mockResponse({ id: 'abc' }));
      await getCompilationDetail('abc-123');
      expect(fetchSpy).toHaveBeenCalledWith('/dashboard/api/compilations/abc-123', undefined);
    });

    it('getEntities builds query string with filters', async () => {
      fetchSpy.mockResolvedValueOnce(mockResponse([]));
      await getEntities({ namespace: 'ns', entity_type: 'person', q: 'alice' });
      const url = fetchSpy.mock.calls[0][0] as string;
      expect(url).toContain('namespace=ns');
      expect(url).toContain('entity_type=person');
      expect(url).toContain('q=alice');
    });

    it('getEntityDetail calls correct path', async () => {
      fetchSpy.mockResolvedValueOnce(mockResponse({ id: 'e1', name: 'Test' }));
      await getEntityDetail('e1');
      expect(fetchSpy).toHaveBeenCalledWith('/dashboard/api/entities/e1', undefined);
    });

    it('getEntityGraph calls correct path', async () => {
      fetchSpy.mockResolvedValueOnce(mockResponse({ root_entity_id: 'e1', nodes: [], edges: [] }));
      await getEntityGraph('e1');
      expect(fetchSpy).toHaveBeenCalledWith('/dashboard/api/entities/e1/graph', undefined);
    });

    it('getConflicts calls /conflicts', async () => {
      fetchSpy.mockResolvedValueOnce(mockResponse([]));
      await getConflicts();
      expect(fetchSpy).toHaveBeenCalledWith('/dashboard/api/conflicts', undefined);
    });

    it('getPredicateCandidates calls correct path', async () => {
      fetchSpy.mockResolvedValueOnce(mockResponse([]));
      await getPredicateCandidates();
      expect(fetchSpy).toHaveBeenCalledWith('/dashboard/api/predicates/candidates', undefined);
    });

    it('getPredicatePacks calls correct path', async () => {
      fetchSpy.mockResolvedValueOnce(mockResponse([]));
      await getPredicatePacks();
      expect(fetchSpy).toHaveBeenCalledWith('/dashboard/api/predicates/packs', undefined);
    });

    it('getRetrievalMetrics calls correct path', async () => {
      fetchSpy.mockResolvedValueOnce(
        mockResponse({
          daily_precision: [],
          latency_p50: null,
          latency_p95: null,
          latency_p99: null,
        }),
      );
      await getRetrievalMetrics();
      expect(fetchSpy).toHaveBeenCalledWith('/dashboard/api/metrics/retrieval', undefined);
    });
  });

  describe('POST endpoints', () => {
    it('resolveConflict sends correct request', async () => {
      const response = {
        id: 'c1',
        resolved: true,
        resolution: 'kept_separate',
        resolved_at: '2025-01-01',
      };
      fetchSpy.mockResolvedValueOnce(mockResponse(response));
      const result = await resolveConflict('c1', { resolution: 'kept_separate' });
      expect(result.resolved).toBe(true);
      expect(fetchSpy).toHaveBeenCalledWith('/dashboard/api/conflicts/c1/resolve', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ resolution: 'kept_separate' }),
      });
    });

    it('resolvePredicateCandidate sends promote request', async () => {
      const response = {
        id: 'p1',
        predicate: 'custom_pred',
        action: 'promote',
        mapped_to: null,
        promoted_to_pack: 'core',
        resolved_at: '2025-01-01',
      };
      fetchSpy.mockResolvedValueOnce(mockResponse(response));
      const result = await resolvePredicateCandidate('p1', {
        action: 'promote',
        target_pack: 'core',
        category: 'structural',
      });
      expect(result.promoted_to_pack).toBe('core');
    });
  });
});
