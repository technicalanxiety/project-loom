# ── Stage 1: Build ────────────────────────────────────────────
FROM rust:1.86-slim AS builder

WORKDIR /build

# Install build dependencies for OpenSSL / native TLS and musl for static linking
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
        musl-tools \
        ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Add musl target for fully static binary
RUN rustup target add x86_64-unknown-linux-musl

# Copy manifests first for dependency caching
COPY Cargo.toml Cargo.lock* ./

# Create a dummy main.rs so cargo can resolve and cache dependencies
RUN mkdir src && echo 'fn main() {}' > src/main.rs
RUN cargo build --release --target x86_64-unknown-linux-musl || true
RUN rm -rf src

# Copy real source, migrations, and prompts
COPY src/ src/
COPY migrations/ migrations/
COPY prompts/ prompts/

# Build the release binary (static, musl-linked)
RUN cargo build --release --target x86_64-unknown-linux-musl && \
    strip /build/target/x86_64-unknown-linux-musl/release/loom-engine

# ── Stage 2: Runtime (minimal Debian for shell + health checks) ─
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends curl ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/x86_64-unknown-linux-musl/release/loom-engine /loom-engine
COPY --from=builder /build/migrations/ /app/migrations/
COPY --from=builder /build/prompts/   /app/prompts/

WORKDIR /app

EXPOSE 8080

# Health check — curl is available in the runtime image for
# Docker HEALTHCHECK and docker-compose health checks.
HEALTHCHECK --interval=10s --timeout=5s --retries=5 \
    CMD curl -sf http://localhost:8080/api/health || exit 1

ENTRYPOINT ["/loom-engine"]
