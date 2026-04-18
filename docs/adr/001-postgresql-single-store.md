# ADR-001: PostgreSQL as Single System of Record

## Status

Accepted

## Context

AI memory systems typically use multiple specialized stores: a vector database for
embeddings (Pinecone, Weaviate), a graph database for relationships (Neo4j), and a
relational database for metadata. This creates operational complexity, consistency
challenges, and deployment friction.

Project Loom needs to store episodes (text + embeddings), entities and facts (graph
relationships), and audit logs (relational data). The question is whether to use
specialized stores or consolidate.

## Decision

Use PostgreSQL 16 as the single system of record for all data:

- **Embeddings**: pgvector extension with IVFFlat indexes for vector similarity search.
- **Graph traversal**: Recursive CTEs constrained to 1-2 hops (shallow graph by design).
- **Audit**: pgAudit extension + custom audit log table.
- **Full-text**: PostgreSQL built-in tsvector if needed later.

No external vector store, no graph database, no secondary data store.

## Consequences

### Positive

- Single deployment target — one database to back up, monitor, and scale.
- ACID transactions across all data types (episodes, entities, facts, audit).
- Compile-time query checking via sqlx — catches schema drift at build time.
- Familiar operational model for most teams.
- Simpler Docker Compose setup for contributors.

### Negative

- pgvector IVFFlat is slower than purpose-built vector databases at scale (>1M vectors).
- Recursive CTEs for graph traversal are less expressive than Cypher/Gremlin.
- May need to revisit if retrieval latency exceeds targets at scale.

### Neutral

- Constraining graph depth to 1-2 hops is a design choice, not a PostgreSQL limitation.
- pgvector is actively maintained and improving (HNSW indexes available if IVFFlat is insufficient).
