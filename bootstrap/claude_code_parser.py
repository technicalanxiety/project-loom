#!/usr/bin/env python3
"""Claude Code JSONL bootstrap parser.

Walks ``~/.claude/projects/*.jsonl`` session files, asserts the pinned
schema on each record, and POSTs each session verbatim to Loom's
``/api/learn`` endpoint as a single ``vendor_import`` episode.

Session-level rather than message-level granularity is deliberate: Loom's
online pipeline does entity/fact extraction downstream, and keeping one
episode per session preserves causal ordering for the extractor.

The JSONL format here is the shape Claude Code writes to disk locally; it
is not a published vendor contract. The assertion catches drift early.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path
from typing import Iterator

import urllib.request
import urllib.error

from schema_assertions import SchemaAssertionError, assert_schema

PARSER_VERSION = "claude_code_parser@0.1.0"
PARSER_SOURCE_SCHEMA = "claude_code_jsonl_v1"

# Pinned schema. Each session file is a newline-delimited sequence of event
# objects; every object must carry at minimum a type and a timestamp. Text
# lives under different keys depending on the event type, but the outer
# envelope is stable.
REQUIRED_FIELDS = {
    "type": str,
    "timestamp": str,
}


def iter_session_files(session_dir: Path) -> Iterator[Path]:
    """Yield every ``*.jsonl`` file under ``session_dir`` recursively."""
    yield from sorted(session_dir.rglob("*.jsonl"))


def read_session(path: Path) -> tuple[str, str]:
    """Read a JSONL session file and return (raw_text, occurred_at).

    Each record is validated against the pinned schema. The first record's
    timestamp becomes the session's `occurred_at`. Raw text is returned
    unchanged (no reformatting, no summarization) so the episode content
    reaching Loom is byte-exact.
    """
    raw = path.read_text(encoding="utf-8")
    occurred_at: str | None = None
    for lineno, line in enumerate(raw.splitlines(), start=1):
        if not line.strip():
            continue
        try:
            record = json.loads(line)
        except json.JSONDecodeError as exc:
            raise SchemaAssertionError(
                f"{PARSER_SOURCE_SCHEMA}: {path}:{lineno} is not valid JSON: {exc}"
            ) from exc
        assert_schema(record, REQUIRED_FIELDS, PARSER_SOURCE_SCHEMA)
        if occurred_at is None:
            occurred_at = record["timestamp"]
    if occurred_at is None:
        raise SchemaAssertionError(
            f"{PARSER_SOURCE_SCHEMA}: {path} contains no non-empty records"
        )
    return raw, occurred_at


def post_episode(
    base_url: str,
    token: str,
    namespace: str,
    content: str,
    occurred_at: str,
    source_event_id: str,
) -> None:
    """POST a single session to /api/learn as a vendor_import episode."""
    payload = {
        "content": content,
        "source": "claude-code-bootstrap",
        "namespace": namespace,
        "ingestion_mode": "vendor_import",
        "parser_version": PARSER_VERSION,
        "parser_source_schema": PARSER_SOURCE_SCHEMA,
        "occurred_at": occurred_at,
        "source_event_id": source_event_id,
    }
    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        f"{base_url.rstrip('/')}/api/learn",
        data=data,
        method="POST",
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            body = resp.read().decode("utf-8")
            print(f"ok {source_event_id} -> {body}")
    except urllib.error.HTTPError as exc:
        # 4xx errors surface validation failures from the server; re-raise
        # so the caller can decide to halt.
        body = exc.read().decode("utf-8", errors="replace")
        raise SystemExit(
            f"api/learn rejected {source_event_id} "
            f"(status={exc.code}): {body}"
        )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--session-dir",
        type=Path,
        default=Path.home() / ".claude" / "projects",
        help="Directory containing Claude Code session JSONL files "
        "(default: ~/.claude/projects)",
    )
    parser.add_argument(
        "--namespace",
        required=True,
        help="Loom namespace to ingest these sessions into.",
    )
    args = parser.parse_args(argv)

    loom_url = os.environ.get("LOOM_URL")
    loom_token = os.environ.get("LOOM_TOKEN")
    if not loom_url or not loom_token:
        print("error: set LOOM_URL and LOOM_TOKEN in the environment", file=sys.stderr)
        return 2

    if not args.session_dir.is_dir():
        print(f"error: {args.session_dir} is not a directory", file=sys.stderr)
        return 2

    count = 0
    for path in iter_session_files(args.session_dir):
        raw, occurred_at = read_session(path)
        source_event_id = f"claude-code-bootstrap:{path.relative_to(args.session_dir)}"
        post_episode(
            base_url=loom_url,
            token=loom_token,
            namespace=args.namespace,
            content=raw,
            occurred_at=occurred_at,
            source_event_id=source_event_id,
        )
        count += 1

    print(f"done: posted {count} sessions from {args.session_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
