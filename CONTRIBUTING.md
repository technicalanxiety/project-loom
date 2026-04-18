# Contributing to Project Loom

Thank you for your interest in contributing to Project Loom. This guide will help you get
started.

## Getting Started

### Prerequisites

- **Rust** (stable, latest) — [rustup.rs](https://rustup.rs)
- **Node.js** (v20+) and npm
- **Docker** and Docker Compose
- **cargo-nextest** — `cargo install cargo-nextest`
- **cargo-tarpaulin** (optional, for coverage) — `cargo install cargo-tarpaulin`

### Local Development Setup

```bash
# Clone the repository
git clone https://github.com/technicalanxiety/project-loom.git
cd project-loom

# Copy environment config
cp .env.example .env

# Start all services
docker-compose up -d

# Install dashboard dependencies
cd loom-dashboard && npm install && cd ..

# Verify the engine builds
cd loom-engine && cargo check && cd ..
```

### Running Tests

```bash
# Rust unit tests
cd loom-engine && cargo nextest run

# Start test database for integration tests
docker-compose -f docker-compose.test.yml up -d postgres-test

# Run integration tests
DATABASE_URL_TEST=postgres://loom_test:loom_test@localhost:5433/loom_test \
  cargo nextest run --profile integration

# Dashboard tests
cd loom-dashboard && npm test

# Dashboard coverage
npm run test:coverage
```

## Development Workflow

1. **Branch from main**: Create a feature branch (`feat/your-feature` or `fix/your-bug`).
2. **Write code**: Follow the conventions below.
3. **Write tests**: All new code must have tests. No exceptions.
4. **Run checks**: `cargo clippy`, `cargo fmt --check`, `biome check` (dashboard).
5. **Commit**: Use conventional commits (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`).
6. **Open a PR**: Describe what changed, what was tested, and any open questions.

## Code Conventions

### Rust (loom-engine)

- Functional style preferred. Pure functions, immutable data, composition.
- All public items get `///` doc comments. Modules get `//!` docs.
- Use `Result<T, E>` everywhere. No `unwrap()` in non-test code.
- SQL queries use `sqlx::query!` / `sqlx::query_as!` for compile-time checking.
- Parameterized queries only — never interpolate strings into SQL.
- Keep files under ~300 lines.

### TypeScript (loom-dashboard)

- Functional React components only. No class components.
- All exports get `/** JSDoc */` comments.
- Named exports preferred over default exports.
- API calls go through `src/api/client.ts` — no fetch in components.
- Use Biome for linting and formatting: `npm run lint:fix`.

### SQL Migrations

- Sequential numbering: `NNN_description.sql`.
- All tables prefixed with `loom_`.
- Soft deletes via `deleted_at` column.
- Comments explaining why tables/columns exist.

## Architecture Decisions

Key design decisions are documented in `docs/adr/`. Read these before proposing
architectural changes. If your contribution involves a design decision, add a new ADR.

## Reporting Issues

- Use GitHub Issues for bugs and feature requests.
- For security vulnerabilities, see [SECURITY.md](SECURITY.md).

## License

By contributing, you agree that your contributions will be licensed under the Apache-2.0
license.
