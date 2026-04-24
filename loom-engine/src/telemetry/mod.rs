//! Runtime telemetry — in-process sampling and SSE-streamed snapshots.
//!
//! The sampler task runs every second, reads host resources (CPU / memory)
//! via `sysinfo`, polls Ollama `/api/ps` every 5 seconds for model state, and
//! queries `loom_audit_log` + `loom_episodes` every 5 seconds for pipeline
//! counters and stage-latency percentiles. Results are written to a shared
//! `TelemetryState` which the SSE handler in `api::dashboard` serializes once
//! per second.
//!
//! See `docs/adr/010-streaming-telemetry.md` for the design rationale.

pub mod sampler;
pub mod state;

pub use state::{new_shared, SharedTelemetry};
