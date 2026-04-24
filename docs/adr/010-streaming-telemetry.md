# ADR-010: Streaming Telemetry via SSE + In-Process Ring Buffers

## Status

Accepted

## Context

The dashboard is entirely poll-based REST. Every view does a GET on demand; no
data is pushed to the client. This is correct for all the existing views —
entity graphs, compilation traces, and extraction metrics are not time-sensitive.

The runtime view is different. The operator needs to see what is happening right
now: CPU and memory under extraction load, whether Ollama has a model loaded and
on which compute, the latency of each pipeline stage, and whether the ingestion
queue is draining or growing. Poll-based REST at any reasonable interval (5–10 s)
is too coarse. A 1 s poll loop from the React client produces 1 request/second
indefinitely from every open browser tab, which is wasteful.

The question is how to push a live snapshot from the engine to the dashboard at
1-second cadence with minimal complexity.

Two sub-questions arise:
1. What collects host resource data (CPU, memory)?
2. How is GPU/model state obtained given Loom does not own Ollama's process?

(Browser auth for `EventSource` — notably, `EventSource` cannot set a custom
`Authorization` header — is **not** a concern for Loom because Caddy's
`/dashboard/api/*` handler already injects `Authorization: Bearer <token>` on
the proxy hop. SSE rides the same path as every other dashboard endpoint.)

## Decision

### Transport: Server-Sent Events (SSE), not WebSockets

SSE is unidirectional server-to-client push over a single persistent HTTP
connection. The telemetry feed is entirely one-directional: the engine pushes
snapshots, the browser renders them. WebSockets are bidirectional and carry
the overhead of a custom framing protocol. SSE is appropriate and axum 0.8
supports it natively via `axum::response::sse`. The only additional crate for
the transport layer is `tokio-stream` for the `IntervalStream` wrapper.

### Data source: in-process ring buffers, not a time-series database

Persisting telemetry to PostgreSQL or a separate time-series store (InfluxDB,
Prometheus) would be tool sprawl. The operational question is "what is happening
right now and in the last 5 minutes," not "what happened last Tuesday."
`RING_DEPTH = 60` points pushed every 5 s give exactly 5 minutes of sparkline
history. They are correct for the use case, free at rest (no storage writes),
and reset on restart — which is acceptable because a restart is itself an
observable event that resets all counters.

Pipeline stage latency, queue counters, and the failed-episode tail are queried
from PostgreSQL by the sampler task every 5 seconds. This re-uses the existing
`loom_audit_log` and `loom_episodes` tables with no schema changes. The DB query
is cheap: a `PERCENTILE_CONT` over the most recent 60 rows plus a pair of count
queries with a 60-second window filter.

### Host resource sampling: `sysinfo` crate

`sysinfo` is cross-platform, requires no native daemon and no elevated
privileges, and has ~2 ms overhead per sample. It reads `/proc/stat` and
`/proc/meminfo` on Linux — the same sources btop uses. Alternatives considered:

- `procfs` — Linux-only, excluded because the engine runs on macOS in dev
- `nvml-wrapper` — NVIDIA management library; requires the proprietary driver,
  breaks on non-NVIDIA machines including the current iGPU host
- Reading `/proc` directly — re-implementing sysinfo for no gain

`sysinfo` is the correct choice. One quirk worth noting: the first CPU refresh
always reads 0.0 % — two refreshes separated by ~200 ms are required before the
percentage is meaningful. The sampler primes at startup before entering its loop.

### GPU / model state: Ollama `/api/ps`

Ollama exposes `/api/ps`, which returns currently loaded models, their size, and
`size_vram` (the portion of the model stored in VRAM). A non-zero `size_vram`
means the model is on GPU. This is sufficient for operational awareness — the
operator can see whether Ollama has loaded a model and whether it is on GPU or
CPU. Total VRAM is not currently exposed by `/api/ps`; it is deferred.

The sampler polls `/api/ps` every 5 seconds with a 2-second timeout. If Ollama
is unreachable (not running, still loading), the poll returns `Err` and the
dashboard shows "no model loaded." This is correct behavior.

NVML is explicitly rejected. It requires `libnvidia-ml.so` at link time, fails
on macOS, fails on non-NVIDIA Linux, and adds an FFI safety surface. The
operational benefit over Ollama's own API does not justify the portability cost.

### Auth: no change — Caddy handles it

The W3C `EventSource` API does not support custom headers. This is normally a
problem for bearer-token-authenticated SSE, but the Loom dashboard never carries
a token client-side. Caddy's `/dashboard/api/*` handler injects
`Authorization: Bearer {env.LOOM_BEARER_TOKEN}` via `header_up` on every
proxied request ([Caddyfile](../../Caddyfile)). The browser-issued
`EventSource` connection traverses the same proxy, so Caddy adds the header
transparently. The engine's existing `require_bearer_token` middleware
validates the injected header. No middleware change, no query-param fallback,
and no `localStorage` token handling is needed.

In the Vite dev proxy path, the engine receives no `Authorization` header and
the existing REST dashboard views already rely on that; SSE works identically.

### Sampler task lifecycle

The sampler is a `tokio::spawn`ed task that reads the same `CancellationToken`
used by the existing processor and scheduler workers. It shuts down cleanly on
Ctrl-C alongside those workers. The `SharedTelemetry` (`Arc<RwLock<_>>`) is
initialized in `main` and cloned into `AppState` so the SSE handler can read it
without a channel or additional synchronization.

The handler uses `try_read()` rather than `read().await` so a concurrent write
from the sampler (even brief) cannot delay an SSE frame. A contended read emits
an SSE comment (`: busy`) and the client ignores it; the next frame arrives
1 second later. The write itself is always short — the sampler does DB I/O and
Ollama I/O *before* acquiring the lock, then a synchronous copy into shared
state — but the contended-read path exists for belt-and-braces resilience.

### Visualization: inline SVG sparklines, no charting library

The existing `package.json` has no charting dependency (React 19 + Vite only).
Adding recharts or D3 for three sparklines would be technical gluttony.
Sparklines are straightforward with inline `<polyline>` / `<polygon>` SVG —
the implementation is roughly 30 lines with no layout or animation complexity.
The btop aesthetic (dense, monospaced, color-coded bars) is achievable with
plain HTML and the existing design-system CSS custom properties
(`--signal-*`, `--surface-*`, `--border-*`, `--fg-*`).

## Consequences

### Positive

- Live 1-second cadence without per-second polling from the client
- No new services, no Prometheus, no external monitoring stack
- Schema unchanged — no new migrations
- CPU / memory sampling works on macOS (dev) and Linux (prod) without changes
- Graceful degradation: if Ollama is unreachable, the page still works with
  host-resource data
- SSE auto-reconnects on disconnect; the client hook does not need retry logic
- Auth works without any new code — reuses the existing Caddy `header_up`
  injection

### Negative

- 5 minutes of sparkline history is lost on engine restart — acceptable for
  local single-operator infrastructure, not acceptable for a shared deployment
- One additional DB connection slot is consumed by the sampler's 5-second
  queries — negligible given the existing pool configuration
- Every SSE frame serializes the full ring (~60 points × 3 sparklines × 2
  scalars) rather than a delta. At ~3 KB per frame this is fine for a single
  operator, but it would be wasteful at scale

### Neutral

- Three new Rust crates (`sysinfo`, `async-stream`, `tokio-stream`) — all
  well-maintained, no transitive complexity
- The `RuntimePage` component has no unit tests — consistent with other
  dashboard pages, which also have no component-level tests for
  data-display-only views. Unit tests exist on the Rust side for
  `push_sparkline` / `push_error` cap behavior.
