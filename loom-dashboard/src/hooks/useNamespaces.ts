/**
 * Hook that fetches the list of configured namespaces.
 *
 * Used by filter dropdowns across multiple views so each view
 * doesn't need to fetch namespaces independently.
 */
import { getNamespaces } from '../api/client';
import type { NamespaceInfo } from '../types';
import { useApi } from './useApi';

/**
 * Fetch all configured namespaces from the dashboard API.
 *
 * @returns An {@link ApiState} containing the namespace list.
 *
 * @example
 * ```tsx
 * const { data: namespaces, loading } = useNamespaces();
 * ```
 */
export function useNamespaces() {
  return useApi<NamespaceInfo[]>(() => getNamespaces(), []);
}
