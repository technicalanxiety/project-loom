/**
 * Compilation trace list page.
 *
 * The operator's debug surface for `loom_think`. The table is the
 * primary read; signals (latency, confidence, class, tokens) are
 * pilled or bar-scaled so a slow row jumps off the page.
 */

import type React from 'react';
import { useMemo } from 'react';
import { Link } from 'react-router-dom';
import { getCompilations } from '../api/client';
import { useApi } from '../hooks/useApi';
import { confidenceTone, latencyTone, relativeTime, TONE_CLASS } from '../lib/thresholds';
import type { CompilationSummary } from '../types';

const TASK_CLASS_PILL: Record<string, string> = {
  structural: 'pill-info',
  temporal: 'pill-success',
  decisional: 'pill-vendor',
  operational: 'pill-warning',
  regulatory: 'pill-error',
};

function classPillFor(taskClass: string): string {
  return TASK_CLASS_PILL[taskClass] ?? 'pill-neutral';
}

/** Median value of a numeric array — null when the array is empty. */
function median(values: number[]): number | null {
  if (values.length === 0) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0 ? (sorted[mid - 1] + sorted[mid]) / 2 : sorted[mid];
}

function percentile(values: number[], p: number): number | null {
  if (values.length === 0) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const idx = Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length));
  return sorted[idx];
}

export const CompilationsPage: React.FC = () => {
  const { data, loading, error } = useApi(() => getCompilations({ limit: 50 }), []);

  const stats = useMemo(() => {
    if (!data) return null;
    const latencies = data.map((c) => c.latency_total_ms).filter((v): v is number => v !== null);
    const tokens = data.map((c) => c.compiled_tokens).filter((v): v is number => v !== null);
    return {
      count: data.length,
      p50: percentile(latencies, 50),
      p95: percentile(latencies, 95),
      tokens: median(tokens),
    };
  }, [data]);

  return (
    <>
      <div className="page-header">
        <div className="page-header-titles">
          <div className="page-eyebrow">Knowledge / Compilations</div>
          <h2>Compilations</h2>
          <p>Trace log of every {`loom_think`} compilation. Click a row to see the full trace.</p>
        </div>
      </div>

      {loading && <div className="loading">Loading compilations…</div>}
      {error && <div className="error">{error}</div>}

      {data && stats && (
        <>
          <div className="kpi-grid">
            <div className="kpi accent">
              <div className="kpi-eyebrow">Recent</div>
              <div className="kpi-value numeric">{stats.count}</div>
              <div className="kpi-sub">compilations</div>
            </div>
            <div className="kpi">
              <div className="kpi-eyebrow">p50 latency</div>
              <div className="kpi-value numeric">
                {stats.p50 !== null ? Math.round(stats.p50) : '—'}
                <span className="kpi-unit">ms</span>
              </div>
            </div>
            <div className="kpi">
              <div className="kpi-eyebrow">p95 latency</div>
              <div
                className="kpi-value numeric"
                style={
                  stats.p95 !== null && latencyTone(stats.p95) !== 'ok'
                    ? {
                        color:
                          latencyTone(stats.p95) === 'crit'
                            ? 'var(--signal-error)'
                            : 'var(--signal-warning)',
                      }
                    : undefined
                }
              >
                {stats.p95 !== null ? Math.round(stats.p95) : '—'}
                <span className="kpi-unit">ms</span>
              </div>
            </div>
            <div className="kpi">
              <div className="kpi-eyebrow">Median tokens</div>
              <div className="kpi-value numeric">
                {stats.tokens !== null ? Math.round(stats.tokens).toLocaleString() : '—'}
              </div>
            </div>
          </div>

          <div className="section-head">
            <h3>
              Trace log <span className="count-pill">{data.length}</span>
            </h3>
          </div>

          {data.length === 0 ? (
            <div className="empty-state">
              <h3>No compilations yet</h3>
              <p>
                Once a client calls <code>loom_think</code>, every compilation lands here with full
                stage timings and candidate selections.
              </p>
            </div>
          ) : (
            <table className="tbl">
              <thead>
                <tr>
                  <th>Time</th>
                  <th>Namespace</th>
                  <th>Query</th>
                  <th>Class</th>
                  <th>Conf</th>
                  <th className="cell-num">Tokens</th>
                  <th className="cell-num">Latency</th>
                </tr>
              </thead>
              <tbody>
                {data.map((c) => (
                  <CompilationRow key={c.id} c={c} />
                ))}
              </tbody>
            </table>
          )}
        </>
      )}
    </>
  );
};

function CompilationRow({ c }: { c: CompilationSummary }) {
  const conf = c.primary_confidence;
  const lat = c.latency_total_ms;
  const latToneVal = latencyTone(lat);
  const confToneVal = conf !== null ? confidenceTone(conf) : null;
  return (
    <tr>
      <td className="cell-muted" title={new Date(c.created_at).toLocaleString()}>
        {relativeTime(c.created_at)}
      </td>
      <td>
        <span className="pill pill-vendor">
          <span className="dot" />
          {c.namespace}
        </span>
      </td>
      <td
        style={{
          maxWidth: 320,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        <Link to={`/compilations/${c.id}`} className="cell-id" title={c.query_text ?? undefined}>
          {c.query_text ?? '—'}
        </Link>
      </td>
      <td>
        <span className={`pill ${classPillFor(c.task_class)}`}>{c.task_class}</span>
      </td>
      <td>
        {conf !== null && confToneVal ? (
          <span
            style={{ display: 'inline-flex', alignItems: 'center', gap: 6, whiteSpace: 'nowrap' }}
          >
            <span
              className={`inline-bar ${TONE_CLASS[confToneVal]}`}
              style={{ width: 50 }}
              aria-hidden="true"
            >
              <span style={{ width: `${conf * 100}%` }} />
            </span>
            <span className="cell-num" style={{ minWidth: 32 }}>
              {(conf * 100).toFixed(0)}%
            </span>
          </span>
        ) : (
          <span className="cell-muted">—</span>
        )}
      </td>
      <td className="cell-num">{c.compiled_tokens?.toLocaleString() ?? '—'}</td>
      <td className="cell-num">
        {lat === null ? (
          <span className="cell-muted">—</span>
        ) : (
          <span
            style={{ display: 'inline-flex', alignItems: 'center', gap: 6, whiteSpace: 'nowrap' }}
          >
            <span
              className={`inline-bar ${TONE_CLASS[latToneVal]}`}
              style={{ width: 50 }}
              aria-hidden="true"
            >
              <span style={{ width: `${Math.min((lat / 2000) * 100, 100)}%` }} />
            </span>
            <span
              style={{
                color:
                  latToneVal === 'crit'
                    ? 'var(--signal-error)'
                    : latToneVal === 'warn'
                      ? 'var(--signal-warning)'
                      : 'var(--fg-1)',
                fontWeight: latToneVal !== 'ok' ? 600 : 400,
              }}
            >
              {lat.toLocaleString()}
            </span>
          </span>
        )}
      </td>
    </tr>
  );
}
