// loom-engine/src/worker/processor.rs
// Background episode processing loop (tokio spawned tasks).
// Picks up unprocessed episodes and runs the offline pipeline.

// TODO: Implement
// - start_processor(pool, llm_client) -> JoinHandle
// - process_next_episode(pool, llm_client) -> Option<ProcessResult>
// - Processing loop with configurable poll interval
