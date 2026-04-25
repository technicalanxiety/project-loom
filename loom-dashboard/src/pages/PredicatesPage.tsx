/**
 * Predicate management page.
 *
 * Two surfaces stacked under one .page-header:
 *   - Candidates: card-per-candidate with inline Map/Promote drawers.
 *     Each card shows the predicate name, occurrences (with an inline-bar
 *     scaling against the run's max), example facts inline, and the
 *     resolution actions. The drawer expands underneath the card so the
 *     forms aren't crammed into a table cell.
 *   - Predicate packs: a kpi-style grid of pack cards. The pack with
 *     the most predicates picks up the .kpi.accent warp-thread border.
 */
import type React from 'react';
import { useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import {
  getPredicateCandidates,
  getPredicatePacks,
  resolvePredicateCandidate,
} from '../api/client';
import { useApi } from '../hooks/useApi';
import type { PredicateCandidateSummary } from '../types';

// ---------------------------------------------------------------------------
// Candidate card with inline resolution drawer
// ---------------------------------------------------------------------------

interface CandidateCardProps {
  candidate: PredicateCandidateSummary;
  maxOccurrences: number;
  onResolved: () => void;
}

const CandidateCard: React.FC<CandidateCardProps> = ({ candidate, maxOccurrences, onResolved }) => {
  const [submitting, setSubmitting] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [drawer, setDrawer] = useState<'map' | 'promote' | null>(null);
  const [targetPack, setTargetPack] = useState('core');
  const [category, setCategory] = useState('structural');
  const [mappedTo, setMappedTo] = useState('');

  const isResolved = candidate.resolved_at != null;
  const occurrencePct = maxOccurrences > 0 ? (candidate.occurrences / maxOccurrences) * 100 : 0;

  const closeDrawer = () => {
    setDrawer(null);
    setActionError(null);
  };

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
    } catch (err) {
      setActionError(err instanceof Error ? err.message : 'Map failed');
    } finally {
      setSubmitting(false);
    }
  };

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
    } catch (err) {
      setActionError(err instanceof Error ? err.message : 'Promote failed');
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className={`conflict-card${isResolved ? ' resolved' : ''}`}>
      <div className="conflict-card-head">
        <h3 className="cell-id" style={{ fontFamily: 'var(--font-mono)', fontSize: 14 }}>
          {candidate.predicate}
        </h3>
        <span style={{ display: 'inline-flex', alignItems: 'center', gap: 8 }}>
          <span
            className="inline-bar tone-ok"
            style={{ width: 80 }}
            aria-hidden="true"
            title={`${candidate.occurrences} occurrences`}
          >
            <span style={{ width: `${occurrencePct}%` }} />
          </span>
          <span className="cell-num" style={{ fontWeight: 600 }}>
            {candidate.occurrences}
          </span>
        </span>
      </div>
      {candidate.example_facts && candidate.example_facts.length > 0 && (
        <div
          className="cell-muted"
          style={{ fontFamily: 'var(--font-mono)', fontSize: 11, marginBottom: 8 }}
        >
          examples:{' '}
          {candidate.example_facts.slice(0, 3).map((f, i) => (
            <span key={f}>
              {i > 0 && ' · '}
              {f}
            </span>
          ))}
        </div>
      )}

      {isResolved ? (
        <span className="pill pill-success">
          <span className="dot" />
          {candidate.mapped_to
            ? `Mapped → ${candidate.mapped_to}`
            : candidate.promoted_to_pack
              ? `Promoted → ${candidate.promoted_to_pack}`
              : 'Resolved'}
        </span>
      ) : (
        <>
          {drawer === null && (
            <div className="conflict-card-actions">
              <button type="button" className="btn btn-secondary" onClick={() => setDrawer('map')}>
                Map
              </button>
              <button
                type="button"
                className="btn btn-primary"
                onClick={() => setDrawer('promote')}
              >
                Promote
              </button>
            </div>
          )}

          {drawer === 'map' && (
            <div style={{ marginTop: 10, display: 'flex', flexDirection: 'column', gap: 8 }}>
              <input
                type="text"
                className="form-control"
                placeholder="Canonical predicate to map to…"
                value={mappedTo}
                onChange={(e) => setMappedTo(e.target.value)}
              />
              <div className="conflict-card-actions">
                <button
                  type="button"
                  className="btn btn-primary"
                  disabled={submitting}
                  onClick={handleMap}
                >
                  Confirm
                </button>
                <button type="button" className="btn btn-ghost" onClick={closeDrawer}>
                  Cancel
                </button>
              </div>
            </div>
          )}

          {drawer === 'promote' && (
            <div style={{ marginTop: 10, display: 'flex', flexDirection: 'column', gap: 8 }}>
              <div style={{ display: 'flex', gap: 8 }}>
                <select
                  className="form-control"
                  value={targetPack}
                  onChange={(e) => setTargetPack(e.target.value)}
                >
                  <option value="core">core</option>
                  <option value="grc">grc</option>
                </select>
                <select
                  className="form-control"
                  value={category}
                  onChange={(e) => setCategory(e.target.value)}
                >
                  <option value="structural">structural</option>
                  <option value="temporal">temporal</option>
                  <option value="decisional">decisional</option>
                  <option value="operational">operational</option>
                  <option value="regulatory">regulatory</option>
                </select>
              </div>
              <div className="conflict-card-actions">
                <button
                  type="button"
                  className="btn btn-primary"
                  disabled={submitting}
                  onClick={handlePromote}
                >
                  Confirm
                </button>
                <button type="button" className="btn btn-ghost" onClick={closeDrawer}>
                  Cancel
                </button>
              </div>
            </div>
          )}

          {actionError && (
            <div className="callout callout-error" style={{ marginTop: 10 }}>
              <div className="callout-body">{actionError}</div>
            </div>
          )}
        </>
      )}
    </div>
  );
};

