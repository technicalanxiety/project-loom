//! loom-engine library crate.
//!
//! Re-exports public modules for integration tests and external consumers.
//! The binary entry point is in `main.rs`.

pub mod api;
pub mod config;
pub mod db;
pub mod llm;
pub mod pipeline;
pub mod types;
pub mod worker;

use std::sync::Once;

/// Install `ring` as the default rustls crypto provider exactly once.
///
/// Needed because `reqwest` is compiled with `rustls-no-provider` (so we can
/// avoid pulling `aws-lc-sys`, which cannot cross-compile to musl). Without a
/// provider installed, any reqwest TLS handshake — including the ones in the
/// LLM client test suite — panics with `No provider set`.
///
/// Idempotent: safe to call from `main` at startup, from `LlmClient::new`,
/// and from test setup. `Once` guarantees a single install attempt; if a
/// provider is already installed (e.g. the binary already set it before a
/// test harness runs), `install_default()` returns `Err` which we discard.
pub fn ensure_crypto_provider() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}
