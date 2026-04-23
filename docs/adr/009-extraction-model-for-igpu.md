# ADR-009: Extraction model selection for memory-bandwidth-bound hosts

## Status

Accepted

## Context

ADR-002 and its 2026-04 amendment establish Ollama as the inference
path and give two defaults for `EXTRACTION_MODEL`: `gemma4:e4b` for
"16 GB hosts" and `gemma4:26b` for "larger-memory hosts." That framing
assumes total RAM is the only axis that matters. It isn't.

The reference dev node is a Beelink SER5 — AMD Ryzen 5 with integrated
Vega graphics and 32 GB DDR4. Memory *capacity* is fine (any of the
three candidate models fit). Memory *bandwidth* is the bottleneck: the
iGPU shares system DDR4 at ~50 GB/s, compared to dedicated VRAM on a
discrete card at ~500+ GB/s. Inference throughput on this class of
hardware tracks bandwidth, not compute, not RAM size.

Empirically on that host:

| Model | Size | Per-episode extraction | Structured-output quality |
|---|---|---|---|
| `gemma4:26b` | 17 GB | 4-5 min | Good |
| `gemma4:e4b` | 3.3 GB | ~5 s | **Unusable** — returns empty content for complex extraction prompts |
| `qwen2.5:14b` | 8.9 GB | 30-60 s | Reliable JSON |

`gemma4:26b` blows the worker's per-request timeout (previously 30 s,
now 300 s — see *Consequences*) and makes the pipeline impractically
slow: a 2000-episode backlog would take days. `gemma4:e4b` is fast
enough but silently returns `""` for the 25-predicate fact-extraction
prompt, so the pipeline completes without producing facts. Neither is
acceptable.

The timeout issue deserves its own note. Bumping `REQUEST_TIMEOUT` from
30 s to 300 s in `src/llm/client.rs` was a prerequisite: any model
slower than 30 s/request failed before it could produce output, which
also made this benchmarking impossible until the bump landed. 300 s
accommodates the slow path (`gemma4:26b` on iGPU) without masking real
connectivity problems, and embeddings — which always return in under a
second — are unaffected.

## Decision

Introduce a third hardware tier in the `EXTRACTION_MODEL` guidance:

- **Discrete GPU / high bandwidth (≥16 GB VRAM)**: `gemma4:26b` — use
  the full MoE extractor.
- **Shared-memory iGPU or CPU-only, ≥16 GB system RAM**:
  **`qwen2.5:14b`** — best quality/speed trade-off for the class.
- **Very tight memory (≤16 GB total, no discrete GPU)**: `gemma4:e4b`
  accepting lower extraction quality, *or* run extraction off-host via
  Azure OpenAI fallback.

`gemma4:e4b` remains the default for classification everywhere — the
classifier prompt is simple enough that it produces reliable output
even on the small model, and classification latency matters more than
classification quality in the overall pipeline.

The per-request HTTP timeout stays at 300 s project-wide. It is not
configurable — a single constant keeps the operating envelope honest.

## Consequences

### Positive

- Operators on iGPU / APU hardware get a practical default that
  actually produces facts within one ingestion cycle.
- The three-tier framing makes the *bandwidth* axis explicit, so
  future hardware classes (ARM SoCs, Snapdragon X, Strix Halo, etc.)
  map cleanly onto whichever tier their memory path resembles.
- Bumping `REQUEST_TIMEOUT` to 300 s unblocks anyone on slow hardware
  who hits the previous 30 s wall silently.

### Negative

- `qwen2.5:14b` is a different model family from `gemma4:*`, so
  prompt tuning that was validated against Gemma may need revisiting
  if output drift appears in the benchmark suite.
- The longer timeout means a genuinely hung Ollama process takes 5
  minutes to surface as a failed episode instead of 30 seconds.
  Acceptable trade-off given the retry/backoff state machine (ADR-007)
  bounds total wasted work.

### Neutral

- Total model-storage footprint is unchanged; operators pull only the
  tier they need.
- Azure OpenAI fallback still applies as the escape hatch for any
  host where local extraction is impractical at any tier.
