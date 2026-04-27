# Notification Service API surface

The Notification Service exposes a REST API for every other
internal service that needs to send a transactional message to a
user — order confirmations, password-reset emails, billing alerts,
and the like. The REST API has three resources: `/messages` for
single sends, `/batches` for fan-out, and `/templates` for managing
the rendered output.

The Notification Service implements the platform's standard
idempotency contract. Every POST to the REST API accepts an
`Idempotency-Key` header; if the same key arrives twice within
twenty-four hours, the second call returns the original response
without re-sending the message. That contract is the same one the
Payment Service implements, so the calling pattern is consistent
across the platform.

Authentication for the REST API is bearer-token based. The
Notification Service implements per-route scoping: a token granted
the `notifications:send` scope can hit `/messages` and `/batches`
but cannot create or modify templates. Template management requires
the `notifications:admin` scope, which is granted only to the
service-owner team's CI principal.
