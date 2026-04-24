# Streaming Telemetry — Implementation Spec

**For Claude Code.** This document is the authoritative implementation reference for
the streaming runtime telemetry view in Project Loom. Read it fully before writing
any code. Cross-reference `CLAUDE.md` for repo-wide invariants.

---

## What This Is

A Server-Sent Events (SSE) feed from the loom-engine that streams a live system
snapshot every second, consumed by a new React page (`RuntimePage`) that renders
a btop-inspired dense status view. No external monitoring stack. No new service.
The engine already has all the data — this adds the transport and the visual.

---

## What Is NOT Being Built

- Prometheus / Grafana / OpenTelemetry integration — out of scope, tool sprawl
- WebSockets — SSE is unidirectional read, which is all this needs
- NVML / NVIDIA management library — use Ollama's `/api/ps` endpoint for GPU state
- Historical time-series persistence — ring buffers in memory only, reset on restart
- Configurable alert thresholds in the UI — thresholds are hardcoded per the spec below
- New auth paths — the existing Caddy `header_up Authorization` injection is reused

---

## Auth — nothing to change

The dashboard SPA does **not** carry a bearer token. Caddy's `/dashboard/api/*`
handler injects `Authorization: Bearer {env.LOOM_BEARER_TOKEN}` on every proxied
request ([Caddyfile](Caddyfile)). `EventSource` traverses the same reverse proxy,
so Caddy adds the header transparently — SSE requires no dashboard-side token
handling, no `localStorage` read, and no middleware change. In dev, Vite's proxy
forwards without auth; existing dashboard GETs already rely on this, so SSE works
identically.

---

## Repo Orientation

```
loom-engine/
  Cargo.toml                        ← add sysinfo, async-stream, tokio-stream
  src/
    main.rs                         ← register SSE route + spawn sampler task
    api/
      mcp.rs                        ← add SharedTelemetry field to AppState
      dashboard.rs                  ← add SSE handler + TelemetrySnapshot type
    telemetry/                      ← NEW module
      mod.rs
      state.rs                      ← SharedTelemetry + TelemetryState
      sampler.rs                    ← background 1s sampling task

loom-dashboard/
  src/
    App.tsx                         ← add nav entry + route
    hooks/
      useTelemetryStream.ts         ← NEW SSE hook
    pages/
      RuntimePage.tsx               ← NEW btop-style view
    types/
      index.ts                      ← add TelemetrySnapshot type
```

---

## Phase 1: Rust Engine

### 1.1 Cargo.toml additions

Add to `[dependencies]` in `loom-engine/Cargo.toml`:

```toml
sysinfo      = "0.33"
async-stream = "0.3"
tokio-stream = "0.1"
```

### 1.2 New module: `src/telemetry/`

Create `loom-engine/src/telemetry/mod.rs`:

```rust
pub mod sampler;
pub mod state;

pub use state::{SharedTelemetry, TelemetryState};
```

---

### 1.3 `src/telemetry/state.rs`

**Ring-buffer cadence note.** The sampler pushes rate / latency sparkline points
only on DB-poll ticks (every 5 s). With `RING_DEPTH = 60`, the sparkline carries
60 × 5 s = 300 s = 5 minutes of history, matching the UI label.

