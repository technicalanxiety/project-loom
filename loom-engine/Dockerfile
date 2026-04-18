# Build stage
FROM rust:1.82-slim AS builder
WORKDIR /app
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock* ./
COPY src/ src/
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/loom-engine /usr/local/bin/loom-engine
COPY migrations/ /app/migrations/
COPY prompts/ /app/prompts/
WORKDIR /app
EXPOSE 8080
CMD ["loom-engine"]
