---
inclusion: auto
---

# Testing Conventions — Project Loom

Testing is mandatory for all new code. No feature ships without tests.

## Rust (loom-engine)

### Tools

- **cargo-nextest** for running tests (install: `cargo install cargo-nextest`)
- **cargo-tarpaulin** for coverage (install: `cargo install cargo-tarpaulin`)
- **proptest** for property-based testing
- **wiremock** for HTTP mocking (Ollama API calls)
- **fake** for generating test data

### Commands

```bash
# Run all tests (fast, parallel)
cargo nextest run

# Run with integration profile (longer timeouts for DB tests)
cargo nextest run --profile integration

# Coverage report
cargo tarpaulin --out html --output-dir target/coverage

# Property-based tests only
cargo nextest run -E 'test(proptest)'
```

### Patterns

- **Unit tests**: In the same file, inside `#[cfg(test)] mod tests { ... }`.
- **Integration tests**: In `loom-engine/tests/` directory. Use `#[tokio::test]` for async.
- **DB integration tests**: Connect to test Postgres on port 5433 (see `docker-compose.test.yml`).
  Use `DATABASE_URL_TEST=postgres://loom_test:loom_test@localhost:5433/loom_test`.
- **HTTP mocking**: Use `wiremock::MockServer` for Ollama API calls. Never hit real LLM in tests.
- **Test data**: Use `fake` crate for generating realistic test entities, episodes, facts.
- **Property tests**: Use `proptest` for invariant testing on serialization, parsing, ranking logic.

### What to Test

- Every public function gets at least one unit test.
- Every API endpoint gets integration tests (happy path + error cases).
- Every DB query function gets a test against the test database.
- Pipeline stages get both unit tests (mocked dependencies) and integration tests (real DB).
- Entity resolution gets property-based tests (fuzzy matching invariants).

### Test Naming

```rust
#[test]
fn entity_resolution_exact_match_returns_existing_entity() { ... }

#[test]
fn ingest_episode_rejects_duplicate_content_hash() { ... }

#[tokio::test]
async fn health_endpoint_returns_200_when_db_connected() { ... }
```

## TypeScript (loom-dashboard)

### Tools

- **Vitest** for test runner (configured in vite.config.ts)
- **React Testing Library** for component tests
- **@testing-library/user-event** for simulating user interactions
- **@vitest/coverage-v8** for coverage

### Commands

```bash
# Run all tests (single run)
npm test

# Watch mode during development
npm run test:watch

# Coverage report
npm run test:coverage
```

### Patterns

- Test files colocated with source: `Component.tsx` → `Component.test.tsx`.
- Test user behavior, not implementation details.
- Use `screen.getByRole()` over `getByTestId()` — accessibility-first queries.
- Mock API calls at the `src/api/client.ts` boundary, not at fetch level.

### Coverage Thresholds

- Minimum 60% across statements, branches, functions, lines (will increase as codebase matures).

## Integration Test Infrastructure

```bash
# Start test database (isolated, ephemeral — uses tmpfs)
docker-compose -f docker-compose.test.yml up -d postgres-test

# Run integration tests against it
DATABASE_URL_TEST=postgres://loom_test:loom_test@localhost:5433/loom_test cargo nextest run --profile integration

# Tear down
docker-compose -f docker-compose.test.yml down
```
