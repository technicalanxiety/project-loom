// loom-engine/src/pipeline/offline/supersede.rs
// Fact supersession detection.
// When new fact contradicts existing (same subject + predicate, different object),
// sets valid_until on old fact and links via superseded_by.

// TODO: Implement
// - check_supersession(pool, new_fact) -> Option<FactId>
// - apply_supersession(pool, old_fact_id, new_fact_id)
