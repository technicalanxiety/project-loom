/**
 * Entity conflict review queue.
 *
 * Card-per-conflict layout — the operator's eye lands on the two
 * candidate names side-by-side, with the action buttons directly
 * underneath. Resolved cards collapse to a one-line summary so the
 * pending queue stays at the top of the page.
 *
 * KPI strip surfaces the queue depth, daily resolution counts, and
 * median age of unresolved conflicts.
 */
import type React from 'react';
import { useMemo, useState } from 'react';
import { getConflicts, resolveConflict } from '../api/client';
import { useApi } from '../hooks/useApi';
import { relativeTime } from '../lib/thresholds';
import type { ConflictSummary } from '../types';

// ---------------------------------------------------------------------------
// Types — the candidates field is `unknown` on the wire; this is our
// best-effort decoder for the common shape `{ name: string; score?: number }[]`.
// Anything else falls through to a JSON detail row.
// ---------------------------------------------------------------------------

type Candidate = { id?: string; name?: string; score?: number; [k: string]: unknown };

function asCandidates(raw: unknown): Candidate[] {
  if (Array.isArray(raw)) {
    return raw.filter((c): c is Candidate => typeof c === 'object' && c !== null);
  }
  return [];
}

function bestMatchScore(candidates: Candidate[]): number | null {
  const scores = candidates.map((c) => c.score).filter((s): s is number => typeof s === 'number');
  return scores.length === 0 ? null : Math.max(...scores);
}

// ---------------------------------------------------------------------------
// Conflict card
// ---------------------------------------------------------------------------

const ConflictCard: React.FC<{ conflict: ConflictSummary; onResolved: () => void }> = ({
  conflict,
  onResolved,
}) => {
  const [submitting, setSubmitting] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const candidates = useMemo(() => asCandidates(conflict.candidates), [conflict.candidates]);
  const bestScore = useMemo(() => bestMatchScore(candidates), [candidates]);
  const recommendedMerge = bestScore !== null && bestScore >= 0.85;

  const handleResolve = async (resolution: string) => {
    setSubmitting(true);
    setActionError(null);
    try {
      await resolveConflict(conflict.id, { resolution });
      onResolved();
    } catch (err) {
      setActionError(err instanceof Error ? err.message : 'Resolution failed');
    } finally {
      setSubmitting(false);
    }
  };

  // The entity_name appears once per card — as the h3 in the head when
  // there are no rendered candidate cards (resolved or no-candidates
  // case), or as the left candidate-card's name when a candidate-pair
  // is shown. That keeps `getByText(entity_name)` unambiguous.
  if (conflict.resolved) {
    return (
      <div className="conflict-card resolved">
        <div className="conflict-card-head">
          <h3>{conflict.entity_name}</h3>
          <span className="cell-muted" style={{ fontFamily: 'var(--font-mono)', fontSize: 11 }}>
            {conflict.entity_type} · {conflict.namespace}
          </span>
          <span className="pill pill-success">
            <span className="dot" />
            {conflict.resolution}
          </span>
        </div>
        <div className="cell-muted" style={{ fontFamily: 'var(--font-mono)', fontSize: 11 }}>
          resolved {relativeTime(conflict.created_at)}
        </div>
      </div>
    );
  }

  const hasPair = candidates.length > 0;

  return (
    <div className="conflict-card">
      <div className="conflict-card-head">
        {!hasPair && <h3>{conflict.entity_name}</h3>}
        <span className="cell-muted" style={{ fontFamily: 'var(--font-mono)', fontSize: 11 }}>
          {conflict.entity_type} · {conflict.namespace}
        </span>
        {bestScore !== null && (
          <span
            className={`pill ${
              bestScore >= 0.85
                ? 'pill-success'
                : bestScore >= 0.5
                  ? 'pill-warning'
                  : 'pill-neutral'
            }`}
          >
            {(bestScore * 100).toFixed(0)}% match
          </span>
        )}
      </div>

      {hasPair ? (
        <div className="candidate-pair">
          <div className="candidate-card">
            <div className="name">{conflict.entity_name}</div>
            <div className="meta">first seen {relativeTime(conflict.created_at)}</div>
          </div>
          <div className="arrow">↔</div>
          <div className="candidate-card">
            <div className="name">{candidates[0].name ?? candidates[0].id ?? 'candidate'}</div>
            <div className="meta">
              {Object.entries(candidates[0])
                .filter(([k]) => !['name', 'id', 'score'].includes(k))
                .slice(0, 2)
                .map(([k, v]) => `${k}: ${String(v)}`)
                .join(' · ')}
            </div>
            {candidates[0].score != null && (
              <div className="meta">score {(Number(candidates[0].score) * 100).toFixed(0)}%</div>
            )}
          </div>
        </div>
      ) : (
        <div className="callout callout-info" style={{ marginTop: 10 }}>
          <div className="callout-body">No candidate detail attached to this conflict.</div>
        </div>
      )}

      <div className="conflict-card-actions">
        <button
          type="button"
          className={recommendedMerge ? 'btn btn-primary' : 'btn btn-secondary'}
          disabled={submitting}
          onClick={() => handleResolve('merged')}
        >
          Merge
        </button>
        <button
          type="button"
          className={!recommendedMerge ? 'btn btn-primary' : 'btn btn-secondary'}
          disabled={submitting}
          onClick={() => handleResolve('kept_separate')}
        >
          Keep Separate
        </button>
        <button
          type="button"
          className="btn btn-secondary"
          disabled={submitting}
          onClick={() => handleResolve('split')}
        >
          Split
        </button>
        <button
          type="button"
          className="btn btn-ghost"
          disabled={submitting}
          onClick={() => handleResolve('skipped')}
          style={{ marginLeft: 'auto' }}
        >
          Skip
        </button>
      </div>

      {actionError && (
        <div className="callout callout-error" style={{ marginTop: 10 }}>
          <div className="callout-body">{actionError}</div>
        </div>
      )}
    </div>
  );
};