// ---------------------------------------------------------------------------
// Main page component
// ---------------------------------------------------------------------------

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
  const resolvedCount = candidates ? candidates.length - pendingCount : 0;

  const maxOccurrences = useMemo(
    () => candidates?.reduce((max, c) => Math.max(max, c.occurrences), 0) ?? 0,
    [candidates],
  );

  const accentPack = useMemo(() => {
    if (!packs || packs.length === 0) return null;
    return packs.reduce((a, b) => (a.predicate_count >= b.predicate_count ? a : b)).pack;
  }, [packs]);

  return (
    <>
      <div className="page-header">
        <div className="page-header-titles">
          <div className="page-eyebrow">Operations / Predicates</div>
          <h2>Predicates</h2>
          <p>
            Manage custom predicate candidates and browse predicate packs.
            {candidates && ` ${pendingCount} candidates pending review.`}
          </p>
        </div>
      </div>

      {/* Candidates */}
      <div className="section-head">
        <h3>
          Candidates <span className="count-pill">{pendingCount} pending</span>
        </h3>
        {resolvedCount > 0 && (
          <span
            className="count-pill"
            style={{ background: 'var(--moss-100)', color: 'var(--moss-800)' }}
          >
            {resolvedCount} resolved
          </span>
        )}
      </div>

      {loadingCandidates && <div className="loading">Loading candidates…</div>}
      {errorCandidates && <div className="error">{errorCandidates}</div>}

      {candidates &&
        (candidates.length === 0 ? (
          <div className="empty-state">
            <h3>No predicate candidates</h3>
            <p>
              Custom predicates emerge as the extraction pipeline encounters relations that aren't
              in the active packs. Once they appear, you can map them to canonical predicates or
              promote them into a pack.
            </p>
          </div>
        ) : (
          <div>
            {candidates.map((c) => (
              <CandidateCard
                key={c.id}
                candidate={c}
                maxOccurrences={maxOccurrences}
                onResolved={refetchCandidates}
              />
            ))}
          </div>
        ))}

      {/* Packs */}
      <div className="section-head" style={{ marginTop: 'var(--space-8)' }}>
        <h3>
          Predicate packs <span className="count-pill">{packs?.length ?? 0}</span>
        </h3>
      </div>

      {loadingPacks && <div className="loading">Loading packs…</div>}
      {errorPacks && <div className="error">{errorPacks}</div>}

      {packs &&
        (packs.length === 0 ? (
          <div className="empty-state">
            <h3>No packs available</h3>
            <p>
              Predicate packs ship with the engine. If this list is empty, check that the
              <code>core</code> and <code>grc</code> packs are seeded in the database.
            </p>
          </div>
        ) : (
          <div className="kpi-grid">
            {packs.map((p) => (
              <Link
                key={p.pack}
                to={`/predicates/packs/${encodeURIComponent(p.pack)}`}
                style={{ textDecoration: 'none', color: 'inherit' }}
              >
                <div className={`kpi${p.pack === accentPack ? ' accent' : ''}`}>
                  <div className="kpi-eyebrow">Pack</div>
                  <div className="kpi-value model" style={{ marginBottom: 8 }}>
                    {p.pack}
                  </div>
                  <div
                    style={{
                      fontFamily: 'var(--font-mono)',
                      fontSize: 12,
                      color: 'var(--fg-3)',
                      marginBottom: 8,
                      minHeight: '2.4em',
                      lineHeight: 1.4,
                    }}
                  >
                    {p.description ?? 'No description'}
                  </div>
                  <div
                    className="cell-num"
                    style={{ fontFamily: 'var(--font-mono)', fontWeight: 600, fontSize: 13 }}
                  >
                    {p.predicate_count} predicates
                  </div>
                </div>
              </Link>
            ))}
          </div>
        ))}
    </>
  );
};