```rust
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Ring buffer depth: 60 samples × 5 s cadence = 5 minutes.
pub const RING_DEPTH: usize = 60;

/// A (unix_ms, value) data point for sparklines.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DataPoint {
    pub ts: i64,
    pub v: f64,
}

/// In-memory telemetry state. Shared via Arc<RwLock<_>> between the sampler
/// task and the SSE handler. All fields written exclusively by the sampler.
#[derive(Debug, Default)]
pub struct TelemetryState {
    // Host resources
    pub cpu_pct: f32,
    pub mem_used_mib: u64,
    pub mem_total_mib: u64,

    // Ollama model state
    pub ollama_model: Option<String>,
    pub ollama_on_gpu: bool,
    pub ollama_vram_mib: Option<u64>,

    // Pipeline stage latency (p50 over last 60 compilations)
    pub latency_classify_p50_ms: Option<f64>,
    pub latency_retrieve_p50_ms: Option<f64>,
    pub latency_rank_p50_ms: Option<f64>,
    pub latency_compile_p50_ms: Option<f64>,
    pub latency_total_p50_ms: Option<f64>,

    // Live counters
    pub active_ingestions: i64,
    pub queue_depth: i64,
    pub failed_episodes: i64,

    // Sparkline ring buffers — pushed every 5 s (DB poll cadence)
    pub sparkline_latency: VecDeque<DataPoint>,
    pub sparkline_ingestion_rate: VecDeque<DataPoint>,
    pub sparkline_compilation_rate: VecDeque<DataPoint>,

    // Recent extraction errors (last 10)
    pub recent_errors: VecDeque<ExtractionError>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ExtractionError {
    pub episode_id: String,
    pub source: String,
    pub error: String,
    pub occurred_at: i64,   // unix_ms
}

impl TelemetryState {
    pub fn push_sparkline(ring: &mut VecDeque<DataPoint>, ts: i64, v: f64) {
        ring.push_back(DataPoint { ts, v });
        if ring.len() > RING_DEPTH {
            ring.pop_front();
        }
    }

    pub fn push_error(errors: &mut VecDeque<ExtractionError>, err: ExtractionError) {
        // De-duplicate by episode_id — the sampler re-queries the failed list
        // every 5 s, so the same episode would otherwise be pushed repeatedly.
        if errors.iter().any(|e| e.episode_id == err.episode_id) {
            return;
        }
        errors.push_back(err);
        if errors.len() > 10 {
            errors.pop_front();
        }
    }
}

pub type SharedTelemetry = Arc<RwLock<TelemetryState>>;

pub fn new_shared() -> SharedTelemetry {
    Arc::new(RwLock::new(TelemetryState::default()))
}
```

---

### 1.4 `src/telemetry/sampler.rs`

The sampler runs as a background tokio task. It samples host resources every
second and polls Ollama / the DB every 5 seconds.

**sysinfo CPU priming.** `sysinfo` needs two refreshes separated by
`MINIMUM_CPU_UPDATE_INTERVAL` (~200 ms) before CPU usage is meaningful — the
first reading is always 0.0. The sampler primes by calling `refresh_cpu_all()`
once before entering the loop and sleeping 250 ms. Subsequent per-tick refreshes
are correct.

