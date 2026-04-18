// loom-engine/src/worker/scheduler.rs
// Periodic tasks: 24h snapshots, tier promotion, weekly entity health check.
// Uses tokio::time::interval for scheduling.

// TODO: Implement
// - start_scheduler(pool) -> JoinHandle
// - snapshot_hot_tier(pool) — runs every 24 hours
// - promote_tiers(pool) — runs every 24 hours
// - entity_health_check(pool) — runs weekly
