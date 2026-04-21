/**
 * Generic data-fetching hook for the dashboard API.
 *
 * Wraps `useState` / `useEffect` to provide a consistent loading, error,
 * and data pattern across all views.
 */
import { useCallback, useEffect, useState } from 'react';

/** The three possible states of an API request. */
export interface ApiState<T> {
  /** The fetched data, or `null` while loading / on error. */
  data: T | null;
  /** Whether a request is currently in flight. */
  loading: boolean;
  /** An error message if the request failed, otherwise `null`. */
  error: string | null;
  /** Re-trigger the fetch (e.g. after a mutation). */
  refetch: () => void;
}

/**
 * Fetch data from the API on mount (and whenever `deps` change).
 *
 * @param fetcher - An async function that returns the data.
 * @param deps - Dependency array that triggers a re-fetch when changed.
 * @returns An {@link ApiState} object with `data`, `loading`, `error`, and `refetch`.
 *
 * @example
 * ```tsx
 * const { data, loading, error } = useApi(() => getPipelineHealth(), []);
 * ```
 */
export function useApi<T>(fetcher: () => Promise<T>, deps: React.DependencyList = []): ApiState<T> {
  const [data, setData] = useState<T | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [trigger, setTrigger] = useState(0);

  const refetch = useCallback(() => {
    setTrigger((prev) => prev + 1);
  }, []);

  // biome-ignore lint/correctness/useExhaustiveDependencies: fetcher is intentionally excluded (callers pass inline arrows); trigger drives refetch
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);

    fetcher()
      .then((result) => {
        if (!cancelled) {
          setData(result);
          setLoading(false);
        }
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          const message = err instanceof Error ? err.message : 'Unknown error';
          setError(message);
          setLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [trigger, ...deps]);

  return { data, loading, error, refetch };
}
