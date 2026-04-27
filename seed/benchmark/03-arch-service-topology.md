# Payment Service topology

The Payment Service owns every payment-related write in the platform
— authorizations, captures, refunds, chargebacks, and the ledger
entries that back them. No other service is permitted to write to
the payment tables; consumers go through the Payment Service's
REST API or its event stream.

The Payment Service communicates with the Stripe Gateway over
mTLS, using a dedicated egress proxy that pins Stripe's certificate
chain. Stripe Gateway is a thin adapter the platform team owns; it
translates Stripe's webhook payloads into the platform's internal
event schema and translates the Payment Service's outbound calls
into Stripe's REST shape.

The Payment Service owns the canonical idempotency keys that flow
through both directions. When the Stripe Gateway replays a webhook
or retries an outbound call, those keys are what stop a duplicate
charge from landing on a customer's card.
