//! rustls crypto provider installation.
//!
//! `reqwest` is compiled with `rustls-no-provider` (to avoid `aws-lc-sys`,
//! which cannot cross-compile to musl). Without a provider installed, any
//! reqwest TLS handshake — including the ones in the LLM-client test suite
//! — panics with `No provider set`.
//!
//! This helper lives in its own module so both `main.rs` (binary) and
//! `lib.rs` (library) can declare it and resolve `crate::crypto::...`
//! regardless of which crate root is being compiled. That's necessary
//! because `main.rs` redeclares `mod llm;` etc., giving the binary its own
//! module tree distinct from the library's.

use std::sync::Once;

/// Install `ring` as the default rustls crypto provider exactly once.
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
