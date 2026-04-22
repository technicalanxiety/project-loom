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

from loom_http import build_ssl_context
from schema_assertions import SchemaAssertionError, assert_schema

PARSER_VERSION = "claude_code_parser@0.4.0"
PARSER_SOURCE_SCHEMA = "claude_code_jsonl_v2"

# Pinned schema. Each session file is a newline-delimited sequence of event
# objects; every object carries a `type` and a `sessionId`. `timestamp` is
# present on most record types but not all (`ai-title` and `last-prompt`
# are metadata envelopes with no event time), so it is not part of the
# outer-envelope assertion. The parser still requires *some* record in
# each chunk to carry a timestamp so the episode's occurred_at field is
# populated — see iter_chunks() below.
REQUIRED_FIELDS = {
    "type": str,
    "sessionId": str,
}

# Per-episode byte cap. Long Claude Code sessions (multi-hour coding
# work) can easily produce 3-5 MB of JSONL per file, which blows past
# both the embedding model context window (nomic-embed-text: 8192
# tokens) and the extractor's practical window. The parser splits each
# session into chunks at record boundaries, capping each chunk at this
# many bytes.
#
# Why 4 KiB, not the nominal ~32 KiB for an 8192-token window: Claude
# Code's JSONL is dominated by assistant tool_use records containing
# escaped code diffs, tool-output blobs, and long JSON metadata. That
# content tokenizes at roughly 1 char/token (sometimes worse) because
# BPE tokenizers don't compress escaped JSON or base64 well. Observed
# empirically: 3.2 KiB chunks embed cleanly; 5.9 KiB+ chunks hit the
# 8192-token ceiling and return HTTP 400 "input length exceeds the
# context length". 4 KiB is the conservative floor that leaves room
# for the extraction prompt to sit alongside the content in gemma4:e4b's
# 8192-token window on a per-episode basis.
MAX_CHUNK_BYTES = 4 * 1024


def iter_session_files(session_dir: Path) -> Iterator[Path]:
    """Yield every ``*.jsonl`` file under ``session_dir`` recursively."""
    yield from sorted(session_dir.rglob("*.jsonl"))


def iter_chunks(path: Path) -> Iterator[tuple[str, str, int]]:
    """Yield ``(content, occurred_at, chunk_index)`` tuples for a session.

    Reads the JSONL file line-by-line (each line is one record),
    validates the pinned outer-envelope schema on every record, and
    groups records into chunks that fit under ``MAX_CHUNK_BYTES``.
    Chunks are split at record boundaries only — a single record is
    never divided — so each chunk remains valid JSONL and every
    downstream consumer (embedding, extraction) sees coherent records.

    Every chunk inherits its ``occurred_at`` from the first
    timestamp-bearing record within it. If a chunk contains no
    timestamp-bearing records (metadata-only, rare), it inherits from
    the previous chunk. If the very first chunk has no timestamp,
    we fail loud: the session would be entirely untimed and unusable.

    Raises:
        SchemaAssertionError: malformed JSON, missing required envelope
            fields, or a file that cannot produce a single timed chunk.
    """
    raw = path.read_text(encoding="utf-8")
    last_seen_ts: str | None = None
    buffer_lines: list[str] = []
    buffer_bytes: int = 0
    buffer_ts: str | None = None
    chunk_index: int = 0

    def flush() -> Iterator[tuple[str, str, int]]:
        nonlocal buffer_lines, buffer_bytes, buffer_ts, chunk_index
        if not buffer_lines:
            return
        # Prefer this chunk's own first timestamp; otherwise inherit
        # from the last timestamp we saw in any prior chunk.
        effective_ts = buffer_ts or last_seen_ts
        if effective_ts is None:
            raise SchemaAssertionError(
                f"{PARSER_SOURCE_SCHEMA}: {path} chunk {chunk_index} "
                f"has no timestamp and no prior chunk to inherit from"
            )
        content = "\n".join(buffer_lines) + "\n"
        yield content, effective_ts, chunk_index
        chunk_index += 1
        buffer_lines = []
        buffer_bytes = 0
        buffer_ts = None

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

        line_bytes = len(line.encode("utf-8")) + 1  # +1 for newline
        ts = record.get("timestamp")
        if isinstance(ts, str):
            last_seen_ts = ts
            if buffer_ts is None:
                buffer_ts = ts

        # If this line would overflow the chunk, flush first. A single
        # record that exceeds MAX_CHUNK_BYTES on its own is emitted as
        # its own oversized chunk — better to let the downstream
        # pipeline hit a context-window error on one record than to
        # silently drop data.
        if buffer_bytes + line_bytes > MAX_CHUNK_BYTES and buffer_lines:
            yield from flush()

        buffer_lines.append(line)
        buffer_bytes += line_bytes

    # Emit the final chunk.
    yield from flush()

    if chunk_index == 0:
        raise SchemaAssertionError(
            f"{PARSER_SOURCE_SCHEMA}: {path} contains no non-empty records"
        )


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
        with urllib.request.urlopen(req, timeout=30, context=build_ssl_context()) as resp:
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

    session_count = 0
    chunk_count = 0
    for path in iter_session_files(args.session_dir):
        rel = path.relative_to(args.session_dir)
        for content, occurred_at, chunk_index in iter_chunks(path):
            source_event_id = f"claude-code-bootstrap:{rel}:{chunk_index:05d}"
            post_episode(
                base_url=loom_url,
                token=loom_token,
                namespace=args.namespace,
                content=content,
                occurred_at=occurred_at,
                source_event_id=source_event_id,
            )
            chunk_count += 1
        session_count += 1

    print(
        f"done: posted {chunk_count} chunks across {session_count} sessions "
        f"from {args.session_dir}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
