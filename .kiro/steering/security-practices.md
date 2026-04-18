---
inclusion: auto
---

# Security Practices — Project Loom

## Secrets Management

- **NEVER** hardcode secrets, tokens, passwords, or connection strings in source code.
- All secrets come from environment variables loaded via `.env` files (local) or a secret manager (production).
- Use `dotenvy` in Rust to load `.env` at startup. Access via `std::env::var()`.
- When writing example configs or documentation, use placeholder values: `changeme`, `your-token-here`, `<REPLACE_ME>`.
- If you encounter a hardcoded secret in existing code, flag it immediately and replace with an env var reference.

## Authentication (Target Architecture)

The current bearer token auth (`LOOM_BEARER_TOKEN`) is a prototype placeholder. The target auth model:

- **API keys with rotation**: Each client gets a unique API key stored hashed in the database.
- **Key rotation**: Support issuing new keys and deprecating old ones with a grace period.
- **Rate limiting**: Per-key rate limits via tower middleware.
- **MCP auth**: Follow the MCP specification's auth mechanism when it stabilizes.
- Do NOT implement OAuth/OIDC in MVP — it's overkill for a local-first tool. API keys are sufficient.

## Input Validation

- Validate all external input at the API boundary (axum extractors + custom validators).
- Sanitize namespace names, entity names, and predicate values — no SQL injection vectors.
- Use sqlx parameterized queries exclusively. String interpolation in SQL is a hard error.
- Limit request body sizes via tower middleware.

## Open-Source Sanitization Checklist

Before any public release:

- [ ] Grep for internal references (company names, internal URLs, private IPs, employee names)
- [ ] Verify `.env.example` contains only placeholder values
- [ ] Verify no `.env` files are tracked in git history (use `git log --all --full-history -- .env`)
- [ ] Review `docker-compose.yml` for internal registry references
- [ ] Check all migration files for real data in seed scripts
- [ ] Verify no API keys, tokens, or passwords in commit history
- [ ] Run `git secrets --scan-history` if available
- [ ] Review README and docs for internal-only references
- [ ] Ensure license headers are present where required

## Dependency Security

- Pin all dependency versions (exact in Cargo.toml, exact in package.json).
- Review new dependencies before adding — check maintenance status, download counts, known vulnerabilities.
- Run `cargo audit` periodically for Rust dependency vulnerabilities.
- Run `npm audit` periodically for Node dependency vulnerabilities.
