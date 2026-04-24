/**
 * Runtime — btop-inspired dense live status view.
 *
 * Subscribes to /dashboard/api/stream/telemetry via EventSource and renders:
 *   - System row:    CPU gauge, memory gauge, Ollama model / compute badge
 *   - Pipeline row:  per-stage p50 latency bars, ingestion queue counters
 *   - Errors row:    tail of the 10 most-recent failed episodes
 *
 * Sparklines are inline SVG — no charting library. All colors come from the
 * design-system tokens in `design-system.css`.
 */
import type { CSSProperties } from 'react';
import { useTelemetryStream } from '../hooks/useTelemetryStream';
import type { DataPoint, ExtractionError } from '../types';

// ── Threshold helpers ────────────────────────────────────────────────────────

type Health = 'ok' | 'warn' | 'crit';

function cpuHealth(pct: number): Health {
  if (pct >= 90) return 'crit';
  if (pct >= 70) return 'warn';
  return 'ok';
}

function memHealth(usedMib: number, totalMib: number): Health {
  if (totalMib === 0) return 'ok';
  const pct = (usedMib / totalMib) * 100;
  if (pct >= 90) return 'crit';
  if (pct >= 75) return 'warn';
  return 'ok';
}

function latencyHealth(ms: number | null): Health {
  if (ms === null) return 'ok';
  if (ms >= 1000) return 'crit';
  if (ms >= 500) return 'warn';
  return 'ok';
}

function queueHealth(depth: number): Health {
  if (depth >= 500) return 'crit';
  if (depth >= 100) return 'warn';
  return 'ok';
}

function failedHealth(count: number): Health {
  if (count >= 10) return 'crit';
  if (count >= 1) return 'warn';
  return 'ok';
}

const HEALTH_COLOR: Record<Health, string> = {
  ok: 'var(--signal-success)',
  warn: 'var(--signal-warning)',
  crit: 'var(--signal-error)',
};

const HEALTH_BG: Record<Health, string> = {
  ok: 'var(--signal-success-bg)',
  warn: 'var(--signal-warning-bg)',
  crit: 'var(--signal-error-bg)',
};

// ── Inline SVG sparkline ─────────────────────────────────────────────────────

function Sparkline({
  data,
  width = 240,
  height = 40,
  color = 'var(--signal-info)',
}: {
  data: DataPoint[];
  width?: number;
  height?: number;
  color?: string;
}) {
  if (data.length < 2) {
    return <svg width={width} height={height} aria-hidden="true" />;
  }

  const values = data.map((d) => d.v);
  const min = Math.min(...values);
  const max = Math.max(...values);
  const range = max - min || 1;

  const xStep = width / (data.length - 1);
  const yScale = (v: number) => height - ((v - min) / range) * (height - 4) - 2;

  const points = data.map((d, i) => `${i * xStep},${yScale(d.v)}`).join(' ');
  const areaPoints = `0,${height} ${points} ${(data.length - 1) * xStep},${height}`;

  return (
    <svg width={width} height={height} style={{ display: 'block' }} aria-hidden="true">
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

// ── Bar gauge ────────────────────────────────────────────────────────────────

function BarGauge({
  value,
  max,
  health,
  label,
  unit = '%',
}: {
  value: number;
  max: number;
  health: Health;
  label: string;
  unit?: string;
}) {
  const pct = Math.min((value / max) * 100, 100);
  const color = HEALTH_COLOR[health];

  return (
    <div style={{ marginBottom: 8 }}>
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--text-xs)',
          marginBottom: 2,
        }}
      >
        <span style={{ color: 'var(--fg-muted)' }}>{label}</span>
        <span style={{ color, fontWeight: 600 }}>
          {value.toFixed(0)}
          {unit}
        </span>
      </div>
      <div
        style={{
          background: 'var(--surface-sunken)',
          borderRadius: 2,
          height: 6,
          overflow: 'hidden',
        }}
      >
        <div
          style={{
            width: `${pct}%`,
            height: '100%',
            background: color,
            transition: 'width 0.3s ease, background 0.3s ease',
            borderRadius: 2,
          }}
        />
      </div>
    </div>
  );
}

