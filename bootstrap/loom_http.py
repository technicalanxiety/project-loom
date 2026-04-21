"""Shared HTTP helpers for Loom bootstrap parsers.

Every parser POSTs to the same endpoint (`/api/learn`) with the same
auth (bearer token from `LOOM_TOKEN`) and the same TLS posture. This
module factors the pieces that would otherwise be copy-pasted six ways.

For now it exposes only :func:`build_ssl_context`. If the HTTP POST
wrapper ever ends up with client-specific differences beyond request
body shape, they should be added as parameters here rather than forked
back into each parser.
"""
from __future__ import annotations

import os
import ssl


def build_ssl_context() -> ssl.SSLContext | None:
    """Build the SSL context urllib should use on HTTPS requests.

    Returns ``None`` when ``LOOM_TLS_INSECURE`` is unset — urllib falls
    back to its default context, which verifies certificates against
    the system trust store. This is the correct posture for production
    deployments with a real certificate.

    Returns a permissive context when ``LOOM_TLS_INSECURE`` is set to
    ``1``, ``true``, or ``yes`` (case-insensitive). Intended for
    localhost development where Caddy ships a self-signed cert via its
    local CA. Leaving the env var set in production would silently
    disable certificate verification, which is why the gate is
    opt-in-per-run rather than a default.
    """
    insecure = os.environ.get("LOOM_TLS_INSECURE", "").lower()
    if insecure not in ("1", "true", "yes"):
        return None
    ctx = ssl.create_default_context()
    ctx.check_hostname = False
    ctx.verify_mode = ssl.CERT_NONE
    return ctx