```rust
use std::time::Duration;

use sysinfo::System;
use tokio_util::sync::CancellationToken;

use crate::db::pool::DbPools;
use crate::telemetry::state::{
    ExtractionError, SharedTelemetry, TelemetryState,
};

/// Ollama /api/ps response shape (only the fields we need).
#[derive(Debug, serde::Deserialize)]
struct OllamaPsResponse {
    models: Vec<OllamaModel>,
}

#[derive(Debug, serde::Deserialize)]
struct OllamaModel {
    name: String,
    #[serde(default)]
    size_vram: u64,
}

pub async fn run_sampler(
    telemetry: SharedTelemetry,
    pools: DbPools,
    ollama_base_url: String,
    cancel: CancellationToken,
) {
    let mut sys = System::new_all();
    // Prime the CPU sample — the first refresh always reports 0.0.
    sys.refresh_cpu_all();
    tokio::time::sleep(Duration::from_millis(250)).await;

    let http = reqwest::Client::new();
    let mut tick_count: u64 = 0;
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    let mut ollama_last_ok = true;
    let mut db_last_ok = true;

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!("telemetry sampler shutting down");
                break;
            }
            _ = interval.tick() => {}
        }

        tick_count += 1;
        let now_ms = chrono::Utc::now().timestamp_millis();

        // Host resources — every tick
        sys.refresh_cpu_all();
        sys.refresh_memory();
        let cpu_pct = sys.global_cpu_usage();
        let mem_used_mib = sys.used_memory() / 1024 / 1024;
        let mem_total_mib = sys.total_memory() / 1024 / 1024;

        // Ollama + DB — every 5 ticks
        let poll_slow_paths = tick_count % 5 == 0;

        let ollama_snapshot = if poll_slow_paths {
            match poll_ollama(&http, &ollama_base_url).await {
                Ok(snap) => {
                    if !ollama_last_ok {
                        tracing::info!("telemetry: ollama reachable again");
                        ollama_last_ok = true;
                    }
                    Some(snap)
                }
                Err(e) => {
                    if ollama_last_ok {
                        tracing::warn!(error = %e, "telemetry: ollama /api/ps poll failed");
                        ollama_last_ok = false;
                    }
                    None
                }
            }
        } else {
            None
        };

        let db_snapshot = if poll_slow_paths {
            match query_db_snapshot(&pools).await {
                Ok(snap) => {
                    if !db_last_ok {
                        tracing::info!("telemetry: db queries recovered");
                        db_last_ok = true;
                    }
                    Some(snap)
                }
                Err(e) => {
                    if db_last_ok {
                        tracing::warn!(error = %e, "telemetry: db snapshot query failed");
                        db_last_ok = false;
                    }
                    None
                }
            }
        } else {
            None
        };

        // Write to shared state
        let mut state = telemetry.write().await;

        state.cpu_pct = cpu_pct;
        state.mem_used_mib = mem_used_mib;
        state.mem_total_mib = mem_total_mib;

        if let Some(ollama) = ollama_snapshot {
            state.ollama_model = ollama.model;
            state.ollama_on_gpu = ollama.on_gpu;
            state.ollama_vram_mib = ollama.vram_mib;
        }

        if let Some(db) = db_snapshot {
            state.active_ingestions = db.active_ingestions;
            state.queue_depth = db.queue_depth;
            state.failed_episodes = db.failed_episodes;
            state.latency_classify_p50_ms = db.latency_classify_p50_ms;
            state.latency_retrieve_p50_ms = db.latency_retrieve_p50_ms;
            state.latency_rank_p50_ms = db.latency_rank_p50_ms;
            state.latency_compile_p50_ms = db.latency_compile_p50_ms;
            state.latency_total_p50_ms = db.latency_total_p50_ms;

            if let Some(lat) = db.latency_total_p50_ms {
                TelemetryState::push_sparkline(
                    &mut state.sparkline_latency, now_ms, lat,
                );
            }
            TelemetryState::push_sparkline(
                &mut state.sparkline_ingestion_rate,
                now_ms,
                db.ingestion_completions_last_min as f64,
            );
            TelemetryState::push_sparkline(
                &mut state.sparkline_compilation_rate,
                now_ms,
                db.compilation_requests_last_min as f64,
            );

            for err in db.recent_errors {
                TelemetryState::push_error(&mut state.recent_errors, err);
            }
        }
    }
}

// ── Ollama poll ──────────────────────────────────────────────────────────────

struct OllamaSnapshot {
    model: Option<String>,
    on_gpu: bool,
    vram_mib: Option<u64>,
}

async fn poll_ollama(http: &reqwest::Client, base_url: &str) -> Result<OllamaSnapshot, String> {
    let url = format!("{}/api/ps", base_url.trim_end_matches('/'));
    let resp = http
        .get(&url)
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .map_err(|e| format!("request: {e}"))?;

    if !resp.status().is_success() {
        return Ok(OllamaSnapshot { model: None, on_gpu: false, vram_mib: None });
    }

    let ps: OllamaPsResponse = resp.json().await.map_err(|e| format!("decode: {e}"))?;

    if ps.models.is_empty() {
        return Ok(OllamaSnapshot { model: None, on_gpu: false, vram_mib: None });
    }

    // Loom only loads one model at a time; take the first.
    let m = &ps.models[0];
    let on_gpu = m.size_vram > 0;
    let vram_mib = if on_gpu { Some(m.size_vram / 1024 / 1024) } else { None };

    Ok(OllamaSnapshot {
        model: Some(m.name.clone()),
        on_gpu,
        vram_mib,
    })
}

// ── DB snapshot ──────────────────────────────────────────────────────────────

struct DbSnapshot {
    active_ingestions: i64,
    queue_depth: i64,
    failed_episodes: i64,
    ingestion_completions_last_min: i64,
    compilation_requests_last_min: i64,
    latency_classify_p50_ms: Option<f64>,
    latency_retrieve_p50_ms: Option<f64>,
    latency_rank_p50_ms: Option<f64>,
    latency_compile_p50_ms: Option<f64>,
    latency_total_p50_ms: Option<f64>,
    recent_errors: Vec<ExtractionError>,
}

async fn query_db_snapshot(pools: &DbPools) -> Result<DbSnapshot, sqlx::Error> {
    let pool = &pools.online;

    // Episode state counts
    let (active, pending, failed): (i64, i64, i64) = sqlx::query_as(
        r#"
        SELECT
          COUNT(*) FILTER (WHERE processing_status = 'processing') AS active,
          COUNT(*) FILTER (WHERE processing_status = 'pending')    AS pending,
          COUNT(*) FILTER (WHERE processing_status = 'failed')     AS failed
        FROM loom_episodes
        WHERE deleted_at IS NULL
        "#,
    )
    .fetch_one(pool)
    .await?;

    // Completions in last 60 s
    let completions: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) FROM loom_episodes
        WHERE processing_status = 'completed'
          AND processing_last_attempt > NOW() - INTERVAL '60 seconds'
          AND deleted_at IS NULL
        "#,
    )
    .fetch_one(pool)
    .await?;

    // Compilation requests in last 60 s (loom_audit_log is compilation-only)
    let compilations: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM loom_audit_log WHERE created_at > NOW() - INTERVAL '60 seconds'",
    )
    .fetch_one(pool)
    .await?;

    // Pipeline stage latency p50 over last 60 compilations
    #[derive(sqlx::FromRow)]
    struct LatencyRow {
        p50_classify: Option<f64>,
        p50_retrieve: Option<f64>,
        p50_rank:     Option<f64>,
        p50_compile:  Option<f64>,
        p50_total:    Option<f64>,
    }

    let latency: Option<LatencyRow> = sqlx::query_as(
        r#"
        SELECT
          PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY latency_classify_ms) AS p50_classify,
          PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY latency_retrieve_ms) AS p50_retrieve,
          PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY latency_rank_ms)     AS p50_rank,
          PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY latency_compile_ms)  AS p50_compile,
          PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY latency_total_ms)    AS p50_total
        FROM (
          SELECT latency_classify_ms, latency_retrieve_ms, latency_rank_ms,
                 latency_compile_ms, latency_total_ms
          FROM loom_audit_log
          WHERE latency_total_ms IS NOT NULL
          ORDER BY created_at DESC
          LIMIT 60
        ) recent
        "#,
    )
    .fetch_optional(pool)
    .await?;

    // Recent failed episodes (last 10)
    #[derive(sqlx::FromRow)]
    struct ErrorRow {
        id: uuid::Uuid,
        source: String,
        processing_last_error: Option<String>,
        processing_last_attempt: Option<chrono::DateTime<chrono::Utc>>,
    }

    let error_rows: Vec<ErrorRow> = sqlx::query_as(
        r#"
        SELECT id, source, processing_last_error, processing_last_attempt
        FROM loom_episodes
        WHERE processing_status = 'failed' AND deleted_at IS NULL
        ORDER BY processing_last_attempt DESC NULLS LAST
        LIMIT 10
        "#,
    )
    .fetch_all(pool)
    .await?;

    let recent_errors = error_rows
        .into_iter()
        .map(|r| ExtractionError {
            episode_id: r.id.to_string(),
            source: r.source,
            error: r.processing_last_error.unwrap_or_else(|| "unknown".into()),
            occurred_at: r
                .processing_last_attempt
                .map(|t| t.timestamp_millis())
                .unwrap_or(0),
        })
        .collect();

    Ok(DbSnapshot {
        active_ingestions: active,
        queue_depth: pending,
        failed_episodes: failed,
        ingestion_completions_last_min: completions,
        compilation_requests_last_min: compilations,
        latency_classify_p50_ms: latency.as_ref().and_then(|l| l.p50_classify),
        latency_retrieve_p50_ms: latency.as_ref().and_then(|l| l.p50_retrieve),
        latency_rank_p50_ms:     latency.as_ref().and_then(|l| l.p50_rank),
        latency_compile_p50_ms:  latency.as_ref().and_then(|l| l.p50_compile),
        latency_total_p50_ms:    latency.as_ref().and_then(|l| l.p50_total),
        recent_errors,
    })
}
```