// ── Latency stage row ────────────────────────────────────────────────────────

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
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          marginBottom: 4,
        }}
      >
        <span
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--text-xs)',
            color: 'var(--fg-disabled)',
            width: 70,
          }}
        >
          {label}
        </span>
        <span
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--text-xs)',
            color: 'var(--fg-disabled)',
          }}
        >
          —
        </span>
      </div>
    );
  }

  const health = isTotal ? latencyHealth(ms) : 'ok';
  const color = HEALTH_COLOR[health];
  const pct = Math.min((ms / maxMs) * 100, 100);

  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        marginBottom: 4,
      }}
    >
      <span
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--text-xs)',
          color: 'var(--fg-3)',
          width: 70,
          flexShrink: 0,
        }}
      >
        {label}
      </span>
      <div
        style={{
          flex: 1,
          background: 'var(--surface-sunken)',
          borderRadius: 2,
          height: 8,
        }}
      >
        <div
          style={{
            width: `${pct}%`,
            height: '100%',
            background: color,
            borderRadius: 2,
          }}
        />
      </div>
      <span
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--text-xs)',
          color,
          width: 60,
          textAlign: 'right',
          flexShrink: 0,
        }}
      >
        {ms.toFixed(0)} ms
      </span>
    </div>
  );
}

// ── Card wrapper ─────────────────────────────────────────────────────────────

const CARD_STYLE: CSSProperties = {
  border: '1px solid var(--border-1)',
  borderRadius: 6,
  padding: '12px 14px',
  background: 'var(--surface-card)',
};

function Card({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div style={CARD_STYLE}>
      <div
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 10,
          textTransform: 'uppercase',
          letterSpacing: '0.08em',
          color: 'var(--fg-muted)',
          marginBottom: 10,
        }}
      >
        {title}
      </div>
      {children}
    </div>
  );
}

// ── Error row ────────────────────────────────────────────────────────────────

