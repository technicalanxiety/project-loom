//! Database queries for the `loom_predicates`, `loom_predicate_packs`, and
//! `loom_predicate_candidates` tables.
//!
//! Provides pack-aware predicate lookups, usage tracking, candidate lifecycle
//! management, and operator resolution workflows. All functions accept a
//! `&PgPool` reference and return `Result<T, PredicateError>`.

use sqlx::PgPool;
use uuid::Uuid;

use crate::types::predicate::{PredicateCandidate, PredicateEntry, PredicatePack};

/// Errors that can occur during predicate database operations.
#[derive(Debug, thiserror::Error)]
pub enum PredicateError {
    /// An underlying sqlx / PostgreSQL error.
    #[error("database error: {0}")]
    Sqlx(#[from] sqlx::Error),
}

// ---------------------------------------------------------------------------
// Pack queries
// ---------------------------------------------------------------------------

/// List all predicate packs.
///
/// Returns every row from `loom_predicate_packs` ordered by pack name.
pub async fn get_all_packs(pool: &PgPool) -> Result<Vec<PredicatePack>, PredicateError> {
    let rows = sqlx::query_as::<_, PredicatePack>(
        "SELECT * FROM loom_predicate_packs ORDER BY pack",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Get all predicates belonging to a single pack.
///
/// Returns predicates ordered by category then predicate name.
pub async fn get_pack_predicates(
    pool: &PgPool,
    pack: &str,
) -> Result<Vec<PredicateEntry>, PredicateError> {
    let rows = sqlx::query_as::<_, PredicateEntry>(
        r#"
        SELECT *
        FROM loom_predicates
        WHERE pack = $1
        ORDER BY category, predicate
        "#,
    )
    .bind(pack)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// Query all predicates belonging to any of the given packs.
///
/// Used for pack-aware prompt assembly during fact extraction. The core
/// pack should always be included in the input slice.
pub async fn query_predicates_by_pack(
    pool: &PgPool,
    packs: &[String],
) -> Result<Vec<PredicateEntry>, PredicateError> {
    let rows = sqlx::query_as::<_, PredicateEntry>(
        r#"
        SELECT *
        FROM loom_predicates
        WHERE pack = ANY($1)
        ORDER BY pack, category, predicate
        "#,
    )
    .bind(packs)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

// ---------------------------------------------------------------------------
// Canonical predicate lookup
// ---------------------------------------------------------------------------

/// Check whether a predicate exists in the canonical registry.
///
/// Returns `Some(PredicateEntry)` if the predicate is canonical, `None`
/// otherwise. Used during predicate validation to distinguish canonical
/// predicates from custom ones.
pub async fn find_canonical_predicate(
    pool: &PgPool,
    predicate: &str,
) -> Result<Option<PredicateEntry>, PredicateError> {
    let row = sqlx::query_as::<_, PredicateEntry>(
        "SELECT * FROM loom_predicates WHERE predicate = $1",
    )
    .bind(predicate)
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

// ---------------------------------------------------------------------------
// Usage tracking
// ---------------------------------------------------------------------------

/// Increment the usage_count by 1 on a canonical predicate.
///
/// Called each time a fact is stored using this predicate.
pub async fn increment_usage_count(
    pool: &PgPool,
    predicate: &str,
) -> Result<(), PredicateError> {
    sqlx::query(
        r#"
        UPDATE loom_predicates
        SET usage_count = usage_count + 1
        WHERE predicate = $1
        "#,
    )
    .bind(predicate)
    .execute(pool)
    .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Candidate lifecycle
// ---------------------------------------------------------------------------

/// Insert or update a predicate candidate.
///
/// If an unresolved candidate with the same predicate text already exists,
/// increments `occurrences` and appends `fact_id` to `example_facts`.
/// Otherwise creates a new candidate row.
pub async fn insert_or_update_candidate(
    pool: &PgPool,
    predicate: &str,
    fact_id: Uuid,
) -> Result<(), PredicateError> {
    // Try to update an existing unresolved candidate first.
    let result = sqlx::query(
        r#"
        UPDATE loom_predicate_candidates
        SET occurrences = occurrences + 1,
            example_facts = array_append(
                COALESCE(example_facts, ARRAY[]::uuid[]),
                $2
            )
        WHERE predicate = $1
          AND resolved_at IS NULL
        "#,
    )
    .bind(predicate)
    .bind(fact_id)
    .execute(pool)
    .await?;

    // If no existing unresolved candidate was found, insert a new one.
    if result.rows_affected() == 0 {
        sqlx::query(
            r#"
            INSERT INTO loom_predicate_candidates (predicate, occurrences, example_facts)
            VALUES ($1, 1, ARRAY[$2]::uuid[])
            "#,
        )
        .bind(predicate)
        .bind(fact_id)
        .execute(pool)
        .await?;
    }

    Ok(())
}

/// Get the current occurrence count for an unresolved predicate candidate.
///
/// Returns `Some(occurrences)` if an unresolved candidate exists for the
/// given predicate text, `None` otherwise. Used after
/// [`insert_or_update_candidate`] to check whether the candidate has
/// reached the operator-review threshold.
pub async fn get_candidate_occurrences(
    pool: &PgPool,
    predicate: &str,
) -> Result<Option<i32>, PredicateError> {
    let row: Option<(i32,)> = sqlx::query_as(
        r#"
        SELECT occurrences
        FROM loom_predicate_candidates
        WHERE predicate = $1
          AND resolved_at IS NULL
        "#,
    )
    .bind(predicate)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|(count,)| count))
}

/// Resolve a predicate candidate by mapping or promoting it.
///
/// Sets `mapped_to` and/or `promoted_to_pack` on the candidate and
/// records `resolved_at` as the current timestamp.
pub async fn resolve_candidate(
    pool: &PgPool,
    candidate_id: Uuid,
    mapped_to: Option<&str>,
    promoted_to_pack: Option<&str>,
) -> Result<(), PredicateError> {
    sqlx::query(
        r#"
        UPDATE loom_predicate_candidates
        SET mapped_to = $2,
            promoted_to_pack = $3,
            resolved_at = now()
        WHERE id = $1
        "#,
    )
    .bind(candidate_id)
    .bind(mapped_to)
    .bind(promoted_to_pack)
    .execute(pool)
    .await?;

    Ok(())
}

/// Query unresolved candidates with occurrences at or above a threshold.
///
/// Used to surface candidates that have been seen enough times to warrant
/// operator review (typically threshold = 5).
pub async fn query_candidates_by_threshold(
    pool: &PgPool,
    min_occurrences: i32,
) -> Result<Vec<PredicateCandidate>, PredicateError> {
    let rows = sqlx::query_as::<_, PredicateCandidate>(
        r#"
        SELECT *
        FROM loom_predicate_candidates
        WHERE resolved_at IS NULL
          AND occurrences >= $1
        ORDER BY occurrences DESC
        "#,
    )
    .bind(min_occurrences)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// List all predicate candidates (resolved and unresolved).
///
/// Used by the dashboard for the full candidate view.
pub async fn list_all_candidates(
    pool: &PgPool,
) -> Result<Vec<PredicateCandidate>, PredicateError> {
    let rows = sqlx::query_as::<_, PredicateCandidate>(
        "SELECT * FROM loom_predicate_candidates ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn predicate_error_displays_message() {
        let err = PredicateError::Sqlx(sqlx::Error::RowNotFound);
        assert!(err.to_string().contains("database error"));
    }
}
