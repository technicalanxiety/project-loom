//! loom-engine library crate.
//!
//! Re-exports public modules for integration tests and external consumers.
//! The binary entry point is in `main.rs`.

pub mod api;
pub mod config;
pub mod crypto;
pub mod db;
pub mod llm;
pub mod pipeline;
pub mod types;
pub mod worker;

// Re-export at crate root for call-site ergonomics.
pub use crypto::ensure_crypto_provider;
