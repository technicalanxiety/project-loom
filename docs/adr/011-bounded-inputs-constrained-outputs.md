# ADR-011: Bounded inputs and constrained outputs for the offline LLM pipeline

## Status

Accepted

## Context

ADR-007 introduced bounded retries with exponential backoff to stop
deterministically-bad episodes from consuming inference cycles forever.
That mechanism works — it parks failing episodes in
`processing_status = 'failed'` after `EPISODE_MAX_ATTEMPTS` (default 5).
But ADR-007 only addresses the *symptom*: the underlying causes that
generate failed episodes are still running unchecked, and the failed
episodes are silently lost from recall.

The dashboard's Recent Failures panel shows two recurring failure modes
on `claude-code-bootstrap` ingests:

1. **Embedding 400s.** `nomic-embed-text` returns
   `"the input length exceeds the context length"`. Episodes never
   embed; with `embedding IS NULL`, retrieval at
   `loom-engine/src/pipeline/online/retrieve.rs:497` filters them out
   entirely. Content survives in the `episodes` table but is invisible
   to recall.

2. **Extraction JSON parse failures.** qwen2.5:14b returns malformed
   JSON: trailing prose after the closing brace, missing the required
   `entities` field, or output that's not JSON at all. The
   `serde_json::from_value` deserialization at
   `loom-engine/src/llm/extraction.rs:259` rejects these. The embedding
   already succeeded by this point so the episode is recallable via
   vector search, but no entities or facts are extracted — the graph is
   degraded for that episode.

Both failure modes share a structural cause: the pipeline trusts the
LLM provider to behave well at its boundaries (an oversized input and a
free-form output prompt), and the trust is misplaced.

### Why character-based truncation isn't enough for embeddings

`generate_episode_embedding` truncates content to 30,000 characters
before sending to nomic-embed-text. The 30K figure assumed ~4
chars/token (typical English prose) against the 8192-token context
window — a generous safety margin.

It is not enough for the actual content stream. Claude Code transcripts
contain:

- Escaped JSON (`\"\\n\"` and similar) in tool inputs/outputs.
- Base64-encoded images embedded in screenshots and pasted file content.
- Long tool-result blobs (file reads, `ls` output, grep dumps).
- Code diffs with dense punctuation.

These tokenize at roughly 2 chars/token in the worst case. 30,000 chars
× 0.5 tokens/char = ~15,000 tokens — almost twice the context window.
The embedding API rejects these inputs with HTTP 400.

### Why prompt instructions aren't enough for extraction

The original extraction path called `client.call_llm(...)` with a
system prompt instructing qwen2.5:14b to emit a JSON object. There was
no structural constraint on the output — only the model's incentive to
follow instructions.

qwen2.5:14b is reliable on this prompt most of the time, but
free-form generation has no guarantees. We observed three concrete
failure shapes:

- `"trailing characters at line 1 column 196"` — the model emitted
  valid JSON followed by prose ("Let me know if you need…"). The
  fence-stripping fallback in `deserialize_response` handles markdown
  fences but not arbitrary trailing prose.
- `"missing field 'entities' at line 1 column 529"` — the model
  emitted a different top-level shape (`{"items": [...]}` or similar).
- `"response is neither a valid JSON object nor a JSON string"` — the
  model emitted prose only. No JSON at all.

These are not bugs in qwen2.5:14b. They are the failure mode of any
prompt-only structured output strategy with a small instruction-tuned
model.

## Decision

Two bounded-system changes:

### 1. Lower `EMBED_CHAR_LIMIT` from 30,000 to 16,000

Recompute the budget against worst-case tokenization:
8192 tokens × 2 chars/token = 16,384 chars. Round down to 16,000 for a
small safety margin. This is the simplest change that eliminates the
embedding 400s deterministically, regardless of content type.

