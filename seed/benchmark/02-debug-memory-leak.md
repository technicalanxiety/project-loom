# Worker Service memory and the Redis Cache

The Worker Service depends on Redis Cache for hot-path job
deduplication and for short-lived idempotency keys. Roughly half of
its working set lives in Redis Cache; the rest is in-process Rust
buffers that the Worker Service keeps around between jobs.

We monitor the Worker Service through two telemetry streams. The
first is process-level RSS reported every fifteen seconds; the second
is Redis Cache hit/miss rates for keys owned by the Worker Service.
Together they tell us whether a memory growth problem is local to
the Worker Service heap or is being driven by a Redis Cache eviction
storm causing the worker to re-fetch and cache more aggressively.

The most recent memory-leak suspect was a slab of cached
Tokio-buffered futures that the Worker Service held while waiting
for Redis Cache responses on a slow network link. Reducing the
worker's prefetch depth from 64 to 16 dropped steady-state RSS by
roughly forty percent without any drop in throughput.
