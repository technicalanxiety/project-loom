/**
 * Benchmark comparison page.
 *
 * A/B/C condition comparison: No memory · Episode-only · Full Loom.
 * The visual punch is in the winner treatment — Condition C picks up
 * a moss accent + warp-thread top border + "winner" pill, with the
 * +precision / −tokens deltas in palette-correct moss/madder pills.
 *
 * Run list uses the shared .tbl + .pill-{success,error,warning} status
 * pills. Per-task results render as a flat .tbl with three numeric
 * columns per metric (A/B/C) — the easy read is "find C's column and
 * scan vertically".
 */
import type React from 'react';
import { useCallback, useState } from 'react';
import { getBenchmarkDetail, getBenchmarkRuns, runBenchmark } from '../api/client';
import { useApi } from '../hooks/useApi';
import { relativeTime } from '../lib/thresholds';
import type {
  BenchmarkComparison,
  BenchmarkRun,
  BenchmarkTaskResult,
  ConditionSummary,
} from '../types';

const CONDITION_LABELS: Record<string, string> = {
  A: 'No memory',
  B: 'Episode-only',
  C: 'Full Loom',
};

const STATUS_PILL: Record<string, string> = {
  completed: 'pill-success',
  failed: 'pill-error',
  running: 'pill-warning',
  pending: 'pill-neutral',
};

// ---------------------------------------------------------------------------
// Condition card
// ---------------------------------------------------------------------------

interface ConditionCardProps {
  label: string;
  condition: string;
  summary: ConditionSummary;
  baseline?: ConditionSummary;
  isWinner?: boolean;
}

const ConditionCard: React.FC<ConditionCardProps> = ({
  label,
  condition,
  summary,
  baseline,
  isWinner,
}) => {
  const precDelta =
    baseline && baseline.avg_precision > 0
      ? ((summary.avg_precision - baseline.avg_precision) / baseline.avg_precision) * 100
      : null;
  const tokDelta =
    baseline && baseline.avg_token_count > 0
      ? ((summary.avg_token_count - baseline.avg_token_count) / baseline.avg_token_count) * 100
      : null;

  return (
    <div className={`bench-card${isWinner ? ' winner' : ''}`}>
      <div className="bench-card-head">
        <h4>{label}</h4>
        {isWinner && (
          <span className="pill pill-success">
            <span className="dot" />
            best
          </span>
        )}
        {!isWinner && (
          <span className="cell-muted" style={{ fontFamily: 'var(--font-mono)', fontSize: 11 }}>
            cond. {condition}
          </span>
        )}
      </div>
      <BenchMetric
        label="Precision"
        value={`${(summary.avg_precision * 100).toFixed(1)}%`}
        delta={precDelta}
        positiveIsGood
      />
      <BenchMetric
        label="Tokens"
        value={summary.avg_token_count.toFixed(0)}
        delta={tokDelta}
        positiveIsGood={false}
      />
      <BenchMetric label="Success" value={`${(summary.success_rate * 100).toFixed(1)}%`} />
      <BenchMetric label="Latency" value={`${summary.avg_latency_ms.toFixed(0)} ms`} />
    </div>
  );
};

interface BenchMetricProps {
  label: string;
  value: string;
  delta?: number | null;
  /** True if positive delta is good (precision); false if negative is good (tokens). */
  positiveIsGood?: boolean;
}

