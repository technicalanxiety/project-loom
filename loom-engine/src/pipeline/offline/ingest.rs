// loom-engine/src/pipeline/offline/ingest.rs
// Episode ingestion + dedup (sha2 content hash).
// Stores episode immediately, spawns extraction as tokio task.

// TODO: Implement
// - ingest_episode(pool, content, source, namespace, metadata) -> IngestResult
// - compute_content_hash(content) -> String (SHA-256 via sha2 crate)
// - check_duplicate(pool, hash) -> Option<EpisodeId>
// - IngestResult { episode_id, status: Accepted | Duplicate | Queued }
