/**
 * Pipeline health overview page.
 *
 * Displays episode counts, entity breakdowns, fact stats,
 * queue depth, and active model configuration.
 */
import type React from 'react';
import { getPipelineHealth } from '../api/client';
import { useApi } from '../hooks/useApi';
import type { CountByKey } from '../types';

/** Render a key-count breakdown as a simple list. */
const CountList: React.FC<{ title: string; items: CountByKey[] }> = ({ title, items }) => (
  <div className="card">
    <h3 style={{ fontSize: '0.95rem', marginBottom: '0.5rem' }}>{title}</h3>
    {items.length === 0 ? (
      <p style={{ color: '#999', fontSize: '0.85rem' }}>No data</p>
    ) : (
      <ul style={{ listStyle: 'none', padding: 0 }}>
        {items.map((item) => (
          <li
            key={item.key}
            style={{
              display: 'flex',
              justifyContent: 'space-between',
              padding: '0.25rem 0',
              fontSize: '0.85rem',
              borderBottom: '1px solid #f0f0f0',
            }}
          >
            <span>{item.key}</span>
            <span style={{ fontWeight: 600 }}>{item.count}</span>
          </li>
        ))}
      </ul>
    )}
  </div>
);

/** Pipeline health dashboard — the default landing page. */
export const HomePage: React.FC = () => {
  const { data, loading, error } = useApi(() => getPipelineHealth(), []);

  return (
    <div>
      <div className="page-header">
        <h2>Pipeline Health</h2>
        <p>Overview of episode ingestion, entity extraction, and fact management.</p>
      </div>

      {loading && <p className="loading">Loading health data…</p>}
      {error && <p className="error">{error}</p>}

      {data && (
        <>
          <div
            style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(auto-fit, minmax(200px, 1fr))',
              gap: '1rem',
              marginBottom: '1.5rem',
            }}
          >
            <div className="card">
              <p style={{ fontSize: '0.75rem', color: '#888', textTransform: 'uppercase' }}>
                Current Facts
              </p>
              <p style={{ fontSize: '1.5rem', fontWeight: 700 }}>{data.facts_current}</p>
            </div>
            <div className="card">
              <p style={{ fontSize: '0.75rem', color: '#888', textTransform: 'uppercase' }}>
                Superseded Facts
              </p>
              <p style={{ fontSize: '1.5rem', fontWeight: 700 }}>{data.facts_superseded}</p>
            </div>
            <div className="card">
              <p style={{ fontSize: '0.75rem', color: '#888', textTransform: 'uppercase' }}>
                Queue Depth
              </p>
              <p style={{ fontSize: '1.5rem', fontWeight: 700 }}>{data.queue_depth}</p>
            </div>
            <div className="card">
              <p style={{ fontSize: '0.75rem', color: '#888', textTransform: 'uppercase' }}>
                Extraction Model
              </p>
              <p style={{ fontSize: '0.95rem', fontWeight: 600 }}>{data.extraction_model ?? '—'}</p>
            </div>
            <div className="card">
              <p style={{ fontSize: '0.75rem', color: '#888', textTransform: 'uppercase' }}>
                Classification Model
              </p>
              <p style={{ fontSize: '0.95rem', fontWeight: 600 }}>
                {data.classification_model ?? '—'}
              </p>
            </div>
          </div>

          <div
            style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(auto-fit, minmax(280px, 1fr))',
              gap: '1rem',
            }}
          >
            <CountList title="Episodes by Source" items={data.episodes_by_source} />
            <CountList title="Episodes by Namespace" items={data.episodes_by_namespace} />
            <CountList title="Entities by Type" items={data.entities_by_type} />
          </div>
        </>
      )}
    </div>
  );
};
