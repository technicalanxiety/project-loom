//! Shared telemetry state — written by the sampler, read by the SSE handler.

use std::collections::VecDeque;
use std::sync::Arc;

use tokio::sync::RwLock;

/// Ring buffer depth: 60 samples × 5 s cadence = 5 minutes of sparkline history.
pub const RING_DEPTH: usize = 60;

/// Maximum recent extraction errors surfaced to the dashboard.
pub const MAX_RECENT_ERRORS: usize = 10;

/// A (unix_ms, value) data point for sparklines.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DataPoint {
    pub ts: i64,
    pub v: f64,
}

/// A recent extraction failure, surfaced on the runtime page's error tail.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExtractionError {
    pub episode_id: String,
    pub source: String,
    pub error: String,
    pub occurred_at: i64, // unix_ms
}

/// In-memory telemetry state. Shared via `Arc<RwLock<_>>` between the sampler
/// task and the SSE handler. All fields are written exclusively by the sampler.
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

    // Recent failed episodes, newest appended last.
    pub recent_errors: VecDeque<ExtractionError>,
}

impl TelemetryState {
    /// Append a sparkline data point, evicting the oldest if the ring is full.
    pub fn push_sparkline(ring: &mut VecDeque<DataPoint>, ts: i64, v: f64) {
        ring.push_back(DataPoint { ts, v });
        if ring.len() > RING_DEPTH {
            ring.pop_front();
        }
    }

    /// Append an error, de-duplicating by `episode_id`. The sampler re-queries
    /// the failed list every 5 s, so without this check the same episode would
    /// be pushed repeatedly until it rotated out of the DB's top-10.
    pub fn push_error(errors: &mut VecDeque<ExtractionError>, err: ExtractionError) {
        if errors.iter().any(|e| e.episode_id == err.episode_id) {
            return;
        }
        errors.push_back(err);
        if errors.len() > MAX_RECENT_ERRORS {
            errors.pop_front();
        }
    }
}

/// Shared handle — cloned into `AppState` and the sampler task.
pub type SharedTelemetry = Arc<RwLock<TelemetryState>>;

/// Build a fresh shared telemetry state.
pub fn new_shared() -> SharedTelemetry {
    Arc::new(RwLock::new(TelemetryState::default()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparkline_ring_caps_at_depth() {
        let mut ring = VecDeque::new();
        for i in 0..(RING_DEPTH + 10) {
            TelemetryState::push_sparkline(&mut ring, i as i64, i as f64);
        }
        assert_eq!(ring.len(), RING_DEPTH);
        // Oldest entries evicted first — front should hold index 10.
        assert_eq!(ring.front().unwrap().v, 10.0);
        assert_eq!(ring.back().unwrap().v, (RING_DEPTH + 9) as f64);
    }

    #[test]
    fn push_error_caps_at_ten() {
        let mut errors = VecDeque::new();
        for i in 0..(MAX_RECENT_ERRORS + 5) {
            TelemetryState::push_error(
                &mut errors,
                ExtractionError {
                    episode_id: format!("ep-{i}"),
                    source: "test".into(),
                    error: "boom".into(),
                    occurred_at: i as i64,
                },
            );
        }
        assert_eq!(errors.len(), MAX_RECENT_ERRORS);
        // Oldest errors evicted first.
        assert_eq!(errors.front().unwrap().episode_id, "ep-5");
    }

    #[test]
    fn push_error_deduplicates_by_episode_id() {
        let mut errors = VecDeque::new();
        let make = |id: &str| ExtractionError {
            episode_id: id.into(),
            source: "test".into(),
            error: "boom".into(),
            occurred_at: 0,
        };
        TelemetryState::push_error(&mut errors, make("ep-1"));
        TelemetryState::push_error(&mut errors, make("ep-1"));
        TelemetryState::push_error(&mut errors, make("ep-2"));
        assert_eq!(errors.len(), 2);
    }
}
