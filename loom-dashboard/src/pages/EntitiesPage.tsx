/**
 * Entity list page.
 *
 * Displays a searchable, filterable table of entities
 * with links to individual entity detail views.
 */
import type React from 'react';
import { useState } from 'react';
import { Link } from 'react-router-dom';
import { getEntities } from '../api/client';
import { useApi } from '../hooks/useApi';
import { useNamespaces } from '../hooks/useNamespaces';

/** Paginated entity list with namespace and type filters. */
export const EntitiesPage: React.FC = () => {
  const [namespace, setNamespace] = useState<string>('');
  const [search, setSearch] = useState('');
  const { data: namespaces } = useNamespaces();

  const { data, loading, error } = useApi(
    () =>
      getEntities({
        namespace: namespace || undefined,
        q: search || undefined,
        limit: 50,
      }),
    [namespace, search],
  );

  return (
    <div>
      <div className="page-header">
        <h2>Entities</h2>
        <p>Browse and search knowledge-graph entities.</p>
      </div>

      <div style={{ display: 'flex', gap: '0.75rem', marginBottom: '1rem', flexWrap: 'wrap' }}>
        <select
          value={namespace}
          onChange={(e) => setNamespace(e.target.value)}
          style={{
            padding: '0.4rem 0.6rem',
            borderRadius: '4px',
            border: '1px solid #ccc',
            fontSize: '0.85rem',
          }}
        >
          <option value="">All namespaces</option>
          {namespaces?.map((ns) => (
            <option key={ns.namespace} value={ns.namespace}>
              {ns.namespace}
            </option>
          ))}
        </select>
        <input
          type="text"
          placeholder="Search entities…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          style={{
            padding: '0.4rem 0.6rem',
            borderRadius: '4px',
            border: '1px solid #ccc',
            fontSize: '0.85rem',
            minWidth: '200px',
          }}
        />
      </div>

      {loading && <p className="loading">Loading entities…</p>}
      {error && <p className="error">{error}</p>}

      {data && (
        <div className="card" style={{ overflowX: 'auto' }}>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '0.85rem' }}>
            <thead>
              <tr style={{ borderBottom: '2px solid #eee', textAlign: 'left' }}>
                <th style={{ padding: '0.5rem' }}>Name</th>
                <th style={{ padding: '0.5rem' }}>Type</th>
                <th style={{ padding: '0.5rem' }}>Namespace</th>
                <th style={{ padding: '0.5rem' }}>Tier</th>
                <th style={{ padding: '0.5rem' }}>Salience</th>
              </tr>
            </thead>
            <tbody>
              {data.map((e) => (
                <tr key={e.id} style={{ borderBottom: '1px solid #f0f0f0' }}>
                  <td style={{ padding: '0.5rem' }}>
                    <Link to={`/entities/${e.id}`} style={{ color: '#3a3a6a' }}>
                      {e.name}
                    </Link>
                  </td>
                  <td style={{ padding: '0.5rem' }}>{e.entity_type}</td>
                  <td style={{ padding: '0.5rem' }}>{e.namespace}</td>
                  <td style={{ padding: '0.5rem' }}>{e.tier ?? '—'}</td>
                  <td style={{ padding: '0.5rem' }}>
                    {e.salience_score != null ? e.salience_score.toFixed(2) : '—'}
                  </td>
                </tr>
              ))}
              {data.length === 0 && (
                <tr>
                  <td colSpan={5} className="placeholder">
                    No entities found.
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
