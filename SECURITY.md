# Security Policy

## Context before reporting

Loom is personal infrastructure, MIT-licensed, unmaintained for anyone other than the author. See [PROJECT-STANCE.md](PROJECT-STANCE.md). Security reports receive best-effort attention — **no SLA, no response guarantee, no formal disclosure process**. If you need a commercially-supported memory system, Loom is not it.

This document exists so that honest reports have somewhere to go and so that the threat model is on the record for anyone evaluating a fork.

## Reporting a vulnerability

If you discover a security vulnerability, please report it responsibly.

**Do NOT open a public GitHub issue for security vulnerabilities.**

Use GitHub's private vulnerability reporting if available on the repository, or email the author via the contact on [technicalanxiety.com](https://www.technicalanxiety.com). Include:

- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if you have one)

I'll look at it when I can. If the vulnerability affects my own deployment, it gets fixed promptly. If it only affects fork-specific configurations I don't run, I may acknowledge it and flag it in docs rather than patch upstream.

## Security Model

### Threat Model

Project Loom is designed as a **local-first, single-tenant tool** running on the author's (or your fork's) machine or within a trusted network. Multi-tenancy, SaaS hosting, and enterprise SSO are explicitly out of scope. The primary threat model assumes:

- **Trusted operator**: The person running Loom controls the machine and network.
- **Untrusted input**: Episode content ingested from external sources (chat exports, webhooks, vendor imports) may contain malicious content.
- **Local LLM**: Inference runs locally via Ollama. No data leaves the machine for LLM calls by default. If you configure Azure OpenAI as a fallback, that changes; your fork's threat model is your call.

### Content-authority threat

Loom's authority hierarchy — Episodes > Facts > Procedures — assumes episode `content` is verbatim. The most dangerous non-cryptographic attack surface is **LLM-reconstructed content entering as a live-capture episode** (see [ADR-005](docs/adr/005-verbatim-content-invariant.md)). Loom cannot detect this at runtime; it is prevented by shipped templates, MCP hardcoding of `ingestion_mode = live_mcp_capture`, and user discipline. If your fork changes any of those three layers, audit your authority model carefully.

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
