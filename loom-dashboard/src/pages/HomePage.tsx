import type React from 'react';
import { getPipelineHealth } from '../api/client';
import { useApi } from '../hooks/useApi';
import type { CountByKey } from '../types';

function inferMode(source: string): 'live' | 'seed' | 'vendor' {
  if (source.startsWith('vendor.') || source.startsWith('vendor-')) return 'vendor';
  if (source.startsWith('loom-seed') || source === 'loom_seed') return 'seed';
  return 'live';
}

const MODE_COLOR: Record<string, string> = {
  live:   'var(--moss-600)',
  seed:   'var(--saffron-500)',
  vendor: 'var(--indigo-400)',
};

const BreakdownPanel: React.FC<{ title: string; meta: string; items: CountByKey[]; colorFn?: (key: string) => string }> = ({
  title, meta, items, colorFn,
}) => {
  const max = Math.max(...items.map((i) => i.count), 1);
  return (
    <section className="panel">
      <h3>{title}</h3>
      <p className="panel-meta">{meta}</p>
      {items.length === 0 ? (
        <p style={{ color: 'var(--fg-muted)', fontSize: 'var(--text-sm)' }}>No data</p>
      ) : (
        items.slice(0, 8).map((item) => {
          const color = colorFn ? colorFn(item.key) : 'var(--indigo-400)';
          const pct = Math.round((item.count / max) * 100);
          return (
            <div className="bk-row" key={item.key}>
              <div className="bk-swatch" style={{ background: color }} />
              <div className="bk-key">{item.key}</div>
              <div className="bk-bar">
                <span style={{ width: `${pct}%`, background: color }} />
              </div>
              <div className="bk-count numeric">{item.count.toLocaleString()}</div>
            </div>
          );
        })
      )}
    </section>
  );
};

export const HomePage: React.FC = () => {
  const { data, loading, error } = useApi(() => getPipelineHealth(), []);

  return (
    <div>
      <div className="page-header">
        <div className="page-header-titles">
          <div className="page-eyebrow">Overview / Pipeline Health</div>
          <h2>Pipeline health</h2>
          <p>Episode ingestion, extraction queue, and current fact inventory.</p>
        </div>
      </div>

      {loading && <p className="loading">Loading…</p>}
      {error && <p className="error">{error}</p>}

      {data && (
        <>
          {/* KPI tiles */}
          <div className="kpi-grid">
            <div className="kpi accent">
              <div className="kpi-eyebrow">Current facts</div>
              <div className="kpi-value numeric">{data.facts_current.toLocaleString()}</div>
            </div>
            <div className="kpi">
              <div className="kpi-eyebrow">Superseded</div>
              <div className="kpi-value numeric">{data.facts_superseded.toLocaleString()}</div>
              {data.facts_current > 0 && (
                <div className="kpi-sub">
                  {((data.facts_superseded / (data.facts_current + data.facts_superseded)) * 100).toFixed(1)}% of total
                </div>
              )}
            </div>
            <div className="kpi">
              <div className="kpi-eyebrow">Queue depth</div>
              <div className="kpi-value numeric">{data.queue_depth}</div>
              <div className="kpi-sub">pending episodes</div>
            </div>
            <div className="kpi">
              <div className="kpi-eyebrow">Extraction model</div>
              <div className="kpi-value model">{data.extraction_model ?? '—'}</div>
            </div>
            <div className="kpi">
              <div className="kpi-eyebrow">Classification model</div>
              <div className="kpi-value model">{data.classification_model ?? '—'}</div>
            </div>
          </div>

          {/* Two-up panels */}
          <div className="panel-grid">
            <BreakdownPanel
              title="Episodes by source"
              meta={`${data.episodes_by_source.length} source${data.episodes_by_source.length !== 1 ? 's' : ''}`}
              items={data.episodes_by_source}
              colorFn={(key) => MODE_COLOR[inferMode(key)]}
            />
            <BreakdownPanel
              title="Entities by type"
              meta={`${data.entities_by_type.length} type${data.entities_by_type.length !== 1 ? 's' : ''}`}
              items={data.entities_by_type}
            />
          </div>

          {/* Episodes by namespace */}
          {data.episodes_by_namespace.length > 0 && (
            <>
              <div className="section-head">
                <h3>
                  Episodes by namespace
                  <span className="count-pill">{data.episodes_by_namespace.length}</span>
                </h3>
              </div>
              <table className="tbl">
                <thead>
                  <tr>
                    <th>Namespace</th>
                    <th className="cell-num">Episodes</th>
                  </tr>
                </thead>
                <tbody>
                  {data.episodes_by_namespace.map((row) => (
                    <tr key={row.key}>
                      <td className="cell-id">{row.key}</td>
                      <td className="cell-num numeric">{row.count.toLocaleString()}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </>
          )}
        </>
      )}
    </div>
  );
};
