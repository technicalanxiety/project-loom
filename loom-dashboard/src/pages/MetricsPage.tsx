/**
 * Metrics and quality overview.
 *
 * Top KPI strip with the four headline numbers (today's precision, p95
 * retrieval latency, active models, hot-tier utilisation), then a 2×2
 * grid of detail panels: retrieval, extraction, classification, hot-tier.
 *
 * All bar charts now flow through the shared `.bk-row` vocabulary, so
 * the page reads like an extension of HomePage rather than a separate
 * dashboard.
 */

import type React from 'react';
import { useMemo } from 'react';
import {
  getClassificationMetrics,
  getExtractionMetrics,
  getHotTierMetrics,
  getRetrievalMetrics,
} from '../api/client';
import { useApi } from '../hooks/useApi';
import { latencyTone, TONE_CLASS, utilizationTone } from '../lib/thresholds';
import type { CountByKey, DailyMetric } from '../types';

// ---------------------------------------------------------------------------
// Sparkline (shared with the rest of the dashboard's `.spark` wrapper)
// ---------------------------------------------------------------------------

function MetricSparkline({
  data,
  color = 'var(--indigo-500)',
}: {
  data: DailyMetric[];
  color?: string;
}) {
  if (data.length < 2) return <svg viewBox="0 0 240 40" preserveAspectRatio="none" />;

  const width = 240;
  const height = 40;
  const max = Math.max(...data.map((d) => d.value), 0.01);
  const min = Math.min(...data.map((d) => d.value), 0);
  const range = max - min || 1;

  const xStep = width / (data.length - 1);
  const yScale = (v: number) => height - ((v - min) / range) * (height - 4) - 2;
  const points = data.map((d, i) => `${i * xStep},${yScale(d.value)}`).join(' ');
  const areaPoints = `0,${height} ${points} ${(data.length - 1) * xStep},${height}`;

  return (
    <svg viewBox={`0 0 ${width} ${height}`} preserveAspectRatio="none" aria-hidden="true">
      <polygon points={areaPoints} fill={color} fillOpacity={0.15} />
      <polyline
        points={points}
        fill="none"
        stroke={color}
        strokeWidth={1.5}
        strokeLinejoin="round"
      />
    </svg>
  );
}

// ---------------------------------------------------------------------------
// Generic horizontal bar list, using the .bk-row vocabulary from App.css
// ---------------------------------------------------------------------------

interface BarListProps {
  items: { key: string; count: number }[];
  color?: string;
}

const BarList: React.FC<BarListProps> = ({ items, color = 'var(--indigo-500)' }) => {
  if (items.length === 0) return <div className="placeholder">No data.</div>;
  const max = Math.max(...items.map((i) => i.count), 1);
  return (
    <div>
      {items.map((item) => (
        <div className="bk-row" key={item.key}>
          <span />
          <span className="bk-key">{item.key}</span>
          <div className="bk-bar">
            <span style={{ width: `${(item.count / max) * 100}%`, background: color }} />
          </div>
          <span className="bk-count">{item.count.toLocaleString()}</span>
        </div>
      ))}
    </div>
  );
};

