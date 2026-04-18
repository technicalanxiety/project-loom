// loom-engine/src/db/facts.rs
// Fact CRUD + supersession queries — compile-time checked via sqlx.

// TODO: Implement
// - insert_fact(pool, fact) -> Fact
// - get_current_facts(pool, subject_id, namespace) -> Vec<Fact>
// - find_contradicting(pool, subject_id, predicate, namespace) -> Option<Fact>
// - supersede_fact(pool, old_id, new_id)
// - search_by_embedding(pool, embedding, namespace, limit) -> Vec<Fact>
// - get_facts_for_entities(pool, entity_ids, namespace) -> Vec<Fact>