---

### 1.5 `src/api/dashboard.rs` additions

Add at the bottom of `dashboard.rs`, before the `#[cfg(test)]` block.

**Add imports** at the top:

```rust
use axum::response::sse::{Event, KeepAlive, Sse};
use std::convert::Infallible;
use tokio_stream::wrappers::IntervalStream;
use tokio_stream::StreamExt as _;
```

**Add type:**

```rust
/// Snapshot serialized as SSE data payload every second.
#[derive(Debug, Clone, Serialize)]
pub struct TelemetrySnapshot {
    pub ts: i64,
    // Host
    pub cpu_pct: f32,
    pub mem_used_mib: u64,
    pub mem_total_mib: u64,
    // Ollama
    pub ollama_model: Option<String>,
    pub ollama_on_gpu: bool,
    pub ollama_vram_mib: Option<u64>,
    // Pipeline stage latencies (p50, ms)
    pub latency_classify_p50_ms: Option<f64>,
    pub latency_retrieve_p50_ms: Option<f64>,
    pub latency_rank_p50_ms: Option<f64>,
    pub latency_compile_p50_ms: Option<f64>,
    pub latency_total_p50_ms: Option<f64>,
    // Live counters
    pub active_ingestions: i64,
    pub queue_depth: i64,
    pub failed_episodes: i64,
    // Sparklines
    pub sparkline_latency: Vec<crate::telemetry::state::DataPoint>,
    pub sparkline_ingestion_rate: Vec<crate::telemetry::state::DataPoint>,
    pub sparkline_compilation_rate: Vec<crate::telemetry::state::DataPoint>,
    // Error tail
    pub recent_errors: Vec<crate::telemetry::state::ExtractionError>,
}
```