// ---------------------------------------------------------------------------
// Main page
// ---------------------------------------------------------------------------

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
  const errors = [er, ee, ec, eh].filter((s): s is string => Boolean(s));

  const todayPrecision = useMemo(() => {
    const series = retrieval?.daily_precision;
    return series && series.length > 0 ? series[series.length - 1].value : null;
  }, [retrieval]);
  const activeModels = extraction?.by_model.length ?? null;
  const overallHotUtil = useMemo(() => {
    if (!hotTier || hotTier.by_namespace.length === 0) return null;
    const total = hotTier.by_namespace.reduce((sum, ns) => sum + ns.utilization_pct, 0);
    return total / hotTier.by_namespace.length;
  }, [hotTier]);

  return (
    <>
      <div className="page-header">
        <div className="page-header-titles">
          <div className="page-eyebrow">Insights / Metrics</div>
          <h2>Metrics</h2>
          <p>
            Retrieval quality, extraction performance, classification distribution, and hot-tier
            utilization — at a glance and per panel.
          </p>
        </div>
      </div>

      {isLoading && <div className="loading">Loading metrics…</div>}
      {errors.map((e) => (
        <div className="error" key={e}>
          {e}
        </div>
      ))}

      {/* KPI strip */}
      <div className="kpi-grid">
        <div className="kpi accent">
          <div className="kpi-eyebrow">Today's precision</div>
          <div className="kpi-value numeric">
            {todayPrecision !== null ? `${(todayPrecision * 100).toFixed(0)}` : '—'}
            {todayPrecision !== null && <span className="kpi-unit">%</span>}
          </div>
          <div className="kpi-sub">latest sample</div>
        </div>
        <div className="kpi">
          <div className="kpi-eyebrow">p95 retrieval</div>
          <div
            className="kpi-value numeric"
            style={
              retrieval?.latency_p95 != null && latencyTone(retrieval.latency_p95) !== 'ok'
                ? {
                    color:
                      latencyTone(retrieval.latency_p95) === 'crit'
                        ? 'var(--signal-error)'
                        : 'var(--signal-warning)',
                  }
                : undefined
            }
          >
            {retrieval?.latency_p95 != null ? retrieval.latency_p95.toFixed(0) : '—'}
            <span className="kpi-unit">ms</span>
          </div>
        </div>
        <div className="kpi">
          <div className="kpi-eyebrow">Active models</div>
          <div className="kpi-value numeric">{activeModels ?? '—'}</div>
        </div>
        <div className="kpi">
          <div className="kpi-eyebrow">Hot-tier util</div>
          <div
            className="kpi-value numeric"
            style={
              overallHotUtil !== null && utilizationTone(overallHotUtil) !== 'ok'
                ? {
                    color:
                      utilizationTone(overallHotUtil) === 'crit'
                        ? 'var(--signal-error)'
                        : 'var(--signal-warning)',
                  }
                : undefined
            }
          >
            {overallHotUtil !== null ? overallHotUtil.toFixed(0) : '—'}
            {overallHotUtil !== null && <span className="kpi-unit">%</span>}
          </div>
          <div className="kpi-sub">avg across namespaces</div>
        </div>
      </div>

      {/* 2×2 panel grid */}
      <div className="panel-grid">
        {/* Retrieval */}
        <section className="panel">
          <h3>Retrieval quality</h3>
          <p className="panel-meta">latency percentiles · daily precision</p>
          <div className="bk-row">
            <span />
            <span className="bk-key">p50</span>
            <span />
            <span className="bk-count">
              {retrieval?.latency_p50 != null ? `${retrieval.latency_p50.toFixed(0)} ms` : '—'}
            </span>
          </div>
          <div className="bk-row">
            <span />
            <span className="bk-key">p95</span>
            <span />
            <span className="bk-count">
              {retrieval?.latency_p95 != null ? `${retrieval.latency_p95.toFixed(0)} ms` : '—'}
            </span>
          </div>
          <div className="bk-row">
            <span />
            <span className="bk-key">p99</span>
            <span />
            <span className="bk-count">
              {retrieval?.latency_p99 != null ? `${retrieval.latency_p99.toFixed(0)} ms` : '—'}
            </span>
          </div>
          {retrieval?.daily_precision.length ? (
            <div className="spark">
              <MetricSparkline data={retrieval.daily_precision} color="var(--moss-600)" />
              <div className="spark-meta">daily precision · last 30 days</div>
            </div>
          ) : null}
        </section>

        {/* Extraction */}
        <section className="panel">
          <h3>Extraction</h3>
          <p className="panel-meta">model comparison · resolution methods · custom predicates</p>
          {extraction?.by_model.length ? (
            <table className="tbl" style={{ marginBottom: 'var(--space-3)' }}>
              <thead>
                <tr>
                  <th>Model</th>
                  <th className="cell-num">Episodes</th>
                  <th className="cell-num">Avg E</th>
                  <th className="cell-num">Avg F</th>
                </tr>
              </thead>
              <tbody>
                {extraction.by_model.map((m) => (
                  <tr key={m.model}>
                    <td className="cell-id">{m.model}</td>
                    <td className="cell-num">{m.episode_count.toLocaleString()}</td>
                    <td className="cell-num">{m.avg_entity_count?.toFixed(1) ?? '—'}</td>
                    <td className="cell-num">{m.avg_fact_count?.toFixed(1) ?? '—'}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : (
            <div className="placeholder">No model data.</div>
          )}
          {extraction?.resolution_distribution.length ? (
            <>
              <div className="section-head" style={{ margin: '6px 0' }}>
                <h3 style={{ fontSize: 12 }}>Resolution methods</h3>
              </div>
              <BarList items={(extraction.resolution_distribution as CountByKey[]) ?? []} />
            </>
          ) : null}
          {extraction?.custom_predicate_growth.length ? (
            <div className="spark">
              <MetricSparkline
                data={extraction.custom_predicate_growth}
                color="var(--saffron-600)"
              />
              <div className="spark-meta">custom predicate growth · 30 days</div>
            </div>
          ) : null}
        </section>

        {/* Classification */}
        <section className="panel">
          <h3>Classification</h3>
          <p className="panel-meta">confidence distribution · task-class distribution</p>
          {classification?.confidence_distribution.length ? (
            <>
              <div className="section-head" style={{ margin: '0 0 6px' }}>
                <h3 style={{ fontSize: 12 }}>Confidence buckets</h3>
              </div>
              <BarList
                items={classification.confidence_distribution.map((b) => ({
                  key: b.bucket,
                  count: b.count,
                }))}
                color="var(--indigo-500)"
              />
            </>
          ) : null}
          {classification?.class_distribution.length ? (
            <>
              <div className="section-head" style={{ margin: '12px 0 6px' }}>
                <h3 style={{ fontSize: 12 }}>By task class</h3>
              </div>
              <BarList items={classification.class_distribution} color="var(--moss-600)" />
            </>
          ) : null}
        </section>

        {/* Hot tier */}
        <section className="panel">
          <h3>Hot-tier utilisation</h3>
          <p className="panel-meta">budget consumption per namespace</p>
          {hotTier?.by_namespace.length ? (
            <table className="tbl">
              <thead>
                <tr>
                  <th>Namespace</th>
                  <th className="cell-num">E</th>
                  <th className="cell-num">F</th>
                  <th className="cell-num">Budget</th>
                  <th>Utilisation</th>
                </tr>
              </thead>
              <tbody>
                {hotTier.by_namespace.map((ns) => {
                  const tone = utilizationTone(ns.utilization_pct);
                  return (
                    <tr key={ns.namespace}>
                      <td className="cell-id">{ns.namespace}</td>
                      <td className="cell-num">{ns.hot_entity_count}</td>
                      <td className="cell-num">{ns.hot_fact_count}</td>
                      <td className="cell-num">{ns.budget_tokens.toLocaleString()}</td>
                      <td>
                        <span
                          style={{
                            display: 'inline-flex',
                            alignItems: 'center',
                            gap: 6,
                            whiteSpace: 'nowrap',
                          }}
                        >
                          <span
                            className={`inline-bar ${TONE_CLASS[tone]}`}
                            style={{ width: 80 }}
                            aria-hidden="true"
                          >
                            <span style={{ width: `${Math.min(ns.utilization_pct, 100)}%` }} />
                          </span>
                          <span
                            className="cell-num"
                            style={{
                              minWidth: 40,
                              color:
                                tone === 'crit'
                                  ? 'var(--signal-error)'
                                  : tone === 'warn'
                                    ? 'var(--signal-warning)'
                                    : 'var(--fg-1)',
                              fontWeight: tone !== 'ok' ? 600 : 400,
                            }}
                          >
                            {ns.utilization_pct.toFixed(0)}%
                          </span>
                        </span>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          ) : (
            <div className="placeholder">No hot-tier data.</div>
          )}
        </section>
      </div>
    </>
  );
};
