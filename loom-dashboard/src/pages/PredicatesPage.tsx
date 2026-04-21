/**
 * Predicate management page.
 *
 * Shows predicate candidates awaiting review with actions to map
 * or promote, and a pack browser with links to pack detail views.
 */
import type React from 'react';
import { useState } from 'react';
import { Link } from 'react-router-dom';
import {
  getPredicateCandidates,
  getPredicatePacks,
  resolvePredicateCandidate,
} from '../api/client';
import { useApi } from '../hooks/useApi';
import type { PredicateCandidateSummary } from '../types';

// ---------------------------------------------------------------------------
// Candidate row with inline resolution actions
// ---------------------------------------------------------------------------

/** Props for a single candidate row. */
interface CandidateRowProps {
  candidate: PredicateCandidateSummary;
  onResolved: () => void;
}

/** A single predicate candidate row with map/promote actions. */
const CandidateRow: React.FC<CandidateRowProps> = ({ candidate, onResolved }) => {
  const [submitting, setSubmitting] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [showPromote, setShowPromote] = useState(false);
  const [showMap, setShowMap] = useState(false);
  const [targetPack, setTargetPack] = useState('core');
  const [category, setCategory] = useState('structural');
  const [mappedTo, setMappedTo] = useState('');

  /** Submit a map resolution. */
  const handleMap = async () => {
    if (!mappedTo.trim()) {
      setActionError('Enter a canonical predicate to map to.');
      return;
    }
    setSubmitting(true);
    setActionError(null);
    try {
      await resolvePredicateCandidate(candidate.id, {
        action: 'map',
        mapped_to: mappedTo.trim(),
      });
      onResolved();
    } catch (err: unknown) {
      setActionError(err instanceof Error ? err.message : 'Map failed');
    } finally {
      setSubmitting(false);
    }
  };

  /** Submit a promote resolution. */
  const handlePromote = async () => {
    setSubmitting(true);
    setActionError(null);
    try {
      await resolvePredicateCandidate(candidate.id, {
        action: 'promote',
        target_pack: targetPack,
        category,
      });
      onResolved();
    } catch (err: unknown) {
      setActionError(err instanceof Error ? err.message : 'Promote failed');
    } finally {
      setSubmitting(false);
    }
  };

  const isResolved = candidate.resolved_at != null;

  return (
    <tr style={{ borderBottom: '1px solid #f0f0f0', verticalAlign: 'top' }}>
      <td style={{ padding: '0.5rem', fontFamily: 'monospace', fontSize: '0.8rem' }}>
        {candidate.predicate}
      </td>
      <td style={{ padding: '0.5rem' }}>{candidate.occurrences}</td>
      <td style={{ padding: '0.5rem', fontSize: '0.8rem' }}>
        {candidate.example_facts?.length ?? 0} facts
      </td>
      <td style={{ padding: '0.5rem' }}>
        {isResolved ? (
          <span style={{ color: '#27ae60', fontWeight: 600 }}>
            {candidate.mapped_to
              ? `Mapped → ${candidate.mapped_to}`
              : candidate.promoted_to_pack
                ? `Promoted → ${candidate.promoted_to_pack}`
                : 'Resolved'}
          </span>
        ) : (
          <div>
            {!(showMap || showPromote) && (
              <div style={{ display: 'flex', gap: '0.35rem' }}>
                <button
                  type="button"
                  onClick={() => setShowMap(true)}
                  style={{
                    padding: '0.25rem 0.5rem',
                    fontSize: '0.75rem',
                    borderRadius: '4px',
                    border: '1px solid #3498db',
                    background: '#fff',
                    color: '#3498db',
                    cursor: 'pointer',
                  }}
                >
                  Map
                </button>
                <button
                  type="button"
                  onClick={() => setShowPromote(true)}
                  style={{
                    padding: '0.25rem 0.5rem',
                    fontSize: '0.75rem',
                    borderRadius: '4px',
                    border: '1px solid #27ae60',
                    background: '#fff',
                    color: '#27ae60',
                    cursor: 'pointer',
                  }}
                >
                  Promote
                </button>
              </div>
            )}

            {showMap && (
              <div style={{ display: 'flex', flexDirection: 'column', gap: '0.35rem' }}>
                <input
                  type="text"
                  placeholder="Canonical predicate…"
                  value={mappedTo}
                  onChange={(e) => setMappedTo(e.target.value)}
                  style={{
                    padding: '0.25rem 0.4rem',
                    fontSize: '0.8rem',
                    borderRadius: '4px',
                    border: '1px solid #ccc',
                  }}
                />
                <div style={{ display: 'flex', gap: '0.25rem' }}>
                  <button
                    type="button"
                    disabled={submitting}
                    onClick={handleMap}
                    style={{
                      padding: '0.2rem 0.4rem',
                      fontSize: '0.75rem',
                      borderRadius: '4px',
                      border: 'none',
                      background: '#3498db',
                      color: '#fff',
                      cursor: submitting ? 'not-allowed' : 'pointer',
                    }}
                  >
                    Confirm
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      setShowMap(false);
                      setActionError(null);
                    }}
                    style={{
                      padding: '0.2rem 0.4rem',
                      fontSize: '0.75rem',
                      borderRadius: '4px',
                      border: '1px solid #ccc',
                      background: '#fff',
                      color: '#666',
                      cursor: 'pointer',
                    }}
                  >
                    Cancel
                  </button>
                </div>
              </div>
            )}

            {showPromote && (
              <div style={{ display: 'flex', flexDirection: 'column', gap: '0.35rem' }}>
                <select
                  value={targetPack}
                  onChange={(e) => setTargetPack(e.target.value)}
                  style={{
                    padding: '0.25rem 0.4rem',
                    fontSize: '0.8rem',
                    borderRadius: '4px',
                    border: '1px solid #ccc',
                  }}
                >
                  <option value="core">core</option>
                  <option value="grc">grc</option>
                </select>
                <select
                  value={category}
                  onChange={(e) => setCategory(e.target.value)}
                  style={{
                    padding: '0.25rem 0.4rem',
                    fontSize: '0.8rem',
                    borderRadius: '4px',
                    border: '1px solid #ccc',
                  }}
                >
                  <option value="structural">structural</option>
                  <option value="temporal">temporal</option>
                  <option value="decisional">decisional</option>
                  <option value="operational">operational</option>
                  <option value="regulatory">regulatory</option>
                </select>
                <div style={{ display: 'flex', gap: '0.25rem' }}>
                  <button
                    type="button"
                    disabled={submitting}
                    onClick={handlePromote}
                    style={{
                      padding: '0.2rem 0.4rem',
                      fontSize: '0.75rem',
                      borderRadius: '4px',
                      border: 'none',
                      background: '#27ae60',
                      color: '#fff',
                      cursor: submitting ? 'not-allowed' : 'pointer',
                    }}
                  >
                    Confirm
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      setShowPromote(false);
                      setActionError(null);
                    }}
                    style={{
                      padding: '0.2rem 0.4rem',
                      fontSize: '0.75rem',
                      borderRadius: '4px',
                      border: '1px solid #ccc',
                      background: '#fff',
                      color: '#666',
                      cursor: 'pointer',
                    }}
                  >
                    Cancel
                  </button>
                </div>
              </div>
            )}

            {actionError && (
              <p style={{ color: '#c0392b', fontSize: '0.75rem', marginTop: '0.25rem' }}>
                {actionError}
              </p>
            )}
          </div>
        )}
      </td>
    </tr>
  );
};