**Add handler:**

```rust
// ---------------------------------------------------------------------------
// GET /dashboard/api/stream/telemetry
// ---------------------------------------------------------------------------

/// Server-Sent Events stream delivering a TelemetrySnapshot every second.
/// Clients connect once; the connection stays open. KeepAlive pings prevent
/// proxy timeouts on inactive seconds.
pub async fn handle_telemetry_stream(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let telemetry = state.telemetry.clone();
    let interval = tokio::time::interval(std::time::Duration::from_secs(1));

    let stream = IntervalStream::new(interval).map(move |_| {
        // try_read avoids blocking the SSE frame on a concurrent write from
        // the sampler. A contended read yields an SSE comment and the client
        // receives the next frame 1 s later.
        let event = match telemetry.try_read() {
            Ok(s) => {
                let snapshot = TelemetrySnapshot {
                    ts: chrono::Utc::now().timestamp_millis(),
                    cpu_pct: s.cpu_pct,
                    mem_used_mib: s.mem_used_mib,
                    mem_total_mib: s.mem_total_mib,
                    ollama_model: s.ollama_model.clone(),
                    ollama_on_gpu: s.ollama_on_gpu,
                    ollama_vram_mib: s.ollama_vram_mib,
                    latency_classify_p50_ms: s.latency_classify_p50_ms,
                    latency_retrieve_p50_ms: s.latency_retrieve_p50_ms,
                    latency_rank_p50_ms: s.latency_rank_p50_ms,
                    latency_compile_p50_ms: s.latency_compile_p50_ms,
                    latency_total_p50_ms: s.latency_total_p50_ms,
                    active_ingestions: s.active_ingestions,
                    queue_depth: s.queue_depth,
                    failed_episodes: s.failed_episodes,
                    sparkline_latency: s.sparkline_latency.iter().cloned().collect(),
                    sparkline_ingestion_rate: s.sparkline_ingestion_rate.iter().cloned().collect(),
                    sparkline_compilation_rate: s.sparkline_compilation_rate.iter().cloned().collect(),
                    recent_errors: s.recent_errors.iter().cloned().collect(),
                };
                match serde_json::to_string(&snapshot) {
                    Ok(data) => Event::default().data(data),
                    Err(_) => Event::default().comment("encode-error"),
                }
            }
            Err(_) => Event::default().comment("busy"),
        };
        Ok(event)
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}
```

