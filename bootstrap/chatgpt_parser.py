#!/usr/bin/env python3
"""ChatGPT Data Controls export bootstrap parser.

Parses the ``conversations.json`` file produced by ChatGPT's Data
Controls export (Settings → Data Controls → Export data) and POSTs
each conversation as one ``vendor_import`` episode.

ChatGPT's export stores conversations as a tree (to preserve edit /
regeneration branches) rather than a linear transcript. This parser
walks from ``current_node`` back through ``parent`` links to reconstruct
the canonical linear path — i.e. the conversation as it existed when
the user last interacted with it. Edit-branches (paths not taken) are
dropped; extraction on them would attribute "decisions" that never
actually happened.

The schema is pinned at ``chatgpt_export_v1``. OpenAI rotates field
names periodically — when that happens, this parser exits non-zero
naming the specific failing field. See bootstrap/README.md for the
degraded-mode contract.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path

import urllib.error
import urllib.request

from loom_http import build_ssl_context
from schema_assertions import SchemaAssertionError, assert_schema

PARSER_VERSION = "chatgpt_parser@0.1.0"
PARSER_SOURCE_SCHEMA = "chatgpt_export_v1"

# Pinned schema. conversations.json is a list of conversation objects.
# Each conversation has a title, create_time (float epoch seconds), a
# mapping object keyed by node id, and a current_node pointer into that
# mapping. Each node has id, parent (nullable for the root), children
# (list of node ids), and a message (nullable — branching nodes can be
# messageless).
REQUIRED_CONVERSATION_FIELDS = {
    "title": str,
    "create_time": float,
    "mapping": dict,
    "current_node": str,
}


def walk_current_path(conversation: dict) -> list[dict]:
    """Walk parent pointers from current_node back to root.

    Returns the nodes in chronological order (root first, leaf last).
    Messageless nodes (typically the synthetic root, or branch points)
    are included in the walk but filtered from the final output.

    Raises:
        SchemaAssertionError: parent pointer points at a missing node
            or the walk exceeds a safety bound.
    """
    mapping = conversation["mapping"]
    current_id = conversation["current_node"]

    # Bound the walk — pathological exports with cycles have been
    # observed on hand-edited conversations.json files.
    max_walk = 10_000

    chain: list[dict] = []
    seen: set[str] = set()
    node_id: str | None = current_id
    for _ in range(max_walk):
        if node_id is None:
            break
        if node_id in seen:
            raise SchemaAssertionError(
                f"{PARSER_SOURCE_SCHEMA}: cycle detected at node {node_id}"
            )
        seen.add(node_id)
        node = mapping.get(node_id)
        if node is None:
            raise SchemaAssertionError(
                f"{PARSER_SOURCE_SCHEMA}: mapping missing node {node_id}"
            )
        chain.append(node)
        node_id = node.get("parent")
    else:
        raise SchemaAssertionError(
            f"{PARSER_SOURCE_SCHEMA}: walk exceeded {max_walk} nodes"
        )

    chain.reverse()
    return chain


def render_message(node: dict) -> str | None:
    """Render one tree node as a transcript line. Returns None if the
    node has no renderable message content (synthetic root, branch
    point, or system-only node).

    The output format is ``[ISO timestamp] role: text`` to match the
    other parsers. Multi-part content is joined with blank lines.
    """
    msg = node.get("message")
    if not msg:
        return None

    author = msg.get("author") or {}
    role = author.get("role", "unknown")
    if role == "system":
        # ChatGPT's synthetic system messages are not user-authored
        # content and would pollute the episode. Skip.
        return None

    content = msg.get("content") or {}
    parts = content.get("parts") or []
    # Parts can be strings or objects (e.g. image references). We only
    # include string parts — vendor-import is for textual transcripts.
    text_parts = [p for p in parts if isinstance(p, str) and p.strip()]
    if not text_parts:
        return None

    create_time = msg.get("create_time")
    if isinstance(create_time, (int, float)):
        iso_ts = datetime.fromtimestamp(create_time, tz=timezone.utc).isoformat()
    else:
        iso_ts = "unknown"

    body = "\n\n".join(text_parts)
    return f"[{iso_ts}] {role}:\n{body}"


def render_conversation(conversation: dict) -> str:
    """Render the current-node path of one conversation as a verbatim
    transcript. Only the current branch is rendered; edit / regeneration
    siblings are dropped.
    """
    lines: list[str] = [f"# {conversation['title']}"]
    create_ts = datetime.fromtimestamp(
        conversation["create_time"], tz=timezone.utc
    ).isoformat()
    lines.append(f"create_time: {create_ts}")
    lines.append(f"current_node: {conversation['current_node']}")
    lines.append("")

    path = walk_current_path(conversation)
    for node in path:
        rendered = render_message(node)
        if rendered is not None:
            lines.append(rendered)
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
        "source": "chatgpt-bootstrap",
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
        help="Path to conversations.json from the ChatGPT Data Controls export.",
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
            f"{PARSER_SOURCE_SCHEMA}: expected top-level list, "
            f"got {type(conversations).__name__}",
            file=sys.stderr,
        )
        return 1

    count = 0
    for idx, conversation in enumerate(conversations):
        try:
            assert_schema(
                conversation, REQUIRED_CONVERSATION_FIELDS, PARSER_SOURCE_SCHEMA
            )
            content = render_conversation(conversation)
        except SchemaAssertionError as exc:
            print(str(exc), file=sys.stderr)
            return 1

        # ChatGPT exports do not always include a stable conversation id
        # — the mapping keys are stable across re-exports but not
        # always present as a top-level field. Use current_node as the
        # dedup key; content-hash catches re-exports with the same tree
        # state, and source_event_id catches re-exports with the same
        # final leaf.
        source_event_id = f"chatgpt-bootstrap:{conversation['current_node']}"
        occurred_at = datetime.fromtimestamp(
            conversation["create_time"], tz=timezone.utc
        ).isoformat()
        post_episode(
            base_url=loom_url,
            token=loom_token,
            namespace=args.namespace,
            content=content,
            occurred_at=occurred_at,
            source_event_id=source_event_id,
        )
        count += 1

    print(f"done: posted {count} conversations from {args.export}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
