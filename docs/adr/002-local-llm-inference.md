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

## Amendment (2026-04): Native Ollama on the Docker host is the default

The original decision treated Ollama as a Docker Compose service
alongside the other containers. That assumption breaks down on Apple
Silicon: Docker Desktop on macOS runs a Linux VM that cannot pass
through Metal / MPS, so Ollama-in-Docker on an M-series Mac is
CPU-only. It needs roughly 3-5× the memory of a native install to run
the same model, and inference latency goes up by a similar factor.
Gemma 4 26B is the harshest case — with Docker Desktop's default
allocation, even the classifier (`gemma4:e4b`) reports "model requires
more system memory (9.8 GiB) than is available (1.8 GiB)" and refuses
to load.

The correct default for a single-operator deployment on any hardware
where Docker can't reach the GPU is **native Ollama on the Docker host,
talking to the engine via `host.docker.internal:11434`**. This
recovers Metal/MPS on macOS, CUDA on Linux without Docker GPU
passthrough, and ROCm on anything else — the engine doesn't care which
acceleration path Ollama uses as long as the HTTP surface works.

The updated decision:

- `.env.example` defaults `OLLAMA_URL=http://host.docker.internal:11434`.
- `docker-compose.yml` keeps the `ollama` service defined but gates it
  behind the `with-docker-ollama` Compose profile, so it's only started
  when explicitly requested. Linux hosts with CUDA that prefer
  containerized Ollama run `docker compose --profile with-docker-ollama
  up -d` and set `OLLAMA_URL=http://ollama:11434`.
- `loom-engine` no longer declares `depends_on: ollama` — the LLM
  client already retries and falls back to Azure OpenAI when
  configured, and the `/api/health` endpoint reports Ollama as
  `degraded` rather than blocking startup.
- README, `docs/clients/`, and the `.env.example` header all lead with
  the native path and treat the Docker path as an escape hatch.

For 16 GB hosts, `EXTRACTION_MODEL=gemma4:e4b` is a reasonable default
(same compact model handles both classification and extraction, lower
extraction quality but it actually runs). Larger-memory hosts can set
`EXTRACTION_MODEL=gemma4:26b` to get the full MoE extractor.

This amendment does not change the original decision to use Ollama
over cloud APIs as the primary inference path. It only updates how
Ollama is deployed alongside the rest of the stack.
