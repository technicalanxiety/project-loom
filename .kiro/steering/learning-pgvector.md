---
description: pgvector learning context — index types, distance operators, query patterns, and sqlx integration
inclusion: fileMatch
fileMatchPattern: "loom-engine/src/llm/embeddings.rs,loom-engine/src/db/traverse.rs,loom-engine/migrations/**/*.sql"
---

# pgvector Learning Context

Jason is actively learning pgvector. When working on embedding-related code, provide
extra context and explain pgvector-specific patterns.

## Key Concepts to Reinforce

### Index Types

- **IVFFlat**: Faster to build, good for datasets under ~1M vectors. Requires `lists` parameter
  (rule of thumb: sqrt(num_rows) for up to 1M rows). Must `VACUUM` after bulk inserts to
  rebuild the index.
- **HNSW**: Better recall at higher dimensions, no need to rebuild after inserts, but uses more
  memory and slower to build. Consider if IVFFlat recall is insufficient.

### Distance Operators

- `<->` — L2 (Euclidean) distance
- `<=>` — Cosine distance (most common for text embeddings)
- `<#>` — Inner product (negative, for max inner product search)

For nomic-embed-text (768-dim), cosine distance (`<=>`) is the standard choice.

### Query Patterns

```sql
-- Find 10 most similar episodes to a query embedding
SELECT id, content, embedding <=> $1::vector AS distance
FROM loom_episodes
WHERE namespace = $2 AND deleted_at IS NULL
ORDER BY embedding <=> $1::vector
LIMIT 10;

-- Hybrid search: vector similarity + recency weighting
SELECT id, content,
  (embedding <=> $1::vector) * 0.7 + 
  (1.0 - EXTRACT(EPOCH FROM (now() - occurred_at)) / 86400.0 / 30.0) * 0.3 AS score
FROM loom_episodes
WHERE namespace = $2 AND deleted_at IS NULL
ORDER BY score
LIMIT 10;
```

### Performance Tips

- Always filter by namespace BEFORE vector search (partial index helps).
- Set `ivfflat.probes` higher for better recall at cost of speed (default 1, try 10-20).
- Pre-filter with WHERE clauses to reduce the candidate set before vector comparison.
- Monitor index size: `SELECT pg_size_pretty(pg_relation_size('idx_episodes_embedding'))`.

### sqlx Integration

pgvector types work with the `pgvector` crate's `Vector` type:

```rust
use pgvector::Vector;

/// Retrieve the most similar episodes to a query embedding within a namespace.
let embedding = Vector::from(vec![0.1, 0.2, ...]);
let results = sqlx::query_as!(
    EpisodeMatch,
    r#"SELECT id, content, embedding <=> $1::vector AS "distance!"
    FROM loom_episodes
    WHERE namespace = $2 AND deleted_at IS NULL
    ORDER BY embedding <=> $1::vector
    LIMIT $3"#,
    embedding as Vector,
    namespace,
    limit
)
.fetch_all(&pool)
.await?;
```
