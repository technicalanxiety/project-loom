//! Graph traversal queries via the `loom_traverse` SQL function.
//!
//! Wraps the recursive CTE-based traversal function that performs 1-2 hop
//! neighborhood exploration with cycle prevention. All functions accept a
//! `&PgPool` reference and return `Result<T, TraverseError>`.

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

/// Errors that can occur during graph traversal operations.
#[derive(Debug, thiserror::Error)]
pub enum TraverseError {
    /// An underlying sqlx / PostgreSQL error.
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
}

/// A single result row from the `loom_traverse` SQL function.
///
/// Represents a discovered entity and the connecting fact at a given
/// hop depth in the traversal.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TraversalResult {
    /// The discovered entity's identifier.
    pub entity_id: Uuid,
    /// The discovered entity's name.
    pub entity_name: String,
    /// The discovered entity's type.
    pub entity_type: String,
    /// The connecting fact's identifier.
    pub fact_id: Option<Uuid>,
    /// The connecting fact's predicate.
    pub predicate: Option<String>,
    /// The connecting fact's evidence status.
    pub evidence_status: Option<String>,
    /// How many hops from the starting entity.
    pub hop_depth: i32,
    /// The traversal path (entity UUIDs visited).
    pub path: Vec<Uuid>,
}

// ---------------------------------------------------------------------------
// Traversal
// ---------------------------------------------------------------------------

/// Traverse the knowledge graph from a starting entity.
///
/// Calls the `loom_traverse` SQL function which performs a recursive CTE
/// traversal with cycle prevention. Returns entities and connecting facts
/// discovered within `max_hops` of the starting entity, scoped to the
/// given namespace.
pub async fn traverse(
    pool: &PgPool,
    entity_id: Uuid,
    max_hops: i32,
    namespace: &str,
) -> Result<Vec<TraversalResult>, TraverseError> {
    let rows = sqlx::query_as::<_, TraversalResult>(
        "SELECT * FROM loom_traverse($1, $2, $3)",
    )
    .bind(entity_id)
    .bind(max_hops)
    .bind(namespace)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traverse_error_displays_message() {
        let err = TraverseError::Sqlx(sqlx::Error::RowNotFound);
        assert!(err.to_string().contains("database error"));
    }
}
