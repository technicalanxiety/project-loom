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
