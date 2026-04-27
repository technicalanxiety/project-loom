# Billing Module ownership

The Platform Team owns the Billing Module. Ownership here means the
Platform Team is on the on-call rotation for billing incidents,
reviews and approves any change touching billing tables, and
maintains the Billing Module's runbooks, dashboards, and SLOs.

The Platform Team maintains the Billing Module's public API,
the internal worker that reconciles invoice line items against
the Payment Service ledger, and the scheduled job that closes a
billing period at month end. They also maintain the migration
history for the Billing Module's tables and own the conversation
with finance whenever a chargeback or revenue-recognition question
needs an engineering answer.

When a question about the Billing Module shows up in another team's
channel — "who owns this?" — the answer is always the Platform
Team. The Platform Team is intentionally a small group and they
have explicitly decided not to spread Billing Module ownership
across multiple teams; the trade-off is concentrated context for a
domain where one wrong query can produce a customer-visible billing
mistake.