---

### 1.6 `AppState` — add telemetry field

In `src/api/mcp.rs`, add:

```rust
use crate::telemetry::state::SharedTelemetry;

#[derive(Clone)]
pub struct AppState {
    pub pools: DbPools,
    pub llm_client: LlmClient,
    pub config: AppConfig,
    pub telemetry: SharedTelemetry, // NEW
}
```

Update the single construction site in `main.rs` (see 1.7).

---

### 1.7 `src/main.rs` changes

```rust
mod telemetry;

// … existing setup …

let telemetry = telemetry::state::new_shared();

let state = AppState {
    pools: pools.clone(),
    llm_client,
    config: config.clone(),
    telemetry: telemetry.clone(),
};
```

Route registration (inside the `dashboard_routes` builder — it inherits
the bearer-token middleware):

```rust
.route(
    "/dashboard/api/stream/telemetry",
    get(dashboard::handle_telemetry_stream),
)
```

Sampler spawn (after `let cancel_token = CancellationToken::new();`):

```rust
let _sampler_handle = tokio::spawn(telemetry::sampler::run_sampler(
    telemetry.clone(),
    state.pools.clone(),
    state.config.llm.ollama_url.clone(),
    cancel_token.clone(),
));
```

---

## Phase 2: React Dashboard

### 2.1 No new npm dependencies

`RuntimePage` uses inline SVG for sparklines. No recharts, no D3.

### 2.2 TypeScript type — `src/types/index.ts`

```typescript
export interface DataPoint {
  ts: number;   // unix_ms
  v: number;
}

export interface ExtractionError {
  episode_id: string;
  source: string;
  error: string;
  occurred_at: number; // unix_ms
}

export interface TelemetrySnapshot {
  ts: number;
  cpu_pct: number;
  mem_used_mib: number;
  mem_total_mib: number;
  ollama_model: string | null;
  ollama_on_gpu: boolean;
  ollama_vram_mib: number | null;
  latency_classify_p50_ms: number | null;
  latency_retrieve_p50_ms: number | null;
  latency_rank_p50_ms: number | null;
  latency_compile_p50_ms: number | null;
  latency_total_p50_ms: number | null;
  active_ingestions: number;
  queue_depth: number;
  failed_episodes: number;
  sparkline_latency: DataPoint[];
  sparkline_ingestion_rate: DataPoint[];
  sparkline_compilation_rate: DataPoint[];
  recent_errors: ExtractionError[];
}
```

### 2.3 SSE hook — `src/hooks/useTelemetryStream.ts`

No token handling — Caddy injects `Authorization` on the proxy hop.

