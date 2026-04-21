/**
 * Predicate pack detail page.
 *
 * Shows all predicates in a pack with their categories,
 * inverse relationships, and usage counts.
 */
import type React from 'react';
import { Link, useParams } from 'react-router-dom';
import { getPredicatePackDetail } from '../api/client';
import { useApi } from '../hooks/useApi';

/** Detail view for a single predicate pack. */
export const PackDetailPage: React.FC = () => {
  const { pack } = useParams<{ pack: string }>();
  // biome-ignore lint/style/noNonNullAssertion: pack guaranteed by route param
  const { data, loading, error } = useApi(() => getPredicatePackDetail(pack!), [pack]);

  return (
    <div>
      <div className="page-header">
        <h2>Pack: {pack}</h2>
        <p>
          <Link to="/predicates" style={{ color: '#3a3a6a' }}>
            ← Back to predicates
          </Link>
        </p>
      </div>

      {loading && <p className="loading">Loading…</p>}
      {error && <p className="error">{error}</p>}

      {data && (
        <div className="card" style={{ overflowX: 'auto' }}>
          {data.description && (
            <p style={{ fontSize: '0.85rem', color: '#666', marginBottom: '1rem' }}>
              {data.description}
            </p>
          )}
          <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '0.85rem' }}>
            <thead>
              <tr style={{ borderBottom: '2px solid #eee', textAlign: 'left' }}>
                <th style={{ padding: '0.5rem' }}>Predicate</th>
                <th style={{ padding: '0.5rem' }}>Category</th>
                <th style={{ padding: '0.5rem' }}>Inverse</th>
                <th style={{ padding: '0.5rem' }}>Description</th>
                <th style={{ padding: '0.5rem' }}>Usage</th>
              </tr>
            </thead>
            <tbody>
              {data.predicates.map((p) => (
                <tr key={p.predicate} style={{ borderBottom: '1px solid #f0f0f0' }}>
                  <td style={{ padding: '0.5rem', fontFamily: 'monospace', fontSize: '0.8rem' }}>
                    {p.predicate}
                  </td>
                  <td style={{ padding: '0.5rem' }}>{p.category}</td>
                  <td style={{ padding: '0.5rem', fontFamily: 'monospace', fontSize: '0.8rem' }}>
                    {p.inverse ?? '—'}
                  </td>
                  <td style={{ padding: '0.5rem', maxWidth: '300px' }}>{p.description ?? '—'}</td>
                  <td style={{ padding: '0.5rem' }}>{p.usage_count}</td>
                </tr>
              ))}
              {data.predicates.length === 0 && (
                <tr>
                  <td colSpan={5} className="placeholder">
                    No predicates in this pack.
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
