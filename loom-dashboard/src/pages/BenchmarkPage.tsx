/**
 * Benchmark comparison page.
 *
 * Displays benchmark evaluation results comparing three conditions:
 * - Condition A: No memory (baseline)
 * - Condition B: Episode-only retrieval
 * - Condition C: Full Loom pipeline
 *
 * Shows per-task results and aggregated summaries with precision,
 * token count, success rate, and latency metrics.
 */
import type React from 'react';
import { useCallback, useState } from 'react';
import { getBenchmarkDetail, getBenchmarkRuns, runBenchmark } from '../api/client';
import { useApi } from '../hooks/useApi';
import type {
  BenchmarkComparison,
  BenchmarkRun,
  BenchmarkTaskResult,
  ConditionSummary,
} from '../types';

/** Condition label mapping for display. */
const CONDITION_LABELS: Record<string, string> = {
  A: 'No Memory',
  B: 'Episode-Only',
  C: 'Full Loom',
};

/** Props for the condition summary card. */
interface ConditionCardProps {
  label: string;
  condition: string;
  summary: ConditionSummary;
  baseline?: ConditionSummary;
}

/** Displays aggregated metrics for a single condition with optional improvement indicators. */
const ConditionCard: React.FC<ConditionCardProps> = ({ label, condition, summary, baseline }) => {
  const precisionImprovement =
    baseline && baseline.avg_precision > 0
      ? ((summary.avg_precision - baseline.avg_precision) / baseline.avg_precision) * 100
      : null;

  const tokenReduction =
    baseline && baseline.avg_token_count > 0
      ? ((baseline.avg_token_count - summary.avg_token_count) / baseline.avg_token_count) * 100
      : null;

  return (
    <div
      style={{
        background: condition === 'C' ? '#f0f7ff' : '#f5f6fa',
        borderRadius: '8px',
        padding: '1rem',
        border: condition === 'C' ? '2px solid #3a3a6a' : '1px solid #eee',
      }}
    >
      <h4 style={{ fontSize: '0.9rem', marginBottom: '0.75rem', color: '#333' }}>
        Condition {condition}: {label}
      </h4>
      <div style={{ display: 'grid', gap: '0.5rem' }}>
        <MetricRow
          label="Avg Precision"
          value={`${(summary.avg_precision * 100).toFixed(1)}%`}
          improvement={precisionImprovement}
        />
        <MetricRow
          label="Avg Tokens"
          value={summary.avg_token_count.toFixed(0)}
          improvement={tokenReduction}
          suffix="reduction"
        />
        <MetricRow label="Success Rate" value={`${(summary.success_rate * 100).toFixed(1)}%`} />
        <MetricRow label="Avg Latency" value={`${summary.avg_latency_ms.toFixed(0)}ms`} />
      </div>
    </div>
  );
};

/** Props for a single metric row. */
interface MetricRowProps {
  label: string;
  value: string;
  improvement?: number | null;
  suffix?: string;
}

/** A single metric with optional improvement badge. */
const MetricRow: React.FC<MetricRowProps> = ({ label, value, improvement, suffix }) => (
  <div
    style={{
      display: 'flex',
      justifyContent: 'space-between',
      alignItems: 'center',
      fontSize: '0.85rem',
    }}
  >
    <span style={{ color: '#666' }}>{label}</span>
    <span style={{ display: 'flex', alignItems: 'center', gap: '0.4rem' }}>
      <strong>{value}</strong>
      {improvement != null && Math.abs(improvement) > 0.1 && (
        <span
          style={{
            fontSize: '0.7rem',
            padding: '0.1rem 0.3rem',
            borderRadius: '3px',
            background: improvement > 0 ? '#e8f5e9' : '#fbe9e7',
            color: improvement > 0 ? '#2e7d32' : '#c62828',
          }}
        >
          {improvement > 0 ? '+' : ''}
          {improvement.toFixed(1)}% {suffix ?? ''}
        </span>
      )}
    </span>
  </div>
);

/** Props for the results table. */
interface ResultsTableProps {
  results: BenchmarkTaskResult[];
}