```typescript
import { useEffect, useState } from 'react';
import type { TelemetrySnapshot } from '../types';

const SSE_URL = '/dashboard/api/stream/telemetry';

export function useTelemetryStream(): TelemetrySnapshot | null {
  const [snapshot, setSnapshot] = useState<TelemetrySnapshot | null>(null);

  useEffect(() => {
    const es = new EventSource(SSE_URL);

    es.onmessage = (e: MessageEvent) => {
      try {
        setSnapshot(JSON.parse(e.data) as TelemetrySnapshot);
      } catch {
        // Ignore malformed frames (sampler initializing, comment pings).
      }
    };

    // EventSource auto-reconnects on error — no manual retry needed.
    return () => es.close();
  }, []);

  return snapshot;
}
```

### 2.4 `src/pages/RuntimePage.tsx`

**Design rules.** btop-inspired: dense, monospaced labels, sparse padding.
Three rows: System / Pipeline / Errors. Inline SVG sparklines, no axes.

**Use the design system tokens** ([design-system.css](loom-dashboard/src/design-system.css)):
`--signal-success`, `--signal-warning`, `--signal-error`, `--surface-card`,
`--surface-sunken`, `--border-1`, `--fg-1`, `--fg-2`, `--fg-muted`.

**Threshold constants (hardcoded):**

| Metric | Yellow | Red |
|--------|--------|-----|
| CPU% | ≥ 70 | ≥ 90 |
| Mem% | ≥ 75 | ≥ 90 |
| Total p50 latency (ms) | ≥ 500 | ≥ 1000 |
| Queue depth | ≥ 100 | ≥ 500 |
| Failed episodes | ≥ 1 | ≥ 10 |

See the implementation for the component shape — helpers (`cpuHealth`,
`memHealth`, `latencyHealth`, `queueHealth`, `failedHealth`), a `Sparkline`
that draws a `<polygon>` area under a `<polyline>`, a `BarGauge`, a
`StageLatency` row, a `Card` wrapper, and the page composing three grid rows.

### 2.5 `src/App.tsx` wiring

Import:

```tsx
import { RuntimePage } from './pages/RuntimePage';
```

Add under the `Overview` nav section (after the Pipeline Health link):

```tsx
<NavLink to="/runtime">Runtime</NavLink>
```

Add the route inside `<Routes>`:

```tsx
<Route path="/runtime" element={<RuntimePage />} />
```

---

## Phase 3: Verification

1. `cd loom-engine && cargo build` — sysinfo / async-stream / tokio-stream resolved
2. `cargo clippy -- -D warnings` — no new warnings
3. `cargo nextest run` — existing tests pass + any new ones
4. `docker compose up -d` — engine + dashboard + Caddy up
5. Navigate to `/runtime` on the dashboard
6. Browser DevTools → Network → EventStream: confirm snapshots at ~1 s cadence
7. CPU / mem gauges animate; Ollama card shows model + GPU/CPU badge when loaded
8. Ingest an episode; `active` or `pending` counter moves, then resolves
9. After 5 min of traffic, sparklines show ~60 data points
10. `cd loom-dashboard && npm test && npx biome check src/` — all green

---

## Deferred

| Capability | Trigger to Add |
|-----------|----------------|
| Configurable thresholds in the dashboard UI | After thresholds are wrong in production more than twice |
| Per-namespace ingestion-rate breakdown | After multi-namespace use is common |
| GPU temperature / fan speed | After NVML is confirmed stable on the local machine |
| Ollama total VRAM display | After Ollama exposes it in `/api/ps` |
| Alert notifications (email / webhook) | After dashboard-observed failure is insufficient |
| Historical sparkline persistence across restarts | After ring-buffer loss proves operationally painful |
| `loom_audit_log(created_at)` index | After the 5 s DB snapshot query exceeds 100 ms in practice |

---

## ADR Reference

See [docs/adr/010-streaming-telemetry.md](docs/adr/010-streaming-telemetry.md).
