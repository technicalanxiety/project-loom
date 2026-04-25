/**
 * Runtime — live SSE-driven status view.
 *
 * Subscribes to /dashboard/api/stream/telemetry via EventSource and renders:
 *   - Page header with a `.pulse` ring while connected, `.pill-error` when not
 *   - KPI strip with the headline numbers (CPU, memory, total p50, queue, model)
 *   - System resources panel-grid (CPU + memory) with `.gauge` + sparkline
 *   - Pipeline panel-grid (stage latency + ingestion queue) with `.stage-row`
 *     and `.queue-grid`
 *   - Recent failures `.tbl` with mode-pilled source column
 *
 * All visual chrome comes from App.css utilities and the shared design tokens.
 * Inline styles are reserved for dynamic geometry — sparkline points, bar
 * widths — that can't live in CSS.
 */
import { useTelemetryStream } from '../hooks/useTelemetryStream';
import {
  cpuTone,
  failedTone,
  latencyTone,
  memTone,
  queueTone,
  relativeTime,
  TONE_CLASS,
  type Tone,
} from '../lib/thresholds';
import type { DataPoint, ExtractionError } from '../types';

// ── Inline SVG sparkline ─────────────────────────────────────────────────────

function Sparkline({ data, color = 'var(--indigo-500)' }: { data: DataPoint[]; color?: string }) {
  if (data.length < 2) {
    return <svg viewBox="0 0 240 40" preserveAspectRatio="none" aria-hidden="true" />;
  }

  const values = data.map((d) => d.v);
  const min = Math.min(...values);
  const max = Math.max(...values);
  const range = max - min || 1;

  const width = 240;
  const height = 40;
  const xStep = width / (data.length - 1);
  const yScale = (v: number) => height - ((v - min) / range) * (height - 4) - 2;

  const points = data.map((d, i) => `${i * xStep},${yScale(d.v)}`).join(' ');
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

// ── Pipeline stage row ───────────────────────────────────────────────────────

function StageLatency({
  label,
  ms,
  maxMs = 2000,
  isTotal = false,
}: {
  label: string;
  ms: number | null;
  maxMs?: number;
  isTotal?: boolean;
}) {
  if (ms === null) {
    return (
      <div className={`stage-row${isTotal ? ' total' : ''} disabled`}>
        <span className="stage-label">{label}</span>
        <div className="stage-bar">
          <span style={{ width: '0%' }} />
        </div>
        <span className="stage-value">—</span>
      </div>
    );
  }

  const tone = isTotal ? latencyTone(ms) : 'ok';
  const pct = Math.min((ms / maxMs) * 100, 100);
  const className = `stage-row${isTotal ? ' total' : ''}${
    isTotal && tone !== 'ok' ? ` ${TONE_CLASS[tone]}` : ''
  }`;

  return (
    <div className={className}>
      <span className="stage-label">{label}</span>
      <div className="stage-bar">
        <span style={{ width: `${pct}%` }} />
      </div>
      <span className="stage-value">{ms.toFixed(0)} ms</span>
    </div>
  );
}

// ── Bar gauge (CPU / memory) ─────────────────────────────────────────────────

function Gauge({
  value,
  label,
  tone,
  foot,
}: {
  value: number;
  label: string;
  tone: Tone;
  foot?: string;
}) {
  const pct = Math.min(value, 100);
  const className = tone === 'ok' ? 'gauge' : `gauge ${TONE_CLASS[tone]}`;
  return (
    <div className={className}>
      <div className="gauge-head">
        <span className="gauge-label">{label}</span>
        <span className="gauge-value">{value.toFixed(0)}%</span>
      </div>
      <div className="gauge-track">
        <div className="gauge-fill" style={{ width: `${pct}%` }} />
      </div>
      {foot && <div className="gauge-foot">{foot}</div>}
    </div>
  );
}

// ── Failure-row source pill ──────────────────────────────────────────────────

function SourcePill({ source }: { source: string }) {
  // Heuristic: the engine stamps 'source' from the ingestion path.
  // 'claude-code', 'mcp-*', 'webhook-*' → live; 'seed-*' → seed; everything
  // else (notion, slack-export, github-issues, …) → vendor.
  const lower = source.toLowerCase();
  const cls =
    lower.startsWith('seed') || lower === 'loom-seed'
      ? 'pill-seed'
      : lower.startsWith('claude') || lower.startsWith('mcp') || lower.startsWith('webhook')
        ? 'pill-live'
        : 'pill-vendor';
  return (
    <span className={`pill ${cls}`}>
      <span className="dot" />
      {source}
    </span>
  );
}

// ── Failures table row ───────────────────────────────────────────────────────

function FailureRow({ err }: { err: ExtractionError }) {
  const time = err.occurred_at > 0 ? relativeTime(new Date(err.occurred_at).toISOString()) : '—';
  return (
    <tr>
      <td className="cell-muted" title={new Date(err.occurred_at).toLocaleString()}>
        {time}
      </td>
      <td>
        <SourcePill source={err.source} />
      </td>
      <td style={{ color: 'var(--signal-error)' }}>{err.error}</td>
      <td className="cell-id cell-num">{err.episode_id.slice(0, 8)}…</td>
    </tr>
  );
}

// ── Main page ────────────────────────────────────────────────────────────────

export function RuntimePage() {
  const { snapshot: snap, connected } = useTelemetryStream();

  if (!snap) {
    return (
      <>
        <div className="page-header">
          <div className="page-header-titles">
            <div className="page-eyebrow">Overview / Runtime</div>
            <h2>Runtime</h2>
            <p>System resources, pipeline stage latency, and live activity.</p>
          </div>
        </div>
        <div className="loading">connecting to telemetry stream…</div>
      </>
    );
  }

  const memPct = snap.mem_total_mib > 0 ? (snap.mem_used_mib / snap.mem_total_mib) * 100 : 0;
  const cpuToneValue = cpuTone(snap.cpu_pct);
  const memToneValue = memTone(snap.mem_used_mib, snap.mem_total_mib);
  const totalLatencyTone = latencyTone(snap.latency_total_p50_ms);
  const queueToneValue = queueTone(snap.queue_depth);
  const failedToneValue = failedTone(snap.failed_episodes);

  return (
    <>
      <div className="page-header">
        <div className="page-header-titles">
          <div className="page-eyebrow">Overview / Runtime</div>
          <h2>Runtime</h2>
          <p>System resources, pipeline stage latency, and live activity.</p>
        </div>
        {connected ? (
          <span className="pulse">
            <span className="dot" />
            live · 1 Hz
          </span>
        ) : (
          <span className="pill pill-error">
            <span className="dot" />
            reconnecting
          </span>
        )}
      </div>

      {/* Headline numbers up top */}
      <div className="kpi-grid">
        <div className="kpi accent">
          <div className="kpi-eyebrow">CPU</div>
          <div className="kpi-value numeric" style={kpiToneStyle(cpuToneValue)}>
            {snap.cpu_pct.toFixed(0)}
            <span className="kpi-unit">%</span>
          </div>
        </div>
        <div className="kpi">
          <div className="kpi-eyebrow">Memory</div>
          <div className="kpi-value numeric" style={kpiToneStyle(memToneValue)}>
            {memPct.toFixed(0)}
            <span className="kpi-unit">%</span>
          </div>
          <div className="kpi-sub">
            {snap.mem_used_mib.toLocaleString()} / {snap.mem_total_mib.toLocaleString()} MiB
          </div>
        </div>
        <div className="kpi">
          <div className="kpi-eyebrow">Total p50 latency</div>
          <div className="kpi-value numeric" style={kpiToneStyle(totalLatencyTone)}>
            {snap.latency_total_p50_ms !== null ? snap.latency_total_p50_ms.toFixed(0) : '—'}
            <span className="kpi-unit"> ms</span>
          </div>
          <div className="kpi-sub">last 60 compilations</div>
        </div>
        <div className="kpi">
          <div className="kpi-eyebrow">Queue depth</div>
          <div className="kpi-value numeric" style={kpiToneStyle(queueToneValue)}>
            {snap.queue_depth.toLocaleString()}
          </div>
          <div className="kpi-sub">
            {snap.active_ingestions} active ·{' '}
            <span style={kpiToneStyle(failedToneValue)}>{snap.failed_episodes} failed</span>
          </div>
        </div>
        <div className="kpi">
          <div className="kpi-eyebrow">Active model</div>
          {snap.ollama_model ? (
            <>
              <div className="kpi-value model">{snap.ollama_model}</div>
              <div className="kpi-sub" style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
                <span className={`pill ${snap.ollama_on_gpu ? 'pill-success' : 'pill-neutral'}`}>
                  <span className="dot" />
                  {snap.ollama_on_gpu ? 'GPU' : 'CPU'}
                </span>
                {snap.ollama_vram_mib !== null && (
                  <span>VRAM {snap.ollama_vram_mib.toLocaleString()} MiB</span>
                )}
              </div>
            </>
          ) : (
            <>
              <div className="kpi-value model" style={{ color: 'var(--fg-disabled)' }}>
                no model loaded
              </div>
              <div className="kpi-sub">Ollama may be starting up</div>
            </>
          )}
        </div>
      </div>

      {/* System resources */}
      <div className="section-head">
        <h3>
          System resources <span className="count-pill">live</span>
        </h3>
      </div>
      <div className="panel-grid">
        <section className="panel">
          <h3>CPU</h3>
          <p className="panel-meta">host load, sampled every second</p>
          <Gauge value={snap.cpu_pct} label="usage" tone={cpuToneValue} />
          <div className="spark">
            <Sparkline
              data={snap.sparkline_compilation_rate}
              color={`var(--signal-${cpuToneValue === 'ok' ? 'success' : cpuToneValue === 'warn' ? 'warning' : 'error'})`}
            />
            <div className="spark-meta">compilations/min · 5 min</div>
          </div>
        </section>

        <section className="panel">
          <h3>Memory</h3>
          <p className="panel-meta">RSS as fraction of host total</p>
          <Gauge
            value={memPct}
            label="used"
            tone={memToneValue}
            foot={`${snap.mem_used_mib.toLocaleString()} / ${snap.mem_total_mib.toLocaleString()} MiB`}
          />
          <div className="spark">
            <Sparkline data={snap.sparkline_ingestion_rate} color="var(--mode-vendor)" />
            <div className="spark-meta">ingestions/min · 5 min</div>
          </div>
        </section>
      </div>

      {/* Pipeline */}
      <div className="section-head">
        <h3>
          Pipeline <span className="count-pill">last 60</span>
        </h3>
      </div>
      <div className="panel-grid">
        <section className="panel">
          <h3>Stage latency (p50)</h3>
          <p className="panel-meta">classify → retrieve → rank → compile</p>
          <StageLatency label="classify" ms={snap.latency_classify_p50_ms} />
          <StageLatency label="retrieve" ms={snap.latency_retrieve_p50_ms} />
          <StageLatency label="rank" ms={snap.latency_rank_p50_ms} />
          <StageLatency label="compile" ms={snap.latency_compile_p50_ms} />
          <StageLatency label="total" ms={snap.latency_total_p50_ms} maxMs={3000} isTotal />
          <div className="spark">
            <Sparkline
              data={snap.sparkline_latency}
              color={`var(--signal-${totalLatencyTone === 'ok' ? 'success' : totalLatencyTone === 'warn' ? 'warning' : 'error'})`}
            />
            <div className="spark-meta">total latency · 5 min</div>
          </div>
        </section>

        <section className="panel">
          <h3>Ingestion queue</h3>
          <p className="panel-meta">live counters, sampled 1 Hz</p>
          <div className="queue-grid">
            <div className="queue-stat">
              <div className="queue-stat-value">{snap.active_ingestions.toLocaleString()}</div>
              <div className="queue-stat-label">active</div>
            </div>
            <div
              className={`queue-stat${queueToneValue !== 'ok' ? ` ${TONE_CLASS[queueToneValue]}` : ''}`}
            >
              <div className="queue-stat-value">{snap.queue_depth.toLocaleString()}</div>
              <div className="queue-stat-label">pending</div>
            </div>
            <div
              className={`queue-stat${failedToneValue !== 'ok' ? ` ${TONE_CLASS[failedToneValue]}` : ''}`}
            >
              <div className="queue-stat-value">{snap.failed_episodes.toLocaleString()}</div>
              <div className="queue-stat-label">failed</div>
            </div>
          </div>
          <div className="spark">
            <Sparkline data={snap.sparkline_ingestion_rate} color="var(--mode-vendor)" />
            <div className="spark-meta">ingestions/min · 5 min</div>
          </div>
        </section>
      </div>

      {/* Recent failures */}
      {snap.recent_errors.length > 0 && (
        <>
          <div className="section-head">
            <h3>
              Recent failures <span className="count-pill">{snap.recent_errors.length}</span>
            </h3>
          </div>
          <table className="tbl">
            <thead>
              <tr>
                <th>Time</th>
                <th>Source</th>
                <th>Error</th>
                <th className="cell-num">Episode</th>
              </tr>
            </thead>
            <tbody>
              {[...snap.recent_errors].reverse().map((err) => (
                <FailureRow key={err.episode_id} err={err} />
              ))}
            </tbody>
          </table>
        </>
      )}
    </>
  );
}

// Map a tone to the colour the KPI value should pick up. Inline because
// `.kpi-value` is a generic block — adding `.tone-*` selectors at the
// .kpi-value level would collide with future uses.
function kpiToneStyle(tone: Tone): React.CSSProperties {
  if (tone === 'warn') return { color: 'var(--signal-warning)' };
  if (tone === 'crit') return { color: 'var(--signal-error)' };
  return {};
}
