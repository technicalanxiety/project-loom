# ADR-002: Local LLM Inference via Ollama

## Status

Accepted

## Context

Project Loom requires LLM capabilities for entity extraction, fact extraction, intent
classification, and embedding generation. Options include cloud APIs (OpenAI, Anthropic,
Azure OpenAI) or local inference.

Key considerations:
- Privacy: Episode content may contain sensitive conversation data.
- Cost: High-volume extraction on cloud APIs is expensive.
- Latency: Local inference avoids network round-trips.
- Availability: No dependency on external service uptime.

## Decision

Use Ollama for local LLM inference:

- **Extraction**: Gemma 4 26B MoE (gemma4:26b) — strong structured output.
- **Classification**: Gemma 4 E4B (gemma4:e4b) — fast, lightweight.
- **Embeddings**: nomic-embed-text — 768-dimension vectors, good quality/speed tradeoff.

Azure OpenAI is configured as an optional fallback (env vars) but not required.

## Consequences

### Positive

- Zero data leaves the machine — complete privacy.
- No per-token costs after hardware investment.
- No external service dependency.
- Reproducible results (same model, same weights, deterministic with temperature=0).

### Negative

- Requires GPU for reasonable performance (NVIDIA recommended).
- Model quality may lag behind frontier cloud models.
- Operator must manage model downloads and updates.

### Neutral

- Ollama's HTTP API is simple and well-documented.
- Fallback to Azure OpenAI provides an escape hatch if local quality is insufficient.
