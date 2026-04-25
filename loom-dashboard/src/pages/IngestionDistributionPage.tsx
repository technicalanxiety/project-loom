/**
 * Ingestion-mode distribution page.
 *
 * Per-namespace breakdown of where episodes come from — live capture,
 * user seed, or vendor import. Seed-only namespaces are surfaced as a
 * warning callout: every compiled fact from such a namespace will carry
 * `sole_source=true` because there is no live or vendor corroboration.
 *
 * Reads `/dashboard/api/metrics/ingestion-distribution`.
 */
import type React from 'react';
import { getIngestionDistributionMetrics } from '../api/client';
import { useApi } from '../hooks/useApi';
import type { IngestionDistributionRow } from '../types';

type Mode = IngestionDistributionRow['ingestion_mode'];

const MODE_ORDER: Mode[] = ['live_mcp_capture', 'user_authored_seed', 'vendor_import'];
const MODE_SEG: Record<Mode, string> = {
  live_mcp_capture: 'dist-seg-live',
  user_authored_seed: 'dist-seg-seed',
  vendor_import: 'dist-seg-vendor',
};
const MODE_LABEL: Record<Mode, string> = {
  live_mcp_capture: 'live',
  user_authored_seed: 'seed',
  vendor_import: 'vendor',
};

/** Collapse the long-form rows into a namespace-indexed grid. */
function groupByNamespace(rows: IngestionDistributionRow[]): Map<string, Map<Mode, number>> {
  const grid = new Map<string, Map<Mode, number>>();
  for (const r of rows) {
    const modes = grid.get(r.namespace) ?? new Map();
    modes.set(r.ingestion_mode, r.episode_count);
    grid.set(r.namespace, modes);
  }
  return grid;
}

export const IngestionDistributionPage: React.FC = () => {
  const { data, loading, error } = useApi(getIngestionDistributionMetrics);

  if (loading) {
    return (
      <>
        <PageHeader />
        <div className="loading">Loading ingestion distribution…</div>
      </>
    );
  }
  if (error) {
    return (
      <>
        <PageHeader />
        <div className="error">Error: {error}</div>
      </>
    );
  }
  if (!data) {
    return (
      <>
        <PageHeader />
        <div className="empty-state">
          <h3>No ingestion data yet</h3>
          <p>
            Once episodes are ingested, this page breaks them down by namespace and source mode.
          </p>
        </div>
      </>
    );
  }

  const grid = groupByNamespace(data.rows);
  const namespaces = Array.from(grid.keys()).sort();

  // Aggregate KPIs.
  let total = 0;
  const modeTotals = new Map<Mode, number>();
  for (const r of data.rows) {
    total += r.episode_count;
    modeTotals.set(r.ingestion_mode, (modeTotals.get(r.ingestion_mode) ?? 0) + r.episode_count);
  }
  const pct = (m: Mode) => (total > 0 ? Math.round(((modeTotals.get(m) ?? 0) / total) * 100) : 0);

  // Per-namespace max for bar scaling — equal-width bars are easier to scan
  // when each namespace fills the row independently. We use the namespace's
  // own total so each row's segments sum to 100%.

  return (
    <>
      <PageHeader />

      <div className="kpi-grid">
        <div className="kpi accent">
          <div className="kpi-eyebrow">Episodes</div>
          <div className="kpi-value numeric">{total.toLocaleString()}</div>
          <div className="kpi-sub">{namespaces.length} namespaces</div>
        </div>
        <div className="kpi">
          <div className="kpi-eyebrow">% live</div>
          <div className="kpi-value numeric" style={{ color: 'var(--moss-700)' }}>
            {pct('live_mcp_capture')}
            <span className="kpi-unit">%</span>
          </div>
          <div className="kpi-sub">
            {(modeTotals.get('live_mcp_capture') ?? 0).toLocaleString()}
          </div>
        </div>
        <div className="kpi">
          <div className="kpi-eyebrow">% seed</div>
          <div className="kpi-value numeric" style={{ color: 'var(--saffron-700)' }}>
            {pct('user_authored_seed')}
            <span className="kpi-unit">%</span>
          </div>
          <div className="kpi-sub">
            {(modeTotals.get('user_authored_seed') ?? 0).toLocaleString()}
          </div>
        </div>
        <div className="kpi">
          <div className="kpi-eyebrow">% vendor</div>
          <div className="kpi-value numeric" style={{ color: 'var(--indigo-600)' }}>
            {pct('vendor_import')}
            <span className="kpi-unit">%</span>
          </div>
          <div className="kpi-sub">{(modeTotals.get('vendor_import') ?? 0).toLocaleString()}</div>
        </div>
      </div>

      {data.seed_only_namespaces.length > 0 && (
        <div className="callout callout-warn">
          <div className="callout-title">
            {data.seed_only_namespaces.length} seed-only{' '}
            {data.seed_only_namespaces.length === 1 ? 'namespace' : 'namespaces'}
          </div>
          <div className="callout-body">
            Every compiled fact from these namespaces is flagged <code>sole_source</code>:{' '}
            {data.seed_only_namespaces.map((ns, i) => (
              <span key={ns.namespace}>
                {i > 0 && ', '}
                <code>{ns.namespace}</code> ({ns.seed_episode_count.toLocaleString()})
              </span>
            ))}
            .
          </div>
        </div>
      )}

      <div className="section-head">
        <h3>
          Per-namespace breakdown{' '}
          <span className="count-pill">{namespaces.length.toLocaleString()}</span>
        </h3>
        <div className="dist-legend">
          {MODE_ORDER.map((m) => (
            <span key={m}>
              <i
                className={`${MODE_SEG[m].replace('dist-seg', 'dist-seg')}`}
                style={legendSwatch(m)}
              />
              {MODE_LABEL[m]}
            </span>
          ))}
        </div>
      </div>

      <section className="panel">
        {namespaces.map((ns) => {
          const modes = grid.get(ns) ?? new Map();
          const rowTotal = MODE_ORDER.reduce((sum, m) => sum + (modes.get(m) ?? 0), 0);
          return (
            <div key={ns} className="dist-row">
              <span className="dist-name" title={ns}>
                {ns}
              </span>
              <div className="dist-stack">
                {MODE_ORDER.map((m) => {
                  const count = modes.get(m) ?? 0;
                  if (count === 0) return null;
                  const segPct = rowTotal > 0 ? (count / rowTotal) * 100 : 0;
                  return (
                    <span
                      key={m}
                      className={MODE_SEG[m]}
                      style={{ width: `${segPct}%` }}
                      title={`${MODE_LABEL[m]}: ${count.toLocaleString()}`}
                    />
                  );
                })}
              </div>
              <span className="dist-total">{rowTotal.toLocaleString()}</span>
            </div>
          );
        })}
      </section>
    </>
  );
};

function PageHeader() {
  return (
    <div className="page-header">
      <div className="page-header-titles">
        <div className="page-eyebrow">Ingestion / Distribution</div>
        <h2>Episode source distribution</h2>
        <p>
          Where each namespace's episodes come from. Seed-only namespaces flag every compiled fact
          as <code>sole_source</code>.
        </p>
      </div>
    </div>
  );
}

function legendSwatch(mode: Mode): React.CSSProperties {
  switch (mode) {
    case 'live_mcp_capture':
      return { background: 'var(--moss-500)' };
    case 'user_authored_seed':
      return { background: 'var(--saffron-500)' };
    case 'vendor_import':
      return { background: 'var(--indigo-400)' };
  }
}
