# ADR-007: Episode Processing State Machine with Bounded Retries

## Status

Accepted

## Context

The offline worker polls `loom_episodes` every 5 seconds and runs the full
extraction pipeline (embed → extract entities → resolve → extract facts →
supersede → state → procedures) against any episode where `processed =
false`. Before this change, failure handling was a one-liner in
`worker/processor.rs`: log the error, return, and let the next poll try
again.

That design broke the first time we fed it a poison pill.

Two Claude Code sessions bootstrapped via `bootstrap/claude_code_parser.py`
at 3.5 MB and 2.4 MB landed as single episodes. Both exceeded
nomic-embed-text's ~8192-token (~32 KiB) context window. Ollama returned
`HTTP 400 "the input length exceeds the context length"` on every
embedding request. Because the episode was never marked processed, the
next 5-second poll picked it up again, and again, and again — two
embedding requests per poll, forever, with no circuit breaker. On a
cloud Ollama or Azure OpenAI fallback that is real money; on local
Ollama it starves legitimate work.

The immediate fix (chunking transcripts to <24 KiB in the parser)
addresses that specific failure mode. It does not address the general
pattern: **any deterministically-unprocessable episode** — malformed
content, missing a required field, a parser regression that ships
garbage, an extraction prompt that the model can't satisfy — generates
the same infinite-retry loop.

We need the worker to notice "this episode keeps failing" and stop,
without a human having to dig through logs and delete rows.

## Decision

Add an explicit state machine to `loom_episodes` (migration 016) with
these columns:

- `processing_status TEXT NOT NULL DEFAULT 'pending'` — one of `pending`,
  `processing`, `completed`, `failed`, enforced by CHECK constraint.
- `processing_attempts INTEGER NOT NULL DEFAULT 0` — monotonic counter.
- `processing_last_attempt TIMESTAMPTZ` — stamp of the most recent claim.
- `processing_last_error TEXT` — truncated (2 KiB cap) error message
  from the most recent failed attempt.

The legacy `processed BOOLEAN` column is retained and kept in sync with
`processing_status = 'completed'` so read-side tooling continues to work
during the transition. New code must consult `processing_status`.

The worker's control flow becomes:

1. **Poll.** `list_unprocessed_episodes` returns rows where
   `processing_status = 'pending'` AND (`processing_last_attempt IS NULL`
   OR `processing_last_attempt + base * 2^attempts seconds < NOW()`). The
   backoff predicate is evaluated in SQL, not Rust — one round-trip, no
   per-row decision in the worker.

2. **Claim.** Before running the pipeline, the worker issues an atomic
   conditional UPDATE: `SET status='processing', attempts=attempts+1,
   last_attempt=NOW() WHERE id=$1 AND status='pending'`. If the UPDATE
   returns `None`, someone else got the row — skip quietly. This makes
   the design safe for multiple worker replicas even though we currently
   run one.

3. **Succeed.** `mark_episode_processed` sets `status='completed'`,
   clears `last_error`, and writes extraction metrics. `processed=true`
   is set in the same statement for back-compat.

4. **Fail.** `record_processing_failure(id, error, max_attempts)` runs
   one UPDATE: if `attempts >= max_attempts`, transition to `failed` and
   stop. Otherwise return to `pending` so the next poll reconsiders
   after the backoff window. Either way, `last_error` is populated with
   a truncated copy of the error message.

Backoff is exponential: `EPISODE_BACKOFF_BASE_SECS * 2^attempts`.
Defaults (5 attempts, base 30s) produce retry gaps of 30s, 60s, 120s,
240s, 480s, for a total window of ~8 minutes before a row is parked in
`failed`. Both knobs are env-var configurable without a rebuild.

Operator surface:
- **`GET /dashboard/api/episodes/failed`** lists parked episodes with
  source, namespace, attempt count, and last error so triage can happen
  from the dashboard without SQL access.
- **`POST /dashboard/api/episodes/{id}/requeue`** resets
  `status=pending`, `attempts=0`, clears `last_error`. This is the
  escape hatch for "I chunked the parser / raised the context window /
  fixed the prompt, try again."
- **`PipelineHealthResponse.failed_episode_count`** is a first-class
  health signal; a non-zero value means the pipeline has unresolved
  work that isn't going to progress without human attention.

## Alternatives considered

1. **Dead-letter table.** Move `failed` rows to `loom_episodes_dead` for
   forensic storage. Rejected: soft-delete semantics already exist on
   the main table (`deleted_at`), and splitting creates two sources of
   truth for episode lookup. A status column is simpler and the
   partial index on `status='failed'` keeps the hot path fast.

2. **Fixed-delay retry (no state, no backoff).** Bumping the poll
   interval to 5 minutes would slow the poison-pill loop but not stop
   it, and would also slow legitimate retries after transient failures.
   Exponential backoff + cap is the standard pattern for a reason.

3. **Rust-side in-memory retry tracking.** Keep a `HashMap<Uuid, u32>`
   of attempts in the processor. Rejected: crashes or restarts lose the
   counter; the ground-truth state must be in Postgres anyway; and
   in-memory tracking wouldn't survive the multi-replica scenario even
   if it isn't the current deployment.

4. **Always fail fast after one bad attempt.** Rejected: transient
   Ollama 500s, connection timeouts, and model cold-starts are common
   enough that a one-strike policy would park too many healthy
   episodes.

## Consequences

### Positive

- Poison pills are bounded: worst case ~8 minutes of retry load, then
  silence, regardless of how many bad episodes land at once. On cloud
  LLM services this is the difference between a small tax and an
  unbounded bill.
- Failure reasons are persisted. Every failed episode carries the
  error message from its last attempt — no more "grep logs from three
  days ago to figure out why this is stuck."
- The dashboard has a concrete "things need human attention" signal.
  `failed_episode_count > 0` is actionable; `processed = false`
  episodes that are actively being retried are not.
- Atomic claim makes the design multi-replica-safe for free. We don't
  need it yet, but we paid nothing to keep the option.

### Negative

- Four new columns and two new indexes on the hottest table in the
  system. Size impact is small (INTEGER + TIMESTAMPTZ + a mostly-NULL
  TEXT), but every write path now touches more columns. Acceptable
  tradeoff for the correctness win.
- Two configuration knobs. `EPISODE_MAX_ATTEMPTS` and
  `EPISODE_BACKOFF_BASE_SECS` are yet another pair of env vars the
  operator has to understand. Documented in `.env.example` and
  README.md with concrete examples of the resulting delay schedule.
- The backoff formula uses a bit-shift in SQL (`1::bigint << LEAST(
  processing_attempts, 20)`). The `LEAST(..., 20)` cap prevents overflow
  but creates a ceiling of ~12 days between attempts — well past any
  sane max_attempts value, but it's a foot-gun if someone sets
  `EPISODE_MAX_ATTEMPTS=100` without reading the code.

### Neutral

- The legacy `processed BOOLEAN` column is still maintained. A future
  migration can drop it once all read-side tooling (exports, the
  Python CLI, any ad-hoc SQL in the ops notebook) has been migrated to
  `processing_status`.
- Long error messages are truncated to 2 KiB. Ollama's 400 responses
  can be verbose; we keep enough for a human to understand what
  happened without letting a pathological error message balloon a
  failed-episodes table.
- Order-of-poll is `processing_last_attempt NULLS FIRST, ingested_at
  ASC` — never-attempted episodes win, then oldest-attempted. This is
  fair under load but means a flood of new episodes can temporarily
  delay the retry of an older failing one. Intentional; retries are
  best-effort, first-time extractions are not.
