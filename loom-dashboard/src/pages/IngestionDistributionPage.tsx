/**
 * Ingestion-mode distribution page.
 *
 * Breaks down every namespace's episodes by ingestion mode and surfaces
 * seed-only namespaces as a warning list. A namespace whose episodes are
 * 100% user_authored_seed will have every compiled fact carry
 * sole_source=true — the user should know which namespaces are in that
 * state and either corroborate them via live capture or accept the flag.
 *
 * Reads `/dashboard/api/metrics/ingestion-distribution`.
 */
import type React from 'react';
import { getIngestionDistributionMetrics } from '../api/client';
import { useApi } from '../hooks/useApi';
import type { IngestionDistributionRow } from '../types';

const MODE_ORDER: Array<IngestionDistributionRow['ingestion_mode']> = [
  'live_mcp_capture',
  'user_authored_seed',
  'vendor_import',
];

const MODE_LABEL: Record<IngestionDistributionRow['ingestion_mode'], string> = {
  live_mcp_capture: 'Live capture',
  user_authored_seed: 'Seed',
  vendor_import: 'Vendor import',
};

/** Collapse long-form rows into a namespace-indexed grid for rendering. */
function groupByNamespace(
  rows: IngestionDistributionRow[],
): Map<string, Map<IngestionDistributionRow['ingestion_mode'], number>> {
  const grid = new Map<string, Map<IngestionDistributionRow['ingestion_mode'], number>>();
  for (const r of rows) {
    const modes = grid.get(r.namespace) ?? new Map();
    modes.set(r.ingestion_mode, r.episode_count);
    grid.set(r.namespace, modes);
  }
  return grid;
}

/** Ingestion-mode distribution page component. */
export const IngestionDistributionPage: React.FC = () => {
  const { data, loading, error } = useApi(getIngestionDistributionMetrics);

  if (loading) {
    return <p>Loading ingestion distribution…</p>;
  }
  if (error) {
    return <p style={{ color: '#c44' }}>Error: {error}</p>;
  }
  if (!data) {
    return null;
  }

  const grid = groupByNamespace(data.rows);
  const namespaces = Array.from(grid.keys()).sort();

  return (
    <div>
      <h2>Ingestion Mode Distribution</h2>
      <p style={{ color: '#888', fontSize: '0.85rem', maxWidth: 640 }}>
        Episode counts per namespace split by ingestion mode. Seed-only namespaces are listed
        separately — every compiled fact from those namespaces carries <code>sole_source=true</code>
        , meaning it has no live or vendor corroboration.
      </p>

      {data.seed_only_namespaces.length > 0 && (
        <div
          style={{
            marginTop: '1rem',
            padding: '1rem',
            background: '#3a2a1a',
            border: '1px solid #6a4a2a',
            borderRadius: '4px',
            fontSize: '0.85rem',
          }}
        >
          <strong style={{ color: '#e0b070' }}>Seed-only namespaces</strong>
          <p style={{ margin: '0.5rem 0', color: '#d0a060' }}>
            These namespaces have no live-captured or vendor-imported episodes. Compilation will
            flag every fact from them as sole-source.
          </p>
          <ul style={{ margin: 0, paddingLeft: '1.5rem' }}>
            {data.seed_only_namespaces.map((ns) => (
              <li key={ns.namespace}>
                <code>{ns.namespace}</code> — {ns.seed_episode_count} seed episodes
              </li>
            ))}
          </ul>
        </div>
      )}

      <table
        style={{
          width: '100%',
          maxWidth: 900,
          borderCollapse: 'collapse',
          marginTop: '1rem',
          fontSize: '0.85rem',
        }}
      >
        <thead>
          <tr style={{ textAlign: 'left', borderBottom: '1px solid #333' }}>
            <th style={{ padding: '0.5rem' }}>Namespace</th>
            {MODE_ORDER.map((mode) => (
              <th key={mode} style={{ padding: '0.5rem', textAlign: 'right' }}>
                {MODE_LABEL[mode]}
              </th>
            ))}
            <th style={{ padding: '0.5rem', textAlign: 'right' }}>Total</th>
          </tr>
        </thead>
        <tbody>
          {namespaces.map((ns) => {
            const modeMap =
              grid.get(ns) ?? new Map<IngestionDistributionRow['ingestion_mode'], number>();
            const total = Array.from(modeMap.values()).reduce((a, b) => a + b, 0);
            return (
              <tr key={ns} style={{ borderBottom: '1px solid #222' }}>
                <td style={{ padding: '0.5rem' }}>
                  <code>{ns}</code>
                </td>
                {MODE_ORDER.map((mode) => (
                  <td
                    key={mode}
                    style={{
                      padding: '0.5rem',
                      textAlign: 'right',
                      color: modeMap.get(mode) ? '#eee' : '#555',
                    }}
                  >
                    {(modeMap.get(mode) ?? 0).toLocaleString()}
                  </td>
                ))}
                <td style={{ padding: '0.5rem', textAlign: 'right', fontWeight: 600 }}>
                  {total.toLocaleString()}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
};