const BenchMetric: React.FC<BenchMetricProps> = ({
  label,
  value,
  delta,
  positiveIsGood = true,
}) => {
  let deltaCls: string | null = null;
  if (delta != null && Math.abs(delta) > 0.1) {
    const isUp = positiveIsGood ? delta > 0 : delta < 0;
    deltaCls = isUp ? 'bench-delta up' : 'bench-delta down';
  }
  return (
    <div className="bench-metric">
      <span className="l">{label}</span>
      <span className="v">
        {value}
        {delta != null && Math.abs(delta) > 0.1 && deltaCls && (
          <span className={deltaCls}>
            {delta > 0 ? '+' : ''}
            {delta.toFixed(1)}%
          </span>
        )}
      </span>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Per-task results table
// ---------------------------------------------------------------------------

const ResultsTable: React.FC<{ results: BenchmarkTaskResult[] }> = ({ results }) => {
  const taskNames = [...new Set(results.map((r) => r.task_name))];
  const get = (task: string, cond: string) =>
    results.find((r) => r.task_name === task && r.condition === cond);

  return (
    <table className="tbl">
      <thead>
        <tr>
          <th>Task</th>
          <th className="cell-num">Precision (A · B · C)</th>
          <th className="cell-num">Tokens (A · B · C)</th>
          <th className="cell-num">Success (A · B · C)</th>
          <th className="cell-num">Latency (A · B · C)</th>
        </tr>
      </thead>
      <tbody>
        {taskNames.map((task) => {
          const a = get(task, 'A');
          const b = get(task, 'B');
          const c = get(task, 'C');
          return (
            <tr key={task}>
              <td className="cell-id">{task}</td>
              <td className="cell-num">
                <span style={{ color: 'var(--fg-muted)' }}>
                  {fmtPct(a?.precision)} · {fmtPct(b?.precision)}
                </span>{' '}
                <span style={{ color: 'var(--moss-700)', fontWeight: 600 }}>
                  {fmtPct(c?.precision)}
                </span>
              </td>
              <td className="cell-num">
                <span style={{ color: 'var(--fg-muted)' }}>
                  {a?.token_count ?? '—'} · {b?.token_count ?? '—'}
                </span>{' '}
                <span style={{ color: 'var(--moss-700)', fontWeight: 600 }}>
                  {c?.token_count ?? '—'}
                </span>
              </td>
              <td className="cell-num">
                {fmtBool(a?.task_success)} · {fmtBool(b?.task_success)} ·{' '}
                <span
                  style={{
                    color: c?.task_success ? 'var(--moss-700)' : 'var(--madder-700)',
                    fontWeight: 600,
                  }}
                >
                  {fmtBool(c?.task_success)}
                </span>
              </td>
              <td className="cell-num">
                {a?.latency_ms ?? '—'} · {b?.latency_ms ?? '—'} · {c?.latency_ms ?? '—'}
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
};

function fmtPct(v?: number): string {
  return v == null ? '—' : `${(v * 100).toFixed(0)}%`;
}
function fmtBool(v?: boolean): string {
  if (v == null) return '—';
  return v ? '✓' : '✗';
}

// ---------------------------------------------------------------------------
// Main page
// ---------------------------------------------------------------------------

export const BenchmarkPage: React.FC = () => {
  const { data: runs, loading, error, refetch } = useApi(() => getBenchmarkRuns(), []);
  const [selectedRun, setSelectedRun] = useState<BenchmarkComparison | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [detailError, setDetailError] = useState<string | null>(null);
  const [runningBenchmark, setRunningBenchmark] = useState(false);

  const handleSelectRun = useCallback(async (run: BenchmarkRun) => {
    setDetailLoading(true);
    setDetailError(null);
    try {
      const detail = await getBenchmarkDetail(run.id);
      setSelectedRun(detail);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to load benchmark detail';
      setDetailError(message);
    } finally {
      setDetailLoading(false);
    }
  }, []);

  const handleRunBenchmark = useCallback(async () => {
    setRunningBenchmark(true);
    try {
      const newRun = await runBenchmark();
      refetch();
      const detail = await getBenchmarkDetail(newRun.id);
      setSelectedRun(detail);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to run benchmark';
      setDetailError(message);
    } finally {
      setRunningBenchmark(false);
    }
  }, [refetch]);

  return (
    <>
      <div className="page-header">
        <div className="page-header-titles">
          <div className="page-eyebrow">Insights / Benchmarks</div>
          <h2>Benchmarks</h2>
          <p>
            A/B/C comparison: <strong>No memory</strong> baseline vs. <strong>episode-only</strong>{' '}
            retrieval vs. <strong>Full Loom</strong> — precision, tokens, success rate, latency.
          </p>
        </div>
        <button
          type="button"
          className="btn btn-primary"
          onClick={handleRunBenchmark}
          disabled={runningBenchmark}
        >
          {runningBenchmark ? 'Running…' : 'Run new benchmark'}
        </button>
      </div>

      {loading && <div className="loading">Loading runs…</div>}
      {error && <div className="error">{error}</div>}

      {runs && (
        <>
          <div className="section-head">
            <h3>
              Runs <span className="count-pill">{runs.length}</span>
            </h3>
          </div>
          {runs.length === 0 ? (
            <div className="empty-state">
              <h3>No benchmark runs yet</h3>
              <p>
                Click <strong>Run new benchmark</strong> to evaluate the three conditions across the
                current task suite.
              </p>
            </div>
          ) : (
            <table className="tbl">
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Created</th>
                  <th>Status</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {runs.map((run) => (
                  <tr
                    key={run.id}
                    onClick={() => handleSelectRun(run)}
                    style={
                      selectedRun?.run.id === run.id
                        ? { background: 'var(--surface-sunken)' }
                        : undefined
                    }
                  >
                    <td className="cell-id">{run.name}</td>
                    <td className="cell-muted" title={new Date(run.created_at).toLocaleString()}>
                      {relativeTime(run.created_at)}
                    </td>
                    <td>
                      <span className={`pill ${STATUS_PILL[run.status] ?? 'pill-neutral'}`}>
                        <span className="dot" />
                        {run.status}
                      </span>
                    </td>
                    <td>
                      <button
                        type="button"
                        className="btn btn-ghost"
                        onClick={(e) => {
                          e.stopPropagation();
                          handleSelectRun(run);
                        }}
                      >
                        View
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </>
      )}

      {detailLoading && <div className="loading">Loading benchmark detail…</div>}
      {detailError && <div className="error">{detailError}</div>}

      {selectedRun && (
        <>
          <div className="section-head" style={{ marginTop: 'var(--space-8)' }}>
            <h3>
              Conditions <span className="count-pill">{selectedRun.run.name}</span>
            </h3>
          </div>
          <div className="bench-grid">
            <ConditionCard
              label={CONDITION_LABELS.A}
              condition="A"
              summary={selectedRun.summary.condition_a}
            />
            <ConditionCard
              label={CONDITION_LABELS.B}
              condition="B"
              summary={selectedRun.summary.condition_b}
              baseline={selectedRun.summary.condition_a}
            />
            <ConditionCard
              label={CONDITION_LABELS.C}
              condition="C"
              summary={selectedRun.summary.condition_c}
              baseline={selectedRun.summary.condition_b}
              isWinner
            />
          </div>

          <div className="section-head">
            <h3>
              Per-task results <span className="count-pill">{selectedRun.results.length}</span>
            </h3>
          </div>
          <ResultsTable results={selectedRun.results} />
        </>
      )}
    </>
  );
};
