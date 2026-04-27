/**
 * Benchmark comparison page.
 *
 * A/B/C condition comparison: No memory · Episode-only · Full Loom.
 * Every condition calls the LLM, so the headline `precision` column is
 * comparable across the row — it measures fraction of expected
 * entities mentioned in the LLM answer. B and C also surface
 * predicate-aware retrieval precision (with hydrated entity names) in
 * the per-task drill-down.
 *
 * The visual punch is in the winner treatment — Condition C picks up
 * a moss accent + warp-thread top border + "winner" pill, with the
 * +precision / −tokens deltas in palette-correct moss/madder pills.
 *
 * Per-task results render as a flat .tbl with three numeric columns
 * per metric (A/B/C). Click a row to expand and see the LLM answer,
 * retrieval-side metrics, and per-condition diagnostics.
 */
import type React from 'react';
import { Fragment, useCallback, useState } from 'react';
import { getBenchmarkDetail, getBenchmarkRuns, runBenchmark, seedBenchmark } from '../api/client';
import { useApi } from '../hooks/useApi';
import { relativeTime } from '../lib/thresholds';
import type {
  BenchmarkComparison,
  BenchmarkRun,
  BenchmarkTaskDetails,
  BenchmarkTaskResult,
  ConditionSummary,
  SeedSummary,
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
// How-to-read explainer
// ---------------------------------------------------------------------------

const HowToRead: React.FC = () => {
  const [open, setOpen] = useState(false);
  return (
    <div className={`bench-help${open ? ' open' : ''}`}>
      <button
        type="button"
        className="bench-help-toggle"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
      >
        <span className="bench-help-icon" aria-hidden>
          {open ? '−' : '?'}
        </span>
        How to read these cards
      </button>
      {open && (
        <div className="bench-help-body">
          <p>
            Each task is run three times against the same query. All three call the LLM, so the{' '}
            <strong>Precision</strong> number is the same kind of measurement across the row: the
            fraction of the task's expected entities that the LLM mentioned in its answer.
          </p>
          <ul>
            <li>
              <strong>No memory (A)</strong> — bare query, no retrieved context. The lower bound:
              how well does the LLM answer from training alone?
            </li>
            <li>
              <strong>Episode-only (B)</strong> — only the episode-recall profile runs. The compiled
              context is passed to the LLM. Tokens reflect the input-context cost.
            </li>
            <li>
              <strong>Full Loom (C)</strong> — every retrieval profile for the task class, plus
              hot-tier items. Same LLM, more (and better-ranked) context.
            </li>
          </ul>
          <p className="bench-help-note">
            Click a row in the per-task table to see the LLM's answer and B/C's retrieval-side
            metrics (entity recall, predicate recall, candidates considered). If every row shows 0%
            across all three conditions, the <code>benchmark</code> namespace is probably empty —
            click <strong>Seed benchmark data</strong> at the top of the page.
          </p>
        </div>
      )}
    </div>
  );
};

// ---------------------------------------------------------------------------
// Empty-namespace hint
// ---------------------------------------------------------------------------

interface EmptyHintProps {
  comparison: BenchmarkComparison;
}

/** Show an explanatory hint when B and C compiled essentially nothing — the
 * common cause is an unpopulated `benchmark` namespace. We anchor on token
 * counts because they bypass the LLM's prior knowledge. */
const EmptyNamespaceHint: React.FC<EmptyHintProps> = ({ comparison }) => {
  const b = comparison.summary.condition_b;
  const c = comparison.summary.condition_c;
  const looksEmpty = b.avg_token_count < 50 && c.avg_token_count < 50;
  if (!looksEmpty) return null;
  return (
    <div className="bench-empty-hint">
      <strong>Looks like the benchmark namespace is empty.</strong> Conditions B and C compiled
      almost no context (under 50 tokens on average — typical for the empty JSON wrapper), so they
      have nothing extra to feed the LLM and should be expected to score similarly to A. Click{' '}
      <strong>Seed benchmark data</strong> at the top of the page, wait for extraction to finish on
      the Compilations page, then re-run. (Or post the corpus from the CLI:{' '}
      <code>cli/loom-seed.py --namespace benchmark loom-engine/seed/benchmark/</code>.)
    </div>
  );
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
// Per-task results table with row expansion
// ---------------------------------------------------------------------------

const ResultsTable: React.FC<{ results: BenchmarkTaskResult[] }> = ({ results }) => {
  const taskNames = [...new Set(results.map((r) => r.task_name))];
  const [expanded, setExpanded] = useState<string | null>(null);
  const get = (task: string, cond: string) =>
    results.find((r) => r.task_name === task && r.condition === cond);

  return (
    <table className="tbl bench-results">
      <thead>
        <tr>
          <th />
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
          const isOpen = expanded === task;
          return (
            <Fragment key={task}>
              <tr
                className={`bench-row${isOpen ? ' open' : ''}`}
                onClick={() => setExpanded(isOpen ? null : task)}
              >
                <td className="bench-row-toggle" aria-hidden>
                  {isOpen ? '▾' : '▸'}
                </td>
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
              {isOpen && (
                <tr className="bench-row-detail">
                  <td colSpan={6}>
                    <TaskDetail a={a} b={b} c={c} />
                  </td>
                </tr>
              )}
            </Fragment>
          );
        })}
      </tbody>
    </table>
  );
};

// ---------------------------------------------------------------------------
// Per-task expanded detail panel
// ---------------------------------------------------------------------------