// ---------------------------------------------------------------------------
// Main page component
// ---------------------------------------------------------------------------

/** Predicate candidates and pack browser. */
export const PredicatesPage: React.FC = () => {
  const {
    data: candidates,
    loading: loadingCandidates,
    error: errorCandidates,
    refetch: refetchCandidates,
  } = useApi(() => getPredicateCandidates(), []);
  const {
    data: packs,
    loading: loadingPacks,
    error: errorPacks,
  } = useApi(() => getPredicatePacks(), []);

  const pendingCount = candidates?.filter((c) => !c.resolved_at).length ?? 0;

  return (
    <div>
      <div className="page-header">
        <h2>Predicates</h2>
        <p>
          Manage custom predicate candidates and browse predicate packs.
          {candidates && ` ${pendingCount} candidates pending review.`}
        </p>
      </div>

      {/* Candidates section */}
      <h3 style={{ fontSize: '1rem', marginBottom: '0.75rem' }}>Candidates</h3>
      {loadingCandidates && <p className="loading">Loading candidates…</p>}
      {errorCandidates && <p className="error">{errorCandidates}</p>}

      {candidates && (
        <div className="card" style={{ overflowX: 'auto', marginBottom: '2rem' }}>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '0.85rem' }}>
            <thead>
              <tr style={{ borderBottom: '2px solid #eee', textAlign: 'left' }}>
                <th style={{ padding: '0.5rem' }}>Predicate</th>
                <th style={{ padding: '0.5rem' }}>Occurrences</th>
                <th style={{ padding: '0.5rem' }}>Examples</th>
                <th style={{ padding: '0.5rem' }}>Action</th>
              </tr>
            </thead>
            <tbody>
              {candidates.map((c) => (
                <CandidateRow key={c.id} candidate={c} onResolved={refetchCandidates} />
              ))}
              {candidates.length === 0 && (
                <tr>
                  <td colSpan={4} className="placeholder">
                    No predicate candidates.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      )}

      {/* Pack browser section */}
      <h3 style={{ fontSize: '1rem', marginBottom: '0.75rem' }}>Predicate Packs</h3>
      {loadingPacks && <p className="loading">Loading packs…</p>}
      {errorPacks && <p className="error">{errorPacks}</p>}

      {packs && (
        <div
          style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fit, minmax(240px, 1fr))',
            gap: '1rem',
          }}
        >
          {packs.map((p) => (
            <Link
              key={p.pack}
              to={`/predicates/packs/${encodeURIComponent(p.pack)}`}
              style={{ textDecoration: 'none', color: 'inherit' }}
            >
              <div className="card" style={{ cursor: 'pointer' }}>
                <h4 style={{ fontSize: '0.95rem', marginBottom: '0.25rem' }}>{p.pack}</h4>
                <p style={{ fontSize: '0.8rem', color: '#666' }}>
                  {p.description ?? 'No description'}
                </p>
                <p style={{ fontSize: '0.85rem', fontWeight: 600, marginTop: '0.5rem' }}>
                  {p.predicate_count} predicates
                </p>
              </div>
            </Link>
          ))}
          {packs.length === 0 && <p className="placeholder">No packs found.</p>}
        </div>
      )}
    </div>
  );
};
