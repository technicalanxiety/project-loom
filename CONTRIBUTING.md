# Contributing

Loom is personal infrastructure. The short version of this document is:

- **PRs are not reviewed.** Open one if it's useful to you; I won't be merging it.
- **Issues are not answered.** If something breaks on your setup, it's a fork-level concern.
- **Feature requests are not tracked.** If you want Loom to do something it doesn't, fork it.
- **Installation and deployment support are not offered.** The install path is documented; if the docs are insufficient for you, your fork is the mechanism for fixing that.

See [PROJECT-STANCE.md](PROJECT-STANCE.md) for the full rationale. The license is MIT precisely so forks can evolve without talking to me.

---

## If you are forking: conventions the author follows

These are not contribution requirements — they are the conventions I follow internally. Adopting them in your fork keeps the codebase consistent with upstream if you ever want to pull changes, but you are free to diverge.

### Local development setup

- **Rust** (stable, latest) — [rustup.rs](https://rustup.rs)
- **Node.js** (v20+) and npm
- **Docker** and Docker Compose
- **cargo-nextest** — `cargo install cargo-nextest`
- **cargo-tarpaulin** (optional, for coverage) — `cargo install cargo-tarpaulin`
- **sqlx-cli** — `cargo install sqlx-cli --no-default-features --features postgres`

```bash
git clone https://github.com/technicalanxiety/project-loom.git
cd project-loom
cp .env.example .env
docker-compose up -d
cd loom-dashboard && npm install && cd ..
cd loom-engine && cargo check && cd ..
```

### Running tests

```bash
# Rust unit tests (no database needed)
cd loom-engine && cargo nextest run

# Integration tests (database required)
docker-compose -f docker-compose.test.yml up -d postgres-test
DATABASE_URL_TEST=postgres://loom_test:loom_test@localhost:5433/loom_test \
  cargo nextest run --profile integration

# Dashboard tests
cd loom-dashboard && npm test
```

### Rust conventions

- Functional style preferred. Pure functions, immutable data, composition.
- All public items get `///` doc comments. Modules get `//!` docs.
- Use `Result<T, E>` everywhere. No `unwrap()` in non-test code.
- SQL queries use parameterized statements via sqlx. Never interpolate strings into SQL.
- Keep files under ~300 lines when practical; several current files exceed this.
- Pre-commit: `cargo clippy -- -D warnings` and `cargo fmt --check`.

### TypeScript conventions (dashboard)

- Functional React components only. No class components.
- All exports get `/** JSDoc */` comments.
- Named exports preferred over default exports.
- API calls go through `src/api/client.ts` — no `fetch` in components.
- Biome for linting and formatting: `npm run lint:fix`.

### SQL migration conventions

- Sequential numbering: `NNN_description.sql`. Do not reuse numbers.
- All tables prefixed with `loom_`.
- Soft deletes via `deleted_at` column.
- Comments explaining why tables/columns exist, not just what they do.
- Additive migrations preferred. Destructive migrations require a clear narrative in the file header.

### Architecture Decisions

Key design decisions live in [docs/adr/](docs/adr/). Read them before changing architectural shape. If a fork makes a decision that diverges from upstream in a load-bearing way, add an ADR documenting the divergence so future-you can remember why.

### Test corpus and private data

Private test corpora (real work-origin conversation content, personal seed documents) stay out of the public repository. The public test suite uses synthetic fixtures only. This is an operational rule — if you fork, you get to decide your own rule, but mixing the two in a public fork is a privacy failure waiting to happen.

### Commit conventions

Conventional commits (`feat:`, `fix:`, `docs:`, `refactor:`, `test:`, `chore:`). Commit messages should explain the *why*; the diff already shows the *what*.

## License

MIT — see [LICENSE](LICENSE). Forks can relicense compatible derivatives; upstream stays MIT.