function ErrorRow({ err }: { err: ExtractionError }) {
  return (
    <tr style={{ borderTop: '1px solid var(--border-1)' }}>
      <td
        style={{
          padding: '4px 8px 4px 0',
          color: 'var(--fg-muted)',
          whiteSpace: 'nowrap',
        }}
      >
        {err.occurred_at > 0 ? new Date(err.occurred_at).toLocaleTimeString() : '—'}
      </td>
      <td
        style={{
          padding: '4px 8px 4px 0',
          color: 'var(--fg-2)',
          whiteSpace: 'nowrap',
        }}
      >
        {err.source}
      </td>
      <td
        style={{
          padding: '4px 0',
          color: 'var(--signal-error)',
          maxWidth: 400,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        {err.error}
      </td>
      <td
        style={{
          padding: '4px 0 4px 8px',
          color: 'var(--fg-disabled)',
          textAlign: 'right',
          whiteSpace: 'nowrap',
        }}
      >
        {err.episode_id.slice(0, 8)}…
      </td>
    </tr>
  );
}

// ── Main page ────────────────────────────────────────────────────────────────

export function RuntimePage() {
  const snap = useTelemetryStream();

  if (!snap) {
    return (
      <div
        style={{
          padding: 24,
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--text-xs)',
          color: 'var(--fg-muted)',
        }}
      >
        connecting to telemetry stream…
      </div>
    );
  }

  const memPct = snap.mem_total_mib > 0 ? (snap.mem_used_mib / snap.mem_total_mib) * 100 : 0;
  const memHealthValue = memHealth(snap.mem_used_mib, snap.mem_total_mib);
  const cpuHealthValue = cpuHealth(snap.cpu_pct);
  const totalLatencyHealth = latencyHealth(snap.latency_total_p50_ms);

  return (
    <div style={{ padding: '24px 28px' }}>
      {/* Header */}
      <div style={{ marginBottom: 20 }}>
        <div
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 10,
            textTransform: 'uppercase',
            letterSpacing: '0.08em',
            color: 'var(--fg-disabled)',
            marginBottom: 4,
          }}
        >
          OVERVIEW / RUNTIME
        </div>
        <h1 style={{ margin: 0, fontSize: 22, fontWeight: 700, color: 'var(--fg-1)' }}>Runtime</h1>
        <div
          style={{
            fontSize: 'var(--text-sm)',
            color: 'var(--fg-muted)',
            marginTop: 4,
          }}
        >
          System resources, pipeline stage latency, and live activity.
        </div>
      </div>

      {/* Row 1: System */}
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: '1fr 1fr 1fr',
          gap: 12,
          marginBottom: 12,
        }}
      >
        {/* CPU */}
        <Card title="CPU">
          <BarGauge value={snap.cpu_pct} max={100} health={cpuHealthValue} label="usage" />
          <div style={{ marginTop: 8 }}>
            <Sparkline
              data={snap.sparkline_compilation_rate}
              color={HEALTH_COLOR[cpuHealthValue]}
            />
            <div
              style={{
                fontFamily: 'var(--font-mono)',
                fontSize: 10,
                color: 'var(--fg-muted)',
                marginTop: 2,
              }}
            >
              compilations/min (5 min)
            </div>
          </div>
        </Card>

        {/* Memory */}
        <Card title="Memory">
          <BarGauge value={memPct} max={100} health={memHealthValue} label="used" />
          <div
            style={{
              fontFamily: 'var(--font-mono)',
              fontSize: 'var(--text-xs)',
              color: 'var(--fg-3)',
              marginTop: 6,
            }}
          >
            {snap.mem_used_mib.toLocaleString()} / {snap.mem_total_mib.toLocaleString()} MiB
          </div>
          <div style={{ marginTop: 8 }}>
            <Sparkline data={snap.sparkline_ingestion_rate} color="var(--mode-vendor)" />
            <div
              style={{
                fontFamily: 'var(--font-mono)',
                fontSize: 10,
                color: 'var(--fg-muted)',
                marginTop: 2,
              }}
            >
              ingestions/min (5 min)
            </div>
          </div>
        </Card>

        {/* Ollama */}
        <Card title="Ollama / Model">
          {snap.ollama_model ? (
            <>
              <div
                style={{
                  fontFamily: 'var(--font-mono)',
                  fontSize: 'var(--text-sm)',
                  fontWeight: 600,
                  color: 'var(--fg-1)',
                  marginBottom: 6,
                  wordBreak: 'break-all',
                }}
              >
                {snap.ollama_model}
              </div>
              <div style={{ display: 'flex', gap: 8, marginBottom: 8 }}>
                <span
                  style={{
                    fontFamily: 'var(--font-mono)',
                    fontSize: 10,
                    padding: '2px 6px',
                    borderRadius: 3,
                    background: snap.ollama_on_gpu
                      ? 'var(--signal-success-bg)'
                      : 'var(--surface-sunken)',
                    color: snap.ollama_on_gpu ? 'var(--signal-success)' : 'var(--fg-3)',
                  }}
                >
                  {snap.ollama_on_gpu ? 'GPU' : 'CPU'}
                </span>
              </div>
              {snap.ollama_vram_mib !== null && (
                <div
                  style={{
                    fontFamily: 'var(--font-mono)',
                    fontSize: 'var(--text-xs)',
                    color: 'var(--fg-3)',
                  }}
                >
                  VRAM: {snap.ollama_vram_mib.toLocaleString()} MiB
                </div>
              )}
            </>
          ) : (
            <div
              style={{
                fontFamily: 'var(--font-mono)',
                fontSize: 'var(--text-xs)',
                color: 'var(--fg-disabled)',
              }}
            >
              no model loaded
            </div>
          )}
        </Card>
      </div>

      {/* Row 2: Pipeline */}
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: '1fr 1fr',
          gap: 12,
          marginBottom: 12,
        }}
      >
        <Card title="Pipeline Stage Latency (p50, last 60 compilations)">
          <StageLatency label="classify" ms={snap.latency_classify_p50_ms} />
          <StageLatency label="retrieve" ms={snap.latency_retrieve_p50_ms} />
          <StageLatency label="rank" ms={snap.latency_rank_p50_ms} />
          <StageLatency label="compile" ms={snap.latency_compile_p50_ms} />
          <div
            style={{
              borderTop: '1px solid var(--border-1)',
              margin: '8px 0',
            }}
          />
          <StageLatency label="total" ms={snap.latency_total_p50_ms} maxMs={3000} isTotal />
          <div style={{ marginTop: 12 }}>
            <Sparkline data={snap.sparkline_latency} color={HEALTH_COLOR[totalLatencyHealth]} />
            <div
              style={{
                fontFamily: 'var(--font-mono)',
                fontSize: 10,
                color: 'var(--fg-muted)',
                marginTop: 2,
              }}
            >
              total latency ms (5 min)
            </div>
          </div>
        </Card>

        <Card title="Ingestion Queue">
          <div
            style={{
              display: 'grid',
              gridTemplateColumns: '1fr 1fr 1fr',
              gap: 8,
              marginBottom: 14,
            }}
          >
            {[
              {
                label: 'active',
                value: snap.active_ingestions,
                health: 'ok' as const,
              },
              {
                label: 'pending',
                value: snap.queue_depth,
                health: queueHealth(snap.queue_depth),
              },
              {
                label: 'failed',
                value: snap.failed_episodes,
                health: failedHealth(snap.failed_episodes),
              },
            ].map(({ label, value, health }) => (
              <div
                key={label}
                style={{
                  textAlign: 'center',
                  padding: '6px 4px',
                  borderRadius: 4,
                  background: health === 'ok' ? 'transparent' : HEALTH_BG[health],
                }}
              >
                <div
                  style={{
                    fontFamily: 'var(--font-mono)',
                    fontSize: 24,
                    fontWeight: 700,
                    color: HEALTH_COLOR[health],
                    lineHeight: 1.1,
                  }}
                >
                  {value.toLocaleString()}
                </div>
                <div
                  style={{
                    fontFamily: 'var(--font-mono)',
                    fontSize: 10,
                    color: 'var(--fg-muted)',
                    marginTop: 2,
                  }}
                >
                  {label}
                </div>
              </div>
            ))}
          </div>
          <Sparkline data={snap.sparkline_ingestion_rate} color="var(--mode-vendor)" />
          <div
            style={{
              fontFamily: 'var(--font-mono)',
              fontSize: 10,
              color: 'var(--fg-muted)',
              marginTop: 2,
            }}
          >
            ingestions/min (5 min)
          </div>
        </Card>
      </div>

      {/* Row 3: Errors */}
      {snap.recent_errors.length > 0 && (
        <Card title={`Recent Failures (${snap.recent_errors.length})`}>
          <div style={{ overflowX: 'auto' }}>
            <table
              style={{
                width: '100%',
                borderCollapse: 'collapse',
                fontFamily: 'var(--font-mono)',
                fontSize: 'var(--text-xs)',
              }}
            >
              <thead>
                <tr style={{ color: 'var(--fg-muted)' }}>
                  <th
                    style={{
                      textAlign: 'left',
                      padding: '2px 8px 6px 0',
                      fontWeight: 400,
                    }}
                  >
                    time
                  </th>
                  <th
                    style={{
                      textAlign: 'left',
                      padding: '2px 8px 6px 0',
                      fontWeight: 400,
                    }}
                  >
                    source
                  </th>
                  <th
                    style={{
                      textAlign: 'left',
                      padding: '2px 0 6px 0',
                      fontWeight: 400,
                    }}
                  >
                    error
                  </th>
                  <th
                    style={{
                      textAlign: 'right',
                      padding: '2px 0 6px 8px',
                      fontWeight: 400,
                    }}
                  >
                    episode
                  </th>
                </tr>
              </thead>
              <tbody>
                {[...snap.recent_errors].reverse().map((err) => (
                  <ErrorRow key={err.episode_id} err={err} />
                ))}
              </tbody>
            </table>
          </div>
        </Card>
      )}
    </div>
  );
}
