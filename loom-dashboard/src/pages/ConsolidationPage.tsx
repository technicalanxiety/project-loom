/**
 * Consolidation health page.
 *
 * Shows:
 * - Consolidation pipeline activity (synthesis runs, clustering metrics)
 * - Pruning activity (stale procedures, auto-resolved conflicts)
 * - Knowledge summary inventory and status
 * - Recent consolidation/pruning run history
 * - Manual "run consolidation now" button per namespace
 */
import type React from 'react';
import { useState } from 'react';
import { getConsolidationHealth, runConsolidationNow } from '../api/client';
import { useApi } from '../hooks/useApi';
import { useNamespaces } from '../hooks/useNamespaces';
import type { ConsolidationHealthResponse, ConsolidationRun } from '../types';

export const ConsolidationPage: React.FC = () => {
  const [namespace, setNamespace] = useState<string>('');
  const { data: namespaces } = useNamespaces();
  const [running, setRunning] = useState(false);

  const { data, loading, error, refetch } = useApi<ConsolidationHealthResponse | null>(
    () => (namespace ? getConsolidationHealth(namespace) : Promise.resolve(null)),
    [namespace],
  );

  const handleRunNow = async () => {
    if (!namespace) return;
    try {
      setRunning(true);
      await runConsolidationNow(namespace);
      setTimeout(() => refetch(), 1000);
    } catch (err) {
      // error state is managed by useApi on next reload
    } finally {
      setRunning(false);
    }
  };

  return (
    <>
      <PageHeader />

      <div className="filter-bar">
        <select
          className="form-control"
          value={namespace}
          onChange={(e) => setNamespace(e.target.value)}
        >
          <option value="">Select namespace…</option>
          {namespaces?.map((ns) => (
            <option key={ns.namespace} value={ns.namespace}>
              {ns.namespace}
            </option>
          ))}
        </select>
      </div>

      {!namespace && (
        <div className="empty-state">
          <h3>Select a namespace</h3>
          <p>Choose a namespace above to view consolidation health.</p>
        </div>
      )}

      {namespace && loading && <div className="loading">Loading consolidation health…</div>}

      {namespace && error && <div className="error">Error: {error}</div>}

      {namespace && !loading && !error && !data && (
        <div className="empty-state">
          <h3>No consolidation data</h3>
          <p>The consolidation pipeline has not run yet for this namespace.</p>
        </div>
      )}

      {data && (
        <>
          <div className="kpi-grid">
            <div className="kpi accent">
              <div className="kpi-eyebrow">Active Summaries</div>
              <div className="kpi-value numeric">{data.active_summaries.toLocaleString()}</div>
              <div className="kpi-sub">of {data.total_summaries.toLocaleString()} total</div>
            </div>
            <div className="kpi">
              <div className="kpi-eyebrow">Invalidated</div>
              <div className="kpi-value numeric">{data.invalidated_summaries.toLocaleString()}</div>
              <div className="kpi-sub">awaiting re-synthesis</div>
            </div>
            {data.latest_consolidation_run && (
              <div className="kpi">
                <div className="kpi-eyebrow">Latest consolidation</div>
                <div className="kpi-value">{relativeTime(data.latest_consolidation_run.started_at)}</div>
                <div
                  className="kpi-sub"
                  style={{ color: data.latest_consolidation_run.status === 'completed' ? 'inherit' : 'var(--madder-600)' }}
                >
                  {data.latest_consolidation_run.status}
                </div>
              </div>
            )}
            {data.latest_pruning_run && (
              <div className="kpi">
                <div className="kpi-eyebrow">Latest pruning</div>
                <div className="kpi-value">{relativeTime(data.latest_pruning_run.started_at)}</div>
                <div
                  className="kpi-sub"
                  style={{ color: data.latest_pruning_run.status === 'completed' ? 'inherit' : 'var(--madder-600)' }}
                >
                  {data.latest_pruning_run.status}
                </div>
              </div>
            )}
          </div>

          <div className="section-head" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
            <h3>Recent activity</h3>
            <button onClick={handleRunNow} disabled={running} className="btn btn-primary">
              {running ? 'Running…' : 'Run consolidation now'}
            </button>
          </div>

          <table className="tbl">
            <thead>
              <tr>
                <th>Type</th>
                <th>Status</th>
                <th>Started</th>
                <th>Duration</th>
                <th>Details</th>
              </tr>
            </thead>
            <tbody>
              {data.recent_runs.map((run) => (
                <ConsolidationRunRow key={run.id} run={run} />
              ))}
            </tbody>
          </table>
        </>
      )}
    </>
  );
};

function ConsolidationRunRow({ run }: { run: ConsolidationRun }) {
  const details =
    run.run_type === 'consolidation'
      ? `${run.summaries_created || 0} created, ${run.summaries_refreshed || 0} refreshed from ${run.clusters_found || 0} clusters`
      : `${run.procedures_pruned || 0} procedures, ${run.conflicts_resolved || 0} conflicts, ${run.summaries_invalidated || 0} summaries`;

  return (
    <tr>
      <td className="cell-id" style={{ textTransform: 'capitalize' }}>
        {run.run_type}
      </td>
      <td>
        <span
          className={`pill ${run.status === 'completed' ? 'pill-success' : run.status === 'failed' ? 'pill-error' : 'pill-warning'}`}
        >
          <span className="dot" />
          {run.status}
        </span>
      </td>
      <td className="cell-muted" title={run.started_at} style={{ whiteSpace: 'nowrap' }}>
        {relativeTime(run.started_at)}
      </td>
      <td className="cell-num">{run.duration_ms ? `${run.duration_ms}ms` : '—'}</td>
      <td className="cell-muted" style={{ fontSize: '0.9em' }}>
        {details}
      </td>
    </tr>
  );
}

function PageHeader() {
  return (
    <div className="page-header">
      <div className="page-header-titles">
        <div className="page-eyebrow">Memory consolidation</div>
        <h2>Consolidation health</h2>
        <p>
          Knowledge summary synthesis and stale-artifact pruning. Consolidation transforms clusters of facts into
          higher-order summaries, and pruning removes low-value procedures and unresolved conflicts.
        </p>
      </div>
    </div>
  );
}

function relativeTime(timestamp: string | null): string {
  if (!timestamp) return '—';
  const date = new Date(timestamp);
  const now = new Date();
  const seconds = Math.floor((now.getTime() - date.getTime()) / 1000);

  if (seconds < 60) return 'just now';
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
  return `${Math.floor(seconds / 86400)}d ago`;
}
