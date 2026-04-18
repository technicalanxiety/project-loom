---
description: Rust coding conventions, crate stack, and patterns for loom-engine
inclusion: fileMatch
fileMatchPattern: "loom-engine/**/*.rs"
---

# Rust Conventions — loom-engine

## Crate Stack

- **axum** for HTTP (not actix, not warp)
- **tokio** async runtime with `full` features
- **sqlx** with compile-time checked queries (`query!` / `query_as!` macros)
- **serde** / **serde_json** for all serialization
- **reqwest** for HTTP client (Ollama calls)
- **tracing** for structured logging (not `log` crate)
- **thiserror** for library errors, **anyhow** acceptable in main/binary code
- **tower** / **tower-http** for middleware

## Style Rules

- Prefer `impl` blocks with associated functions over free functions when operating on a type.
- Use `Result<T, E>` everywhere. Never `unwrap()` in non-test code. `expect()` only with a
  meaningful message for truly impossible states.
- Prefer `?` operator over explicit match on Result/Option.
- All public items get `///` doc comments. Module files get `//!` module-level docs.
- Use `#[derive(Debug, Clone, Serialize, Deserialize)]` on all types that cross boundaries.
- Prefer `&str` over `String` in function parameters where ownership isn't needed.

## SQL Patterns

- Always use `sqlx::query!` or `sqlx::query_as!` for compile-time checking.
- Parameterized queries only — never format SQL strings.
- Separate online and offline connection pools. Online queries must never be starved.

## Error Handling

- Define domain errors per module using `thiserror::Error`.
- Map sqlx errors to domain errors at the db layer boundary.
- API handlers return `axum::response::Result` with appropriate status codes.
- Log errors with `tracing::error!` including span context.

## Testing

- Unit tests in the same file (`#[cfg(test)] mod tests`).
- Integration tests in `tests/` directory.
- Use `#[tokio::test]` for async tests.
- `proptest` is available for property-based testing where appropriate.

## File Organization

- One type per file in `types/` module.
- One query domain per file in `db/` module.
- Pipeline stages are individual files under `pipeline/online/` and `pipeline/offline/`.
- Keep files under ~300 lines. Split if growing.
