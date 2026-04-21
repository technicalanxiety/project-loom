#!/usr/bin/env python3
"""Claude.ai account-export bootstrap parser (Claude Desktop history).

Parses the ``conversations.json`` file produced by Claude.ai's account
data export (Settings → Privacy → Export data) and POSTs each
conversation verbatim to Loom's ``/api/learn`` as a single
``vendor_import`` episode.

Conversation-level (not message-level) granularity is deliberate: Loom's
offline pipeline runs entity/fact extraction downstream, and keeping one
episode per conversation preserves causal ordering for the extractor.

The schema is pinned at ``claude_ai_export_v2``. Claude.ai rotates field
names periodically — when that happens, this parser exits non-zero
naming the specific failing field. No best-effort fallback. See the
degraded-mode contract in bootstrap/README.md.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path

import urllib.error
import urllib.request

from schema_assertions import SchemaAssertionError, assert_schema

PARSER_VERSION = "claude_ai_parser@0.1.0"
PARSER_SOURCE_SCHEMA = "claude_ai_export_v2"

# Pinned schema. conversations.json is a list of conversation objects.
# Each conversation has a uuid, name, created_at, updated_at, and
# chat_messages[] array. Each message has a uuid, text, sender, and
# created_at. Minimum fields required for a meaningful episode.
REQUIRED_CONVERSATION_FIELDS = {
    "uuid": str,
    "name": str,
    "created_at": str,
    "updated_at": str,
    "chat_messages": list,
}

REQUIRED_MESSAGE_FIELDS = {
    "uuid": str,
    "text": str,
    "sender": str,
    "created_at": str,
}


def render_conversation(conversation: dict) -> str:
    """Render one conversation as a verbatim transcript.

    Each message is emitted in the form ``[ISO timestamp] sender: text``
    with a blank line between messages. The format is deterministic so
    content-hash dedup works across re-runs of the same export.

    No summarization, no field renaming, no reordering beyond the
    already-chronological ``chat_messages`` ordering the export
    guarantees. The output is the "verbatim excerpt" referenced in
    ADR-005 — a transformation of structure but not of meaning.
    """
    lines: list[str] = [f"# {conversation['name']}"]
    lines.append(f"conversation_uuid: {conversation['uuid']}")
    lines.append(f"created_at: {conversation['created_at']}")
    lines.append(f"updated_at: {conversation['updated_at']}")
    lines.append("")
    for msg in conversation["chat_messages"]:
        lines.append(f"[{msg['created_at']}] {msg['sender']}:")
        lines.append(msg["text"])
        lines.append("")
    return "\n".join(lines)


def post_episode(
    base_url: str,
    token: str,
    namespace: str,
    content: str,
    occurred_at: str,
    source_event_id: str,
) -> None:
    payload = {
        "content": content,
        "source": "claude-ai-bootstrap",
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
        body = exc.read().decode("utf-8", errors="replace")
        raise SystemExit(
            f"api/learn rejected {source_event_id} "
            f"(status={exc.code}): {body}"
        )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--export",
        type=Path,
        required=True,
        help="Path to conversations.json from the Claude.ai account export.",
    )
    parser.add_argument(
        "--namespace",
        required=True,
        help="Loom namespace to ingest these conversations into.",
    )
    args = parser.parse_args(argv)

    loom_url = os.environ.get("LOOM_URL")
    loom_token = os.environ.get("LOOM_TOKEN")
    if not loom_url or not loom_token:
        print("error: set LOOM_URL and LOOM_TOKEN in the environment", file=sys.stderr)
        return 2

    if not args.export.is_file():
        print(f"error: {args.export} is not a file", file=sys.stderr)
        return 2

    raw = args.export.read_text(encoding="utf-8")
    try:
        conversations = json.loads(raw)
    except json.JSONDecodeError as exc:
        print(
            f"{PARSER_SOURCE_SCHEMA}: {args.export} is not valid JSON: {exc}",
            file=sys.stderr,
        )
        return 1

    if not isinstance(conversations, list):
        print(
            f"{PARSER_SOURCE_SCHEMA}: expected top-level list, got {type(conversations).__name__}",
            file=sys.stderr,
        )
        return 1

    count = 0
    for conversation in conversations:
        try:
            assert_schema(conversation, REQUIRED_CONVERSATION_FIELDS, PARSER_SOURCE_SCHEMA)
            for msg in conversation["chat_messages"]:
                assert_schema(msg, REQUIRED_MESSAGE_FIELDS, PARSER_SOURCE_SCHEMA)
        except SchemaAssertionError as exc:
            print(str(exc), file=sys.stderr)
            return 1

        content = render_conversation(conversation)
        source_event_id = f"claude-ai-bootstrap:{conversation['uuid']}"
        post_episode(
            base_url=loom_url,
            token=loom_token,
            namespace=args.namespace,
            content=content,
            occurred_at=conversation["created_at"],
            source_event_id=source_event_id,
        )
        count += 1

    print(f"done: posted {count} conversations from {args.export}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
