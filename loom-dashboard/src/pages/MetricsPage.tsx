/**
 * Metrics and quality overview page.
 *
 * Displays retrieval quality (precision over time, latency percentiles),
 * extraction performance (model comparison, resolution distribution,
 * custom predicate growth), classification confidence distribution,
 * and hot-tier utilization per namespace.
 */
import type React from 'react';
import {
  getClassificationMetrics,
  getExtractionMetrics,
  getHotTierMetrics,
  getRetrievalMetrics,
} from '../api/client';
import { useApi } from '../hooks/useApi';
import type { CountByKey, DailyMetric } from '../types';

// ---------------------------------------------------------------------------
// Shared sub-components
// ---------------------------------------------------------------------------

/** Props for a simple bar chart rendered as horizontal bars. */
interface BarChartProps {
  items: CountByKey[];
  maxWidth?: number;
}

/** Horizontal bar chart for count-by-key data. */
const BarChart: React.FC<BarChartProps> = ({ items, maxWidth = 200 }) => {
  const maxCount = Math.max(...items.map((i) => i.count), 1);
  return (
    <div>
      {items.map((item) => (
        <div
          key={item.key}
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: '0.5rem',
            marginBottom: '0.35rem',
            fontSize: '0.8rem',
          }}
        >
          <span style={{ minWidth: '100px', textAlign: 'right' }}>{item.key}</span>
          <div
            style={{
              height: '16px',
              width: `${(item.count / maxCount) * maxWidth}px`,
              background: '#3a3a6a',
              borderRadius: '3px',
              minWidth: '2px',
            }}
          />
          <span style={{ fontWeight: 600 }}>{item.count}</span>
        </div>
      ))}
    </div>
  );
};

/** Props for a sparkline-style daily metric chart. */
interface SparklineProps {
  data: DailyMetric[];
  height?: number;
  color?: string;
  label?: string;
}

/** Simple SVG sparkline for daily metric data. */
const Sparkline: React.FC<SparklineProps> = ({ data, height = 80, color = '#3a3a6a', label }) => {
  if (data.length === 0) {
    return <p style={{ color: '#999', fontSize: '0.8rem' }}>No data points.</p>;
  }

  const width = 400;
  const padding = 4;
  const maxVal = Math.max(...data.map((d) => d.value), 0.01);
  const minVal = Math.min(...data.map((d) => d.value), 0);

  const points = data
    .map((d, i) => {
      const x = padding + (i / Math.max(data.length - 1, 1)) * (width - 2 * padding);
      const y =
        height - padding - ((d.value - minVal) / (maxVal - minVal || 1)) * (height - 2 * padding);
      return `${x},${y}`;
    })
    .join(' ');

  return (
    <div>
      {label && (
        <p style={{ fontSize: '0.75rem', color: '#888', marginBottom: '0.25rem' }}>{label}</p>
      )}
      <svg
        viewBox={`0 0 ${width} ${height}`}
        style={{ width: '100%', maxWidth: '400px' }}
        role="img"
        aria-label={label ?? 'Metric sparkline chart'}
      >
        <polyline
          points={points}
          fill="none"
          stroke={color}
          strokeWidth={2}
          strokeLinejoin="round"
        />
        {/* Dots on each point */}
        {data.map((d, i) => {
          const x = padding + (i / Math.max(data.length - 1, 1)) * (width - 2 * padding);
          const y =
            height -
            padding -
            ((d.value - minVal) / (maxVal - minVal || 1)) * (height - 2 * padding);
          return <circle key={d.date} cx={x} cy={y} r={2.5} fill={color} />;
        })}
      </svg>
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          fontSize: '0.7rem',
          color: '#999',
        }}
      >
        <span>{data[0]?.date ?? ''}</span>
        <span>{data[data.length - 1]?.date ?? ''}</span>
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Main page component
// ---------------------------------------------------------------------------

