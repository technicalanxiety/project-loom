# Security Policy

## Reporting a Vulnerability

If you discover a security vulnerability in Project Loom, please report it responsibly.

**Do NOT open a public GitHub issue for security vulnerabilities.**

Instead, please email: **security@technicalanxiety.dev** (or use GitHub's private
vulnerability reporting if available on this repository).

Include:
- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if you have one)

We will acknowledge receipt within 48 hours and provide a timeline for a fix.

## Security Model

### Threat Model

Project Loom is designed as a **local-first tool** running on a developer's machine or
within a trusted network. The primary threat model assumes:

- **Trusted operator**: The person running Loom controls the machine and network.
- **Untrusted input**: Episode content ingested from external sources (chat exports, etc.)
  may contain malicious content.
- **Local LLM**: Inference runs locally via Ollama. No data leaves the machine for LLM calls.

### Authentication

- API access is protected by bearer token authentication (MVP).
- Target architecture: API keys with rotation, hashed storage, and per-key rate limiting.
- All auth tokens are loaded from environment variables, never hardcoded.

### Data Protection

- **PostgreSQL is the single data store.** No data is replicated to external services.
- All SQL queries use parameterized statements via sqlx (compile-time checked).
- Input validation occurs at the API boundary before any database interaction.
- Soft deletes preserve audit trail — data is never hard-deleted.
- Namespace isolation prevents cross-tenant data access.

### Secrets Management

- All secrets (database credentials, API tokens, LLM endpoints) are stored in environment
  variables.
- `.env` files are gitignored. `.env.example` contains only placeholder values.
- No secrets appear in logs — tracing is configured to exclude sensitive fields.

### Dependencies

- Rust dependencies are pinned to specific versions in `Cargo.toml`.
- Node dependencies are pinned in `package.json`.
- `cargo audit` and `npm audit` should be run periodically.

### Known Limitations (MVP)

- Bearer token auth is a single shared secret — no per-client isolation.
- No TLS between internal Docker services (trusted network assumption).
- No rate limiting in MVP (planned for API key migration).
- Ollama API has no authentication (localhost-only access assumed).
- No encryption at rest for PostgreSQL data (relies on host-level disk encryption).

## Supported Versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | Yes (current development) |
