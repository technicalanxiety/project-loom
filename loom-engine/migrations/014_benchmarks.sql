-- Benchmark evaluation tables for A/B/C condition comparison.
--
-- loom_benchmark_runs tracks each benchmark execution.
-- loom_benchmark_results stores per-task, per-condition measurements.

CREATE TABLE IF NOT EXISTS loom_benchmark_runs (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    status      TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'running', 'completed', 'failed'))
);

CREATE TABLE IF NOT EXISTS loom_benchmark_results (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id       UUID NOT NULL REFERENCES loom_benchmark_runs(id) ON DELETE CASCADE,
    task_name    TEXT NOT NULL,
    condition    TEXT NOT NULL CHECK (condition IN ('A', 'B', 'C')),
    precision    DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    token_count  INT NOT NULL DEFAULT 0,
    task_success BOOLEAN NOT NULL DEFAULT false,
    latency_ms   INT NOT NULL DEFAULT 0,
    details      JSONB DEFAULT '{}',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_benchmark_results_run_id ON loom_benchmark_results(run_id);