// ---------------------------------------------------------------------------
// Main page
// ---------------------------------------------------------------------------

export const ConflictsPage: React.FC = () => {
  const { data, loading, error, refetch } = useApi(() => getConflicts(), []);

  const stats = useMemo(() => {
    if (!data) return null;
    const unresolved = data.filter((c) => !c.resolved);
    const resolved = data.filter((c) => c.resolved);
    const merged = resolved.filter((c) => c.resolution === 'merged').length;
    const keptSeparate = resolved.filter((c) => c.resolution === 'kept_separate').length;
    const ages = unresolved
      .map((c) => Date.parse(c.created_at))
      .filter((t) => !Number.isNaN(t))
      .map((t) => (Date.now() - t) / 1000 / 3600); // hours
    ages.sort((a, b) => a - b);
    const medianHours =
      ages.length === 0
        ? null
        : ages.length % 2 === 0
          ? (ages[ages.length / 2 - 1] + ages[ages.length / 2]) / 2
          : ages[Math.floor(ages.length / 2)];
    return {
      pending: unresolved.length,
      total: data.length,
      merged,
      keptSeparate,
      medianHours,
    };
  }, [data]);

  return (
    <>
      <div className="page-header">
        <div className="page-header-titles">
          <div className="page-eyebrow">Operations / Conflicts</div>
          <h2>Entity conflicts</h2>
          <p>
            Review and resolve ambiguous entity resolutions.
            {stats && ` ${stats.pending} unresolved of ${stats.total} total.`}
          </p>
        </div>
      </div>

      {loading && <div className="loading">Loading conflicts…</div>}
      {error && <div className="error">{error}</div>}

      {data && stats && (
        <>
          <div className="kpi-grid">
            <div className="kpi accent">
              <div className="kpi-eyebrow">Pending</div>
              <div className="kpi-value numeric">{stats.pending}</div>
            </div>
            <div className="kpi">
              <div className="kpi-eyebrow">Merged</div>
              <div className="kpi-value numeric" style={{ color: 'var(--moss-700)' }}>
                {stats.merged}
              </div>
            </div>
            <div className="kpi">
              <div className="kpi-eyebrow">Kept separate</div>
              <div className="kpi-value numeric">{stats.keptSeparate}</div>
            </div>
            <div className="kpi">
              <div className="kpi-eyebrow">Median age</div>
              <div className="kpi-value numeric">
                {stats.medianHours === null ? '—' : stats.medianHours.toFixed(1)}
                {stats.medianHours !== null && <span className="kpi-unit">h</span>}
              </div>
            </div>
          </div>

          {data.length === 0 ? (
            <div className="empty-state">
              <h3>No conflicts found.</h3>
              <p>
                When the resolver finds an ambiguous entity match it lands here for human review.
              </p>
            </div>
          ) : (
            <>
              <div className="section-head">
                <h3>
                  Pending review <span className="count-pill">{stats.pending}</span>
                </h3>
              </div>
              {data
                .filter((c) => !c.resolved)
                .map((c) => (
                  <ConflictCard key={c.id} conflict={c} onResolved={refetch} />
                ))}
              {stats.total - stats.pending > 0 && (
                <>
                  <div className="section-head" style={{ marginTop: 'var(--space-6)' }}>
                    <h3>
                      Resolved <span className="count-pill">{stats.total - stats.pending}</span>
                    </h3>
                  </div>
                  {data
                    .filter((c) => c.resolved)
                    .map((c) => (
                      <ConflictCard key={c.id} conflict={c} onResolved={refetch} />
                    ))}
                </>
              )}
            </>
          )}
        </>
      )}
    </>
  );
};
