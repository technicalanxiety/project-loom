//! Classification types for intent detection.
//!
//! The online pipeline classifies each query into one of five task classes
//! to select appropriate retrieval profiles and memory weight modifiers.

use serde::{Deserialize, Serialize};

/// The five task classes for intent classification.
///
/// Maps to the `task_class` column in `loom_audit_log`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskClass {
    /// Debugging and troubleshooting queries.
    Debug,
    /// System design and architecture queries.
    Architecture,
    /// Compliance, audit, and governance queries.
    Compliance,
    /// Documentation and writing queries.
    Writing,
    /// General conversation (default fallback).
    Chat,
}

impl std::fmt::Display for TaskClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Debug => "debug",
            Self::Architecture => "architecture",
            Self::Compliance => "compliance",
            Self::Writing => "writing",
            Self::Chat => "chat",
        };
        write!(f, "{s}")
    }
}

impl std::str::FromStr for TaskClass {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "debug" => Ok(Self::Debug),
            "architecture" => Ok(Self::Architecture),
            "compliance" => Ok(Self::Compliance),
            "writing" => Ok(Self::Writing),
            "chat" => Ok(Self::Chat),
            other => Err(format!("unknown task class: {other}")),
        }
    }
}

/// Result of intent classification for a query.
///
/// Contains primary and optional secondary class with confidence scores.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassificationResult {
    /// Primary task class.
    pub primary_class: TaskClass,
    /// Secondary task class (present when confidence gap < 0.3).
    pub secondary_class: Option<TaskClass>,
    /// Confidence score for the primary class.
    pub primary_confidence: f64,
    /// Confidence score for the secondary class.
    pub secondary_confidence: Option<f64>,
}
