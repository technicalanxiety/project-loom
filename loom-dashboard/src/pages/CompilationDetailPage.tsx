/**
 * Compilation trace detail page.
 *
 * Shows the full breakdown of a single loom_think compilation
 * including latency stages, candidates, and scoring.
 */
import type React from 'react';
import { Link, useParams } from 'react-router-dom';
import { getCompilationDetail } from '../api/client';
import { useApi } from '../hooks/useApi';

/** Detail view for a single compilation trace. */
export const CompilationDetailPage: React.FC = () => {
  const { id } = useParams<{ id: string }>();
  // biome-ignore lint/style/noNonNullAssertion: id guaranteed by route param
  const { data, loading, error } = useApi(() => getCompilationDetail(id!), [id]);

  return (
    <div>
      <div className="page-header">
        <h2>Compilation Detail</h2>
        <p>
          <Link to="/compilations" style={{ color: '#3a3a6a' }}>
            ← Back to compilations
          </Link>
        </p>
      </div>

      {loading && <p className="loading">Loading…</p>}
      {error && <p className="error">{error}</p>}

      {data && (
        <div className="card">
          <dl
            style={{
              display: 'grid',
              gridTemplateColumns: 'max-content 1fr',
              gap: '0.5rem 1rem',
              fontSize: '0.85rem',
            }}
          >
            <dt style={{ fontWeight: 600 }}>ID</dt>
            <dd>{data.id}</dd>
            <dt style={{ fontWeight: 600 }}>Namespace</dt>
            <dd>{data.namespace}</dd>
            <dt style={{ fontWeight: 600 }}>Query</dt>
            <dd>{data.query_text ?? '—'}</dd>
            <dt style={{ fontWeight: 600 }}>Task Class</dt>
            <dd>
              {data.task_class}
              {data.secondary_class ? ` / ${data.secondary_class}` : ''}
            </dd>
            <dt style={{ fontWeight: 600 }}>Confidence</dt>
            <dd>
              {data.primary_confidence != null
                ? `${(data.primary_confidence * 100).toFixed(1)}%`
                : '—'}
              {data.secondary_confidence != null
                ? ` / ${(data.secondary_confidence * 100).toFixed(1)}%`
                : ''}
            </dd>
            <dt style={{ fontWeight: 600 }}>Candidates</dt>
            <dd>
              {data.candidates_found ?? '—'} found, {data.candidates_selected ?? '—'} selected
            </dd>
            <dt style={{ fontWeight: 600 }}>Tokens</dt>
            <dd>{data.compiled_tokens ?? '—'}</dd>
            <dt style={{ fontWeight: 600 }}>Format</dt>
            <dd>{data.output_format ?? '—'}</dd>
            <dt style={{ fontWeight: 600 }}>User Rating</dt>
            <dd>{data.user_rating ?? '—'}</dd>
          </dl>

          <h3 style={{ marginTop: '1.5rem', marginBottom: '0.5rem', fontSize: '0.95rem' }}>
            Profiles Executed
          </h3>
          <p style={{ fontSize: '0.85rem', marginBottom: '1rem' }}>
            {data.profiles_executed?.join(', ') ?? '—'}
          </p>

          <h3 style={{ marginTop: '1.5rem', marginBottom: '0.5rem', fontSize: '0.95rem' }}>
            Latency Breakdown
          </h3>
          <div
            style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(auto-fit, minmax(140px, 1fr))',
              gap: '0.75rem',
            }}
          >
            {[
              { label: 'Classify', value: data.latency_classify_ms },
              { label: 'Retrieve', value: data.latency_retrieve_ms },
              { label: 'Rank', value: data.latency_rank_ms },
              { label: 'Compile', value: data.latency_compile_ms },
              { label: 'Total', value: data.latency_total_ms },
            ].map((stage) => (
              <div
                key={stage.label}
                style={{
                  background: '#f5f6fa',
                  borderRadius: '6px',
                  padding: '0.75rem',
                  textAlign: 'center',
                }}
              >
                <p style={{ fontSize: '0.7rem', color: '#888', textTransform: 'uppercase' }}>
                  {stage.label}
                </p>
                <p style={{ fontSize: '1.1rem', fontWeight: 700 }}>
                  {stage.value != null ? `${stage.value}ms` : '—'}
                </p>
              </div>
            ))}
          </div>

          {data.selected_items != null && (
            <div>
              <h3 style={{ marginTop: '1.5rem', marginBottom: '0.5rem', fontSize: '0.95rem' }}>
                Selected Candidates
              </h3>
              <pre
                style={{
                  background: '#f5f6fa',
                  borderRadius: '6px',
                  padding: '0.75rem',
                  fontSize: '0.8rem',
                  overflow: 'auto',
                  maxHeight: '300px',
                }}
              >
                {JSON.stringify(data.selected_items, null, 2)}
              </pre>
            </div>
          )}

          {data.rejected_items != null && (
            <div>
              <h3 style={{ marginTop: '1.5rem', marginBottom: '0.5rem', fontSize: '0.95rem' }}>
                Rejected Candidates
              </h3>
              <pre
                style={{
                  background: '#fdf2f2',
                  borderRadius: '6px',
                  padding: '0.75rem',
                  fontSize: '0.8rem',
                  overflow: 'auto',
                  maxHeight: '300px',
                }}
              >
                {JSON.stringify(data.rejected_items, null, 2)}
              </pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
};
