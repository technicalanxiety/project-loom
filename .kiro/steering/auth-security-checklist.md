---
description: Security checklist and API key design guidance for the auth module
inclusion: fileMatch
fileMatchPattern: "loom-engine/src/api/auth.rs"
---

# Auth Module Security Checklist

When modifying the auth module, verify:

## Must-Have

- [ ] Tokens/keys are compared using constant-time comparison (prevent timing attacks)
- [ ] Failed auth attempts are logged with request metadata (IP, timestamp) but NOT the submitted token
- [ ] Auth errors return generic messages — never reveal whether a key exists vs. is expired
- [ ] All auth middleware is applied before any route handler executes
- [ ] Bearer token extraction handles malformed Authorization headers gracefully

## API Key Design (Target)

- Keys are stored as SHA-256 hashes, never plaintext
- Key creation returns the raw key exactly once; it's never retrievable again
- Key metadata (created_at, last_used, expires_at, description) is stored alongside the hash
- Revoked keys return 401 immediately, not after other validation
- Rate limit state is keyed by API key hash, not by IP alone

## Testing

- Test with missing Authorization header → 401
- Test with malformed header (no "Bearer " prefix) → 401
- Test with invalid token → 401
- Test with expired key → 401
- Test with valid key → 200 + correct response
- Test rate limiting triggers correctly
