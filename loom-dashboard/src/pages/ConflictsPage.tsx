/**
 * Entity conflict review queue page.
 *
 * Lists unresolved entity-resolution conflicts and provides
 * actions to merge, keep separate, or split via POST to the
 * dashboard API.
 */
import type React from 'react';
import { useState } from 'react';
import { getConflicts, resolveConflict } from '../api/client';
import { useApi } from '../hooks/useApi';
import type { ConflictSummary } from '../types';

// ---------------------------------------------------------------------------
// Conflict row with inline resolution actions
// ---------------------------------------------------------------------------

/** Props for a single conflict row. */
interface ConflictRowProps {
  conflict: ConflictSummary;
  onResolved: () => void;
}

/** A single conflict row with merge/keep-separate/split actions. */
const ConflictRow: React.FC<ConflictRowProps> = ({ conflict, onResolved }) => {
  const [submitting, setSubmitting] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  /** Submit a resolution decision. */
  const handleResolve = async (resolution: string) => {
    setSubmitting(true);
    setActionError(null);
    try {
      await resolveConflict(conflict.id, { resolution });
      onResolved();
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : 'Resolution failed';
      setActionError(message);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <tr style={{ borderBottom: '1px solid #f0f0f0' }}>
      <td style={{ padding: '0.5rem', fontWeight: 600 }}>{conflict.entity_name}</td>
      <td style={{ padding: '0.5rem' }}>{conflict.entity_type}</td>
      <td style={{ padding: '0.5rem' }}>{conflict.namespace}</td>
      <td style={{ padding: '0.5rem' }}>
        {conflict.candidates ? (
          <pre
            style={{
              fontSize: '0.75rem',
              background: '#f5f6fa',
              padding: '0.25rem 0.5rem',
              borderRadius: '4px',
              maxWidth: '200px',
              overflow: 'auto',
              maxHeight: '80px',
              margin: 0,
            }}
          >
            {JSON.stringify(conflict.candidates, null, 1)}
          </pre>
        ) : (
          '—'
        )}
      </td>
      <td style={{ padding: '0.5rem' }}>
        {conflict.resolved ? (
          <span style={{ color: '#27ae60', fontWeight: 600 }}>{conflict.resolution}</span>
        ) : (
          <div style={{ display: 'flex', gap: '0.35rem', flexWrap: 'wrap' }}>
            <button
              type="button"
              disabled={submitting}
              onClick={() => handleResolve('kept_separate')}
              style={{
                padding: '0.25rem 0.5rem',
                fontSize: '0.75rem',
                borderRadius: '4px',
                border: '1px solid #3498db',
                background: '#fff',
                color: '#3498db',
                cursor: submitting ? 'not-allowed' : 'pointer',
              }}
            >
              Keep Separate
            </button>
            <button
              type="button"
              disabled={submitting}
              onClick={() => handleResolve('merged')}
              style={{
                padding: '0.25rem 0.5rem',
                fontSize: '0.75rem',
                borderRadius: '4px',
                border: '1px solid #27ae60',
                background: '#fff',
                color: '#27ae60',
                cursor: submitting ? 'not-allowed' : 'pointer',
              }}
            >
              Merge
            </button>
            <button
              type="button"
              disabled={submitting}
              onClick={() => handleResolve('split')}
              style={{
                padding: '0.25rem 0.5rem',
                fontSize: '0.75rem',
                borderRadius: '4px',
                border: '1px solid #e67e22',
                background: '#fff',
                color: '#e67e22',
                cursor: submitting ? 'not-allowed' : 'pointer',
              }}
            >
              Split
            </button>
          </div>
        )}
        {actionError && (
          <p style={{ color: '#c0392b', fontSize: '0.75rem', marginTop: '0.25rem' }}>
            {actionError}
          </p>
        )}
      </td>
      <td style={{ padding: '0.5rem' }}>{new Date(conflict.created_at).toLocaleDateString()}</td>
    </tr>
  );
};

// ---------------------------------------------------------------------------
// Main page component
// ---------------------------------------------------------------------------

/** Conflict review queue with resolution actions. */
export const ConflictsPage: React.FC = () => {
  const { data, loading, error, refetch } = useApi(() => getConflicts(), []);

  const unresolvedCount = data?.filter((c) => !c.resolved).length ?? 0;

  return (
    <div>
      <div className="page-header">
        <h2>Entity Conflicts</h2>
        <p>
          Review and resolve ambiguous entity resolutions.
          {data && ` ${unresolvedCount} unresolved of ${data.length} total.`}
        </p>
      </div>

      {loading && <p className="loading">Loading conflicts…</p>}
      {error && <p className="error">{error}</p>}

      {data && (
        <div className="card" style={{ overflowX: 'auto' }}>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '0.85rem' }}>
            <thead>
              <tr style={{ borderBottom: '2px solid #eee', textAlign: 'left' }}>
                <th style={{ padding: '0.5rem' }}>Entity</th>
                <th style={{ padding: '0.5rem' }}>Type</th>
                <th style={{ padding: '0.5rem' }}>Namespace</th>
                <th style={{ padding: '0.5rem' }}>Candidates</th>
                <th style={{ padding: '0.5rem' }}>Action</th>
                <th style={{ padding: '0.5rem' }}>Created</th>
              </tr>
            </thead>
            <tbody>
              {data.map((c) => (
                <ConflictRow key={c.id} conflict={c} onResolved={refetch} />
              ))}
              {data.length === 0 && (
                <tr>
                  <td colSpan={6} className="placeholder">
                    No conflicts found.
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