interface TaskDetailProps {
  a?: BenchmarkTaskResult;
  b?: BenchmarkTaskResult;
  c?: BenchmarkTaskResult;
}

const TaskDetail: React.FC<TaskDetailProps> = ({ a, b, c }) => {
  const query = a?.details?.query ?? b?.details?.query ?? c?.details?.query;
  return (
    <div className="bench-detail">
      {query && (
        <div className="bench-detail-query">
          <span className="bench-detail-label">Query</span>
          <code>{query}</code>
        </div>
      )}
      <div className="bench-detail-grid">
        <ConditionDetail label={CONDITION_LABELS.A} condition="A" result={a} />
        <ConditionDetail label={CONDITION_LABELS.B} condition="B" result={b} />
        <ConditionDetail label={CONDITION_LABELS.C} condition="C" result={c} />
      </div>
    </div>
  );
};

interface ConditionDetailProps {
  label: string;
  condition: string;
  result?: BenchmarkTaskResult;
}

const ConditionDetail: React.FC<ConditionDetailProps> = ({ label, condition, result }) => {
  if (!result) {
    return (
      <div className="bench-detail-card">
        <h5>
          {label} <span className="bench-detail-cond">cond. {condition}</span>
        </h5>
        <p className="bench-detail-empty">No data for this condition.</p>
      </div>
    );
  }
  const d = result.details ?? {};
  return (
    <div className="bench-detail-card">
      <h5>
        {label} <span className="bench-detail-cond">cond. {condition}</span>
      </h5>
      {d.error ? (
        <p className="bench-detail-error">
          <strong>Error:</strong> {d.error}
        </p>
      ) : null}
      {d.answer ? (
        <div className="bench-detail-answer">
          <span className="bench-detail-label">Answer</span>
          <p>{d.answer}</p>
        </div>
      ) : (
        !d.error && <p className="bench-detail-empty">No answer recorded.</p>
      )}
      <DetailMetrics result={result} details={d} />
    </div>
  );
};

interface DetailMetricsProps {
  result: BenchmarkTaskResult;
  details: BenchmarkTaskDetails;
}

const DetailMetrics: React.FC<DetailMetricsProps> = ({ result, details }) => (
  <dl className="bench-detail-metrics">
    <dt>Answer precision</dt>
    <dd>{fmtPct(result.precision)}</dd>
    {details.retrieval_precision != null && (
      <>
        <dt>Retrieval precision</dt>
        <dd>{fmtPct(details.retrieval_precision)}</dd>
        <dt>Entity recall</dt>
        <dd>{fmtPct(details.entity_recall)}</dd>
        <dt>Predicate recall</dt>
        <dd>{fmtPct(details.predicate_recall)}</dd>
      </>
    )}
    {details.candidates_found != null && (
      <>
        <dt>Candidates</dt>
        <dd>
          {details.candidates_found} found
          {details.candidates_selected != null && <>, {details.candidates_selected} selected</>}
        </dd>
      </>
    )}
    <dt>Tokens</dt>
    <dd>{result.token_count}</dd>
    <dt>Latency</dt>
    <dd>{result.latency_ms} ms</dd>
  </dl>
);

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
  const [seeding, setSeeding] = useState(false);
  const [seedResult, setSeedResult] = useState<SeedSummary | null>(null);
  const [seedError, setSeedError] = useState<string | null>(null);

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

  const handleSeed = useCallback(async () => {
    setSeeding(true);
    setSeedError(null);
    try {
      const summary = await seedBenchmark();
      setSeedResult(summary);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to seed benchmark namespace';
      setSeedError(message);
    } finally {
      setSeeding(false);
    }
  }, []);

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
        <div className="page-header-actions">
          <button
            type="button"
            className="btn btn-secondary"
            onClick={handleSeed}
            disabled={seeding}
            title="Post the embedded benchmark seed corpus to the `benchmark` namespace. Safe to click multiple times — duplicates are skipped."
          >
            {seeding ? 'Seeding…' : 'Seed benchmark data'}
          </button>
          <button
            type="button"
            className="btn btn-primary"
            onClick={handleRunBenchmark}
            disabled={runningBenchmark}
          >
            {runningBenchmark ? 'Running…' : 'Run new benchmark'}
          </button>
        </div>
      </div>

      {seedResult && (
        <div className="bench-seed-result" role="status">
          <strong>Seed corpus posted.</strong>{' '}
          {seedResult.inserted > 0 ? (
            <>
              {seedResult.inserted} new episode{seedResult.inserted === 1 ? '' : 's'} queued for
              extraction
              {seedResult.duplicates > 0 && <>, {seedResult.duplicates} already present</>}. Wait
              for extraction to complete (visible on the Compilations or Entities page) before
              running the benchmark — graph and fact retrieval need facts to be extracted first.
            </>
          ) : (
            <>
              All {seedResult.duplicates} seed episode
              {seedResult.duplicates === 1 ? '' : 's'} already present. Nothing changed.
            </>
          )}
        </div>
      )}
      {seedError && <div className="error">{seedError}</div>}

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
                First time? Click <strong>Seed benchmark data</strong> to load the embedded corpus
                into the <code>benchmark</code> namespace, wait for extraction to finish on the
                Compilations page, then click <strong>Run new benchmark</strong> to evaluate the
                three conditions.
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
          <HowToRead />
          <EmptyNamespaceHint comparison={selectedRun} />
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
