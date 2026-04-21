/**
 * Compilation trace list page.
 *
 * Shows a paginated table of loom_think compilation traces
 * with links to individual trace detail views.
 */
import type React from 'react';
import { Link } from 'react-router-dom';
import { getCompilations } from '../api/client';
import { useApi } from '../hooks/useApi';

/** Paginated list of compilation traces. */
export const CompilationsPage: React.FC = () => {
  const { data, loading, error } = useApi(() => getCompilations({ limit: 50 }), []);

  return (
    <div>
      <div className="page-header">
        <h2>Compilations</h2>
        <p>Trace log of loom_think compilation requests.</p>
      </div>

      {loading && <p className="loading">Loading compilations…</p>}
      {error && <p className="error">{error}</p>}

      {data && (
        <div className="card" style={{ overflowX: 'auto' }}>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '0.85rem' }}>
            <thead>
              <tr style={{ borderBottom: '2px solid #eee', textAlign: 'left' }}>
                <th style={{ padding: '0.5rem' }}>Time</th>
                <th style={{ padding: '0.5rem' }}>Namespace</th>
                <th style={{ padding: '0.5rem' }}>Query</th>
                <th style={{ padding: '0.5rem' }}>Class</th>
                <th style={{ padding: '0.5rem' }}>Confidence</th>
                <th style={{ padding: '0.5rem' }}>Tokens</th>
                <th style={{ padding: '0.5rem' }}>Latency</th>
              </tr>
            </thead>
            <tbody>
              {data.map((c) => (
                <tr key={c.id} style={{ borderBottom: '1px solid #f0f0f0' }}>
                  <td style={{ padding: '0.5rem', whiteSpace: 'nowrap' }}>
                    {new Date(c.created_at).toLocaleString()}
                  </td>
                  <td style={{ padding: '0.5rem' }}>{c.namespace}</td>
                  <td
                    style={{
                      padding: '0.5rem',
                      maxWidth: '300px',
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                    }}
                  >
                    <Link to={`/compilations/${c.id}`} style={{ color: '#3a3a6a' }}>
                      {c.query_text ?? '—'}
                    </Link>
                  </td>
                  <td style={{ padding: '0.5rem' }}>{c.task_class}</td>
                  <td style={{ padding: '0.5rem' }}>
                    {c.primary_confidence != null
                      ? `${(c.primary_confidence * 100).toFixed(0)}%`
                      : '—'}
                  </td>
                  <td style={{ padding: '0.5rem' }}>{c.compiled_tokens ?? '—'}</td>
                  <td style={{ padding: '0.5rem' }}>
                    {c.latency_total_ms != null ? `${c.latency_total_ms}ms` : '—'}
                  </td>
                </tr>
              ))}
              {data.length === 0 && (
                <tr>
                  <td colSpan={7} className="placeholder">
                    No compilations found.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
};
