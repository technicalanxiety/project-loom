//! Ingestion-mode taxonomy for episode provenance.
//!
//! Every episode enters through exactly one of three modes. The mode is
//! load-bearing for the authority hierarchy (Episodes > Facts > Procedures):
//! ranking reads it via a provenance coefficient, compilation reads it via
//! the sole-source flag, the dashboard reports on it.
//!
//! There is no `llm_reconstruction` mode. That path is rejected
//! architecturally — see `docs/adr/004-ingestion-modes.md` and
//! `docs/adr/005-verbatim-content-invariant.md`.

use serde::{Deserialize, Serialize};

/// Provenance class for an episode's ingestion path.
///
/// Serialized as snake_case to match the database CHECK constraint values
/// introduced in migration 015.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngestionMode {
    /// User-authored markdown prose ingested via the CLI seed tool or a
    /// direct REST call. The user is the author; an LLM may assist with
    /// drafting but does not author the content.
    UserAuthoredSeed,
    /// Parsed content from a vendor export (Claude.ai, Claude Code JSONL,
    /// ChatGPT, Codex CLI rollouts). Requires `parser_version` and
    /// `parser_source_schema` on the episode row.
    VendorImport,
    /// Real-time verbatim capture via an MCP-aware client. Hardcoded by the
    /// MCP server boundary on every `loom_learn` request; clients cannot
    /// override this value.
    LiveMcpCapture,
}

impl IngestionMode {
    /// The canonical snake_case string form used in the database and on the wire.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::UserAuthoredSeed => "user_authored_seed",
            Self::VendorImport => "vendor_import",
            Self::LiveMcpCapture => "live_mcp_capture",
        }
    }

    /// Provenance coefficient used by Stage 5 ranking.
    ///
    /// Live capture is ground truth (1.0). User-authored seed is authoritative
    /// but unverified against live observation (0.8). Vendor import is
    /// acknowledged as potentially incomplete per export fidelity (0.6).
    pub const fn provenance_coefficient(&self) -> f64 {
        match self {
            Self::LiveMcpCapture => 1.0,
            Self::UserAuthoredSeed => 0.8,
            Self::VendorImport => 0.6,
        }
    }
}

impl std::fmt::Display for IngestionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for IngestionMode {
    type Err = ParseIngestionModeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user_authored_seed" => Ok(Self::UserAuthoredSeed),
            "vendor_import" => Ok(Self::VendorImport),
            "live_mcp_capture" => Ok(Self::LiveMcpCapture),
            other => Err(ParseIngestionModeError(other.to_string())),
        }
    }
}

/// Error type for [`IngestionMode::from_str`].
#[derive(Debug, Clone, thiserror::Error)]
#[error("invalid ingestion_mode: {0}")]
pub struct ParseIngestionModeError(pub String);

/// Validate the parser-metadata / ingestion-mode coupling.
///
/// `VendorImport` requires non-empty `parser_version` and
/// `parser_source_schema`. The other two modes must omit both. Mirrors the
/// `chk_parser_fields_vendor_import` database CHECK constraint added in
/// migration 015; callers should prefer this function so the API produces
/// a structured error rather than a raw Postgres violation.
///
/// Returns `Ok(())` if the combination is valid, or `Err(msg)` with a
/// human-readable explanation otherwise.
pub fn validate_parser_fields(
    mode: IngestionMode,
    parser_version: Option<&str>,
    parser_source_schema: Option<&str>,
) -> Result<(), String> {
    let has_version = parser_version.map(|s| !s.trim().is_empty()).unwrap_or(false);
    let has_schema = parser_source_schema
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);

    match mode {
        IngestionMode::VendorImport => {
            if !has_version || !has_schema {
                return Err(
                    "ingestion_mode=vendor_import requires non-empty parser_version \
                     and parser_source_schema"
                        .into(),
                );
            }
        }
        IngestionMode::UserAuthoredSeed | IngestionMode::LiveMcpCapture => {
            if has_version || has_schema {
                return Err(format!(
                    "ingestion_mode={mode} must not include parser_version \
                     or parser_source_schema (those fields are reserved for vendor_import)"
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn coefficient_ordering_matches_amendment() {
        assert!(
            IngestionMode::LiveMcpCapture.provenance_coefficient()
                > IngestionMode::UserAuthoredSeed.provenance_coefficient()
        );
        assert!(
            IngestionMode::UserAuthoredSeed.provenance_coefficient()
                > IngestionMode::VendorImport.provenance_coefficient()
        );
    }

    #[test]
    fn roundtrips_through_string() {
        for mode in [
            IngestionMode::UserAuthoredSeed,
            IngestionMode::VendorImport,
            IngestionMode::LiveMcpCapture,
        ] {
            let s = mode.as_str();
            let parsed = IngestionMode::from_str(s).expect("valid");
            assert_eq!(parsed, mode);
        }
    }

    #[test]
    fn rejects_llm_reconstruction() {
        assert!(IngestionMode::from_str("llm_reconstruction").is_err());
    }

    #[test]
    fn serde_uses_snake_case() {
        let json = serde_json::to_string(&IngestionMode::LiveMcpCapture).unwrap();
        assert_eq!(json, r#""live_mcp_capture""#);
    }

    #[test]
    fn validate_parser_fields_requires_both_for_vendor_import() {
        assert!(
            validate_parser_fields(IngestionMode::VendorImport, Some("p@1"), Some("schema_v1")).is_ok()
        );
        assert!(
            validate_parser_fields(IngestionMode::VendorImport, Some("p@1"), None).is_err()
        );
        assert!(
            validate_parser_fields(IngestionMode::VendorImport, None, Some("schema_v1")).is_err()
        );
        assert!(validate_parser_fields(IngestionMode::VendorImport, None, None).is_err());
    }

    #[test]
    fn validate_parser_fields_forbids_them_for_other_modes() {
        assert!(
            validate_parser_fields(IngestionMode::UserAuthoredSeed, None, None).is_ok()
        );
        assert!(
            validate_parser_fields(IngestionMode::LiveMcpCapture, None, None).is_ok()
        );
        assert!(
            validate_parser_fields(IngestionMode::UserAuthoredSeed, Some("p@1"), None).is_err()
        );
        assert!(
            validate_parser_fields(IngestionMode::LiveMcpCapture, None, Some("schema_v1")).is_err()
        );
    }

    #[test]
    fn validate_parser_fields_treats_empty_strings_as_absent() {
        assert!(
            validate_parser_fields(IngestionMode::VendorImport, Some(""), Some("schema_v1")).is_err()
        );
        assert!(
            validate_parser_fields(IngestionMode::UserAuthoredSeed, Some("   "), None).is_ok()
        );
    }
}
