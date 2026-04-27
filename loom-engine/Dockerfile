# ── Stage 1: Build ────────────────────────────────────────────
FROM rust:1.88-slim AS builder

WORKDIR /build

# Install build dependencies
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        pkg-config \
        ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Copy manifests first for dependency caching
COPY Cargo.toml Cargo.lock* ./

# Create a dummy main.rs so cargo can resolve and cache dependencies
RUN mkdir src && echo 'fn main() {}' > src/main.rs
RUN cargo build --release || true
RUN rm -rf src

# Copy real source, migrations, prompts, and embedded seed corpus.
# `seed/` is read by `include_str!` in pipeline/benchmark.rs at compile time so
# the benchmark namespace can be populated without a runtime FS dependency.
# Missing this COPY would fail compilation, not runtime.
COPY src/ src/
COPY migrations/ migrations/
COPY prompts/ prompts/
COPY seed/ seed/

# Build the release binary
RUN cargo build --release && \
    strip /build/target/release/loom-engine

# ── Stage 2: Runtime (minimal Debian for shell + health checks) ─
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends curl ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/loom-engine /loom-engine
COPY --from=builder /build/migrations/ /app/migrations/
COPY --from=builder /build/prompts/   /app/prompts/

WORKDIR /app

EXPOSE 8080

# Health check — curl is available in the runtime image for
# Docker HEALTHCHECK and docker-compose health checks.
HEALTHCHECK --interval=10s --timeout=5s --retries=5 \
    CMD curl -sf http://localhost:8080/api/health || exit 1

ENTRYPOINT ["/loom-engine"]
