# APIM authentication path

The APIM gateway is the single ingress point for every external
request hitting our platform. It uses the Auth Service to validate
bearer tokens before forwarding any request downstream — a request
that arrives without a valid token never makes it past APIM.

The Auth Service is deployed to the production Kubernetes cluster
alongside the rest of the platform tier. It exposes a small REST
endpoint that APIM calls on the hot path, and a token-introspection
endpoint that internal tooling uses for debugging.

When authentication fails for the APIM gateway, the cause is almost
always one of three things: the Auth Service has been restarted and
its in-memory key cache is cold, the upstream JWKS endpoint has
rotated keys faster than the Auth Service refreshes them, or APIM is
holding a stale OAuth client secret. The runbook checks Auth Service
logs first, then APIM's connection pool to Auth Service, then the
JWKS rotation timestamp.
