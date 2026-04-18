// loom-engine/src/pipeline/offline/resolve.rs
// Three-pass entity resolution: exact → alias → semantic.
// Prefers fragmentation over collision (recoverable vs. corrupting).

// TODO: Implement
// - resolve_entity(pool, llm_client, extracted, namespace) -> ResolvedEntity
// - pass1_exact_match(pool, name, entity_type, namespace) -> Option<Entity>
// - pass2_alias_match(pool, name, aliases, entity_type, namespace) -> Option<Entity>
// - pass3_semantic_match(pool, llm_client, name, context, entity_type, namespace) -> Option<(Entity, f64)>
// - log_conflict(pool, entity_name, entity_type, namespace, candidates)
