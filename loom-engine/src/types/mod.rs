//! Shared type definitions for the loom-engine.
//!
//! All types that cross module boundaries are defined here with serde
//! serialization. Database row types derive `sqlx::FromRow`.

pub mod audit;
pub mod classification;
pub mod compilation;
pub mod entity;
pub mod episode;
pub mod fact;
pub mod mcp;
pub mod predicate;