The truncation is a *representational* decision — what the embedding
model sees as input. The episode's `content` column remains verbatim,
preserving ADR-005. Vector recall on episodes longer than 16K chars
operates on the embedding of their leading content, which captures the
topic signal in nearly all cases (Claude Code transcripts state the
user's intent up front).

A token-aware truncation pass (using the `tokenizers` crate to load
nomic-embed-text's tokenizer.json and truncate to 7,500 tokens) is the
correct long-term fix and is filed as a follow-up. The 16,000-char cap
is the immediate, deterministic win.

### 2. Constrain extraction output via `response_format: json_schema`

Pass a JSON Schema (auto-derived from `ExtractionResponse` /
`FactExtractionResponse` via `schemars`) to the chat completions request
as `response_format: {"type": "json_schema", "json_schema": {...}}`.

On Ollama ≥ 0.5 this routes through llama.cpp's GBNF grammar
constraint: the model's sampler can only emit tokens that maintain
schema validity. Output is guaranteed to deserialize. On Azure OpenAI
the same field hits the OpenAI structured-outputs path.

Implementation:

- `LlmClient::call_llm_with_schema` wraps the existing chat
  completions plumbing, passing `(name, schema)` through
  `build_chat_body` to attach `response_format`.
- `extract_entities` and `extract_facts` call
  `schemars::schema_for!(ResponseType)` and pass the resulting schema
  to `call_llm_with_schema`. Generation is now schema-constrained at
  the provider boundary.
- `LlmClient::call_llm` (used by `classification.rs`) is unchanged —
  classification's fallback-to-Chat behavior on parse failure means
  schema mode would buy nothing there.

The fence-stripping and `Value::String` fallback in
`deserialize_response` remain. They are no longer the primary defense,
but they remain a cheap second line for any future provider whose
schema enforcement is incomplete.

## Consequences

### Positive

- Embedding 400s on Claude Code transcripts stop generating. The
  highest-volume failure class disappears.
- Extraction JSON parse failures stop generating. The pipeline
  produces entities and facts deterministically when the LLM is
  reachable.
- Retrieval coverage improves cumulatively as previously-failed
  episodes are requeued and successfully processed.
- The fixes are entirely engine-internal. No bootstrap parser changes,
  no client template changes, no MCP wire-protocol changes. Verbatim
  invariant (ADR-005) is preserved — only what the embedding model
  *sees* is truncated, not what is stored.
- Aligns with the bounded-retry philosophy of ADR-007: rather than
  tolerating routine failures via retry, eliminate the routine
  failures at their source. Retry/backoff still bounds the residual
  poison-pill class.

### Negative

- Embedding semantic fidelity is reduced for episodes longer than
  16,000 chars. The vector represents only the leading content. For
  most Claude Code transcripts (intent-up-front structure) this is
  fine; for an episode that pivots topic past the truncation point,
  the embedding will be biased toward the early content.
- Schema-constrained generation is ~5–15% slower per qwen2.5:14b
  call. Negligible at single-operator volume; worth noting for
  benchmarking.
- Adds a new compile-time dependency (`schemars`). Build times
  increase modestly.
- Schema features that GBNF cannot enforce (regex `pattern`,
  `format` keywords like `date-time`, recursive `$ref`) are silently
  dropped on Ollama. Our schemas use only objects, arrays, enums,
  primitives, and one-level `$ref` — within the supported set.

### Neutral

- ADR-007's retry/backoff mechanism is unchanged. Episodes that fail
  for genuinely poison-pill reasons (e.g. an LLM provider outage
  mid-extraction) still bound out at `EPISODE_MAX_ATTEMPTS`. The
  difference is that the *steady-state* failure rate trends to zero
  rather than a recurring background level.
- Operators with episodes already in `processing_status = 'failed'`
  must explicitly requeue them (`POST
  /dashboard/api/episodes/{id}/requeue`) to benefit from the fixes.
  Failed-state is operator-controlled by design (ADR-007); no
  automatic re-processing.
- Ollama versions older than 0.5 silently ignore unknown
  `response_format` types and fall back to free-form output. The
  fence-stripping fallback in `deserialize_response` mitigates this
  but does not eliminate it. Operators on stale Ollama should upgrade.
- A token-aware embedding truncation (using a real tokenizer) is the
  correct long-term refinement and is filed as follow-up work. The
  16,000-char cap is the immediate fix; the tokenizer-based path
  reclaims the prose-heavy completeness loss.

## Follow-up

- Token-aware embedding truncation: load `nomic-embed-text`'s
  tokenizer.json via the `tokenizers` crate at startup, truncate
  input by token count rather than char count. Recovers ~50% of the
  fidelity loss for prose-heavy content while keeping the same safety
  margin.
- Spot-check 3–5 newly extracted episodes after the schema-mode
  rollout: confirm entity and fact richness has not degraded vs.
  free-form prompting (theoretical risk: schema rigidity could push
  the model toward emptier output).