/** Combined metrics dashboard covering retrieval, extraction, classification, and hot-tier. */
export const MetricsPage: React.FC = () => {
  const { data: retrieval, loading: lr, error: er } = useApi(() => getRetrievalMetrics(), []);
  const { data: extraction, loading: le, error: ee } = useApi(() => getExtractionMetrics(), []);
  const {
    data: classification,
    loading: lc,
    error: ec,
  } = useApi(() => getClassificationMetrics(), []);
  const { data: hotTier, loading: lh, error: eh } = useApi(() => getHotTierMetrics(), []);

  const isLoading = lr || le || lc || lh;
  const errors = [er, ee, ec, eh].filter(Boolean);

  return (
    <div>
      <div className="page-header">
        <h2>Metrics</h2>
        <p>
          Retrieval quality, extraction performance, classification distribution, and hot-tier
          utilization.
        </p>
      </div>

      {isLoading && <p className="loading">Loading metrics…</p>}
      {errors.map((e) => (
        <p key={e} className="error">
          {e}
        </p>
      ))}

      {/* Retrieval Quality */}
      {retrieval && (
        <div className="card">
          <h3 style={{ fontSize: '0.95rem', marginBottom: '0.75rem' }}>Retrieval Quality</h3>

          <div
            style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(auto-fit, minmax(140px, 1fr))',
              gap: '0.75rem',
              marginBottom: '1rem',
            }}
          >
            {[
              { label: 'p50 Latency', value: retrieval.latency_p50, suffix: 'ms' },
              { label: 'p95 Latency', value: retrieval.latency_p95, suffix: 'ms' },
              { label: 'p99 Latency', value: retrieval.latency_p99, suffix: 'ms' },
            ].map((m) => (
              <div
                key={m.label}
                style={{
                  background: '#f5f6fa',
                  borderRadius: '6px',
                  padding: '0.75rem',
                  textAlign: 'center',
                }}
              >
                <p style={{ fontSize: '0.7rem', color: '#888', textTransform: 'uppercase' }}>
                  {m.label}
                </p>
                <p style={{ fontSize: '1.1rem', fontWeight: 700 }}>
                  {m.value != null ? `${m.value.toFixed(1)}${m.suffix}` : '—'}
                </p>
              </div>
            ))}
          </div>

          {retrieval.daily_precision.length > 0 && (
            <Sparkline
              data={retrieval.daily_precision}
              label="Daily Precision (last 30 days)"
              color="#27ae60"
            />
          )}
        </div>
      )}

      {/* Extraction Performance */}
      {extraction && (
        <div className="card">
          <h3 style={{ fontSize: '0.95rem', marginBottom: '0.75rem' }}>Extraction Performance</h3>

          {/* Model comparison table */}
          <h4 style={{ fontSize: '0.85rem', marginBottom: '0.5rem' }}>Model Comparison</h4>
          <table
            style={{
              width: '100%',
              borderCollapse: 'collapse',
              fontSize: '0.85rem',
              marginBottom: '1.25rem',
            }}
          >
            <thead>
              <tr style={{ borderBottom: '2px solid #eee', textAlign: 'left' }}>
                <th style={{ padding: '0.5rem' }}>Model</th>
                <th style={{ padding: '0.5rem' }}>Episodes</th>
                <th style={{ padding: '0.5rem' }}>Avg Entities</th>
                <th style={{ padding: '0.5rem' }}>Avg Facts</th>
              </tr>
            </thead>
            <tbody>
              {extraction.by_model.map((m) => (
                <tr key={m.model} style={{ borderBottom: '1px solid #f0f0f0' }}>
                  <td style={{ padding: '0.5rem' }}>{m.model}</td>
                  <td style={{ padding: '0.5rem' }}>{m.episode_count}</td>
                  <td style={{ padding: '0.5rem' }}>{m.avg_entity_count?.toFixed(1) ?? '—'}</td>
                  <td style={{ padding: '0.5rem' }}>{m.avg_fact_count?.toFixed(1) ?? '—'}</td>
                </tr>
              ))}
              {extraction.by_model.length === 0 && (
                <tr>
                  <td colSpan={4} className="placeholder">
                    No model data.
                  </td>
                </tr>
              )}
            </tbody>
          </table>

          {/* Resolution method distribution */}
          {extraction.resolution_distribution.length > 0 && (
            <div style={{ marginBottom: '1.25rem' }}>
              <h4 style={{ fontSize: '0.85rem', marginBottom: '0.5rem' }}>
                Entity Resolution Method Distribution
              </h4>
              <BarChart items={extraction.resolution_distribution} />
            </div>
          )}

          {/* Custom predicate growth */}
          {extraction.custom_predicate_growth.length > 0 && (
            <Sparkline
              data={extraction.custom_predicate_growth}
              label="Custom Predicate Growth Rate"
              color="#e67e22"
            />
          )}
        </div>
      )}

      {/* Classification Distribution */}
      {classification && (
        <div className="card">
          <h3 style={{ fontSize: '0.95rem', marginBottom: '0.75rem' }}>
            Classification Distribution
          </h3>
          <div
            style={{
              display: 'grid',
              gridTemplateColumns: 'repeat(auto-fit, minmax(250px, 1fr))',
              gap: '1.5rem',
            }}
          >
            <div>
              <h4 style={{ fontSize: '0.85rem', marginBottom: '0.5rem' }}>
                Confidence Distribution
              </h4>
              {classification.confidence_distribution.map((b) => {
                const maxCount = Math.max(
                  ...classification.confidence_distribution.map((x) => x.count),
                  1,
                );
                return (
                  <div
                    key={b.bucket}
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      gap: '0.5rem',
                      marginBottom: '0.25rem',
                      fontSize: '0.8rem',
                    }}
                  >
                    <span style={{ minWidth: '60px', textAlign: 'right' }}>{b.bucket}</span>
                    <div
                      style={{
                        height: '14px',
                        width: `${(b.count / maxCount) * 150}px`,
                        background: '#9b59b6',
                        borderRadius: '3px',
                        minWidth: '2px',
                      }}
                    />
                    <span style={{ fontWeight: 600 }}>{b.count}</span>
                  </div>
                );
              })}
            </div>
            <div>
              <h4 style={{ fontSize: '0.85rem', marginBottom: '0.5rem' }}>By Task Class</h4>
              <BarChart items={classification.class_distribution} />
            </div>
          </div>
        </div>
      )}

      {/* Hot-Tier Utilization */}
      {hotTier && (
        <div className="card">
          <h3 style={{ fontSize: '0.95rem', marginBottom: '0.75rem' }}>Hot-Tier Utilization</h3>
          <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: '0.85rem' }}>
            <thead>
              <tr style={{ borderBottom: '2px solid #eee', textAlign: 'left' }}>
                <th style={{ padding: '0.5rem' }}>Namespace</th>
                <th style={{ padding: '0.5rem' }}>Hot Entities</th>
                <th style={{ padding: '0.5rem' }}>Hot Facts</th>
                <th style={{ padding: '0.5rem' }}>Budget</th>
                <th style={{ padding: '0.5rem' }}>Utilization</th>
              </tr>
            </thead>
            <tbody>
              {hotTier.by_namespace.map((ns) => (
                <tr key={ns.namespace} style={{ borderBottom: '1px solid #f0f0f0' }}>
                  <td style={{ padding: '0.5rem' }}>{ns.namespace}</td>
                  <td style={{ padding: '0.5rem' }}>{ns.hot_entity_count}</td>
                  <td style={{ padding: '0.5rem' }}>{ns.hot_fact_count}</td>
                  <td style={{ padding: '0.5rem' }}>{ns.budget_tokens} tokens</td>
                  <td style={{ padding: '0.5rem' }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: '0.5rem' }}>
                      <div
                        style={{
                          width: '80px',
                          height: '10px',
                          background: '#eee',
                          borderRadius: '5px',
                          overflow: 'hidden',
                        }}
                      >
                        <div
                          style={{
                            width: `${Math.min(ns.utilization_pct, 100)}%`,
                            height: '100%',
                            background:
                              ns.utilization_pct > 90
                                ? '#e74c3c'
                                : ns.utilization_pct > 70
                                  ? '#f39c12'
                                  : '#27ae60',
                            borderRadius: '5px',
                          }}
                        />
                      </div>
                      <span style={{ fontWeight: 600 }}>{ns.utilization_pct.toFixed(1)}%</span>
                    </div>
                  </td>
                </tr>
              ))}
              {hotTier.by_namespace.length === 0 && (
                <tr>
                  <td colSpan={5} className="placeholder">
                    No hot-tier data.
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