/** Side-by-side A/B/C comparison table for per-task results. */
const ResultsTable: React.FC<ResultsTableProps> = ({ results }) => {
  // Group results by task name.
  const taskNames = [...new Set(results.map((r) => r.task_name))];

  const getResult = (taskName: string, condition: string): BenchmarkTaskResult | undefined =>
    results.find((r) => r.task_name === taskName && r.condition === condition);

  return (
    <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '0.8rem' }}>
      <thead>
        <tr style={{ borderBottom: '2px solid #ddd', textAlign: 'left' }}>
          <th style={{ padding: '0.5rem' }}>Task</th>
          <th style={{ padding: '0.5rem', textAlign: 'center' }} colSpan={3}>
            Precision
          </th>
          <th style={{ padding: '0.5rem', textAlign: 'center' }} colSpan={3}>
            Tokens
          </th>
          <th style={{ padding: '0.5rem', textAlign: 'center' }} colSpan={3}>
            Success
          </th>
          <th style={{ padding: '0.5rem', textAlign: 'center' }} colSpan={3}>
            Latency (ms)
          </th>
        </tr>
        <tr style={{ borderBottom: '1px solid #eee', fontSize: '0.7rem', color: '#888' }}>
          <th style={{ padding: '0.25rem 0.5rem' }} />
          {['A', 'B', 'C'].map((c) => (
            <th key={`p-${c}`} style={{ padding: '0.25rem', textAlign: 'center' }}>
              {c}
            </th>
          ))}
          {['A', 'B', 'C'].map((c) => (
            <th key={`t-${c}`} style={{ padding: '0.25rem', textAlign: 'center' }}>
              {c}
            </th>
          ))}
          {['A', 'B', 'C'].map((c) => (
            <th key={`s-${c}`} style={{ padding: '0.25rem', textAlign: 'center' }}>
              {c}
            </th>
          ))}
          {['A', 'B', 'C'].map((c) => (
            <th key={`l-${c}`} style={{ padding: '0.25rem', textAlign: 'center' }}>
              {c}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {taskNames.map((taskName) => {
          const a = getResult(taskName, 'A');
          const b = getResult(taskName, 'B');
          const c = getResult(taskName, 'C');
          return (
            <tr key={taskName} style={{ borderBottom: '1px solid #f0f0f0' }}>
              <td style={{ padding: '0.5rem', fontWeight: 500 }}>{taskName}</td>
              {/* Precision */}
              <td style={{ padding: '0.25rem', textAlign: 'center' }}>
                {a ? (a.precision * 100).toFixed(0) : '—'}%
              </td>
              <td style={{ padding: '0.25rem', textAlign: 'center' }}>
                {b ? (b.precision * 100).toFixed(0) : '—'}%
              </td>
              <td
                style={{
                  padding: '0.25rem',
                  textAlign: 'center',
                  fontWeight: 600,
                  color: '#2e7d32',
                }}
              >
                {c ? (c.precision * 100).toFixed(0) : '—'}%
              </td>
              {/* Tokens */}
              <td style={{ padding: '0.25rem', textAlign: 'center' }}>{a?.token_count ?? '—'}</td>
              <td style={{ padding: '0.25rem', textAlign: 'center' }}>{b?.token_count ?? '—'}</td>
              <td
                style={{
                  padding: '0.25rem',
                  textAlign: 'center',
                  fontWeight: 600,
                  color: '#2e7d32',
                }}
              >
                {c?.token_count ?? '—'}
              </td>
              {/* Success */}
              <td style={{ padding: '0.25rem', textAlign: 'center' }}>
                {a?.task_success ? '✓' : '✗'}
              </td>
              <td style={{ padding: '0.25rem', textAlign: 'center' }}>
                {b?.task_success ? '✓' : '✗'}
              </td>
              <td
                style={{
                  padding: '0.25rem',
                  textAlign: 'center',
                  fontWeight: 600,
                  color: c?.task_success ? '#2e7d32' : '#c62828',
                }}
              >
                {c?.task_success ? '✓' : '✗'}
              </td>
              {/* Latency */}
              <td style={{ padding: '0.25rem', textAlign: 'center' }}>{a?.latency_ms ?? '—'}</td>
              <td style={{ padding: '0.25rem', textAlign: 'center' }}>{b?.latency_ms ?? '—'}</td>
              <td style={{ padding: '0.25rem', textAlign: 'center' }}>{c?.latency_ms ?? '—'}</td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
};

/** Benchmark evaluation page with run list, trigger button, and A/B/C comparison. */
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
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : 'Failed to load benchmark detail';
      setDetailError(message);
      console.error('Failed to load benchmark detail:', err);
    } finally {
      setDetailLoading(false);
    }
  }, []);

  const handleRunBenchmark = useCallback(async () => {
    setRunningBenchmark(true);
    try {
      const newRun = await runBenchmark();
      refetch();
      // Auto-select the new run.
      const detail = await getBenchmarkDetail(newRun.id);
      setSelectedRun(detail);
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : 'Failed to run benchmark';
      setDetailError(message);
      console.error('Failed to run benchmark:', err);
    } finally {
      setRunningBenchmark(false);
    }
  }, [refetch]);

  return (
    <div>
      <div className="page-header">
        <h2>Benchmarks</h2>
        <p>A/B/C condition comparison: No Memory (A) vs Episode-Only (B) vs Full Loom (C).</p>
      </div>

      {/* Run controls */}
      <div className="card" style={{ marginBottom: '1rem' }}>
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
          <h3 style={{ fontSize: '0.95rem', margin: 0 }}>Benchmark Runs</h3>
          <button
            type="button"
            onClick={handleRunBenchmark}
            disabled={runningBenchmark}
            style={{
              padding: '0.5rem 1rem',
              background: runningBenchmark ? '#ccc' : '#3a3a6a',
              color: '#fff',
              border: 'none',
              borderRadius: '6px',
              cursor: runningBenchmark ? 'not-allowed' : 'pointer',
              fontSize: '0.85rem',
            }}
          >
            {runningBenchmark ? 'Running…' : 'Run New Benchmark'}
          </button>
        </div>

        {loading && (
          <p className="loading" style={{ marginTop: '0.75rem' }}>
            Loading runs…
          </p>
        )}
        {error && (
          <p className="error" style={{ marginTop: '0.75rem' }}>
            {error}
          </p>
        )}

        {runs && runs.length > 0 && (
          <table
            style={{
              width: '100%',
              borderCollapse: 'collapse',
              fontSize: '0.85rem',
              marginTop: '0.75rem',
            }}
          >
            <thead>
              <tr style={{ borderBottom: '2px solid #eee', textAlign: 'left' }}>
                <th style={{ padding: '0.5rem' }}>Name</th>
                <th style={{ padding: '0.5rem' }}>Created</th>
                <th style={{ padding: '0.5rem' }}>Status</th>
                <th style={{ padding: '0.5rem' }} />
              </tr>
            </thead>
            <tbody>
              {runs.map((run) => (
                <tr
                  key={run.id}
                  style={{
                    borderBottom: '1px solid #f0f0f0',
                    background: selectedRun?.run.id === run.id ? '#f0f7ff' : 'transparent',
                  }}
                >
                  <td style={{ padding: '0.5rem' }}>{run.name}</td>
                  <td style={{ padding: '0.5rem' }}>{new Date(run.created_at).toLocaleString()}</td>
                  <td style={{ padding: '0.5rem' }}>
                    <span
                      style={{
                        padding: '0.15rem 0.4rem',
                        borderRadius: '3px',
                        fontSize: '0.75rem',
                        background:
                          run.status === 'completed'
                            ? '#e8f5e9'
                            : run.status === 'failed'
                              ? '#fbe9e7'
                              : '#fff3e0',
                        color:
                          run.status === 'completed'
                            ? '#2e7d32'
                            : run.status === 'failed'
                              ? '#c62828'
                              : '#e65100',
                      }}
                    >
                      {run.status}
                    </span>
                  </td>
                  <td style={{ padding: '0.5rem' }}>
                    <button
                      type="button"
                      onClick={() => handleSelectRun(run)}
                      style={{
                        padding: '0.25rem 0.5rem',
                        background: 'transparent',
                        border: '1px solid #3a3a6a',
                        borderRadius: '4px',
                        cursor: 'pointer',
                        fontSize: '0.8rem',
                        color: '#3a3a6a',
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

        {runs && runs.length === 0 && (
          <p className="placeholder" style={{ marginTop: '0.75rem' }}>
            No benchmark runs yet. Click "Run New Benchmark" to start.
          </p>
        )}
      </div>

      {/* Detail loading/error */}
      {detailLoading && <p className="loading">Loading benchmark detail…</p>}
      {detailError && <p className="error">{detailError}</p>}

      {/* Benchmark comparison view */}
      {selectedRun && (
        <>
          {/* Summary cards */}
          <div className="card" style={{ marginBottom: '1rem' }}>
            <h3 style={{ fontSize: '0.95rem', marginBottom: '0.75rem' }}>
              Condition Summary — {selectedRun.run.name}
            </h3>
            <div
              style={{
                display: 'grid',
                gridTemplateColumns: 'repeat(auto-fit, minmax(220px, 1fr))',
                gap: '1rem',
              }}
            >
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
              />
            </div>
          </div>

          {/* Per-task results table */}
          <div className="card">
            <h3 style={{ fontSize: '0.95rem', marginBottom: '0.75rem' }}>Per-Task Results</h3>
            <div style={{ overflowX: 'auto' }}>
              <ResultsTable results={selectedRun.results} />
            </div>
          </div>
        </>
      )}
    </div>
  );
};
