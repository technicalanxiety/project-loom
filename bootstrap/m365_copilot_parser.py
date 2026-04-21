#!/usr/bin/env python3
"""Microsoft 365 Copilot Purview audit export parser.

Parses a Microsoft Purview Content Search / eDiscovery export bundle
containing M365 Copilot interaction audit records, and POSTs each
interaction as one ``vendor_import`` episode.

The schema is pinned at ``m365_copilot_audit_v1``. The Purview Copilot
interaction audit schema uses field names like ``CopilotEventData``
containing ``AppHost``, ``Contexts``, ``Messages``, and
``ModelTransparencyMessages`` alongside the standard ``CreationTime``,
``UserId``, and ``Id`` outer envelope. This parser asserts the core
fields on every record; when Microsoft rotates the audit schema (which
happens under the GA cadence), it fails loud naming the specific failing
field. See bootstrap/README.md for the degraded-mode contract.

The input can be either:

- A directory produced by Purview export (treated as a bundle; every
  ``*.json`` and ``*.jsonl`` file under it is scanned).
- A single ``*.json`` (array of records) or ``*.jsonl`` (one record per
  line) file.

Only records where ``Operation == "CopilotInteraction"`` are ingested;
other M365 audit rows are ignored.
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path
from typing import Iterator

import urllib.error
import urllib.request

from loom_http import build_ssl_context
from schema_assertions import SchemaAssertionError, assert_schema

PARSER_VERSION = "m365_copilot_parser@0.1.0"
PARSER_SOURCE_SCHEMA = "m365_copilot_audit_v1"

# Outer audit envelope. CopilotInteraction rows carry a CopilotEventData
# object with the transcript. Other operations are skipped before schema
# assertion.
REQUIRED_AUDIT_FIELDS = {
    "Id": str,
    "CreationTime": str,
    "UserId": str,
    "Operation": str,
    "CopilotEventData": dict,
}

REQUIRED_EVENT_DATA_FIELDS = {
    "AppHost": str,
    "Messages": list,
}


def iter_records(source: Path) -> Iterator[tuple[Path, int, dict]]:
    """Yield ``(path, lineno, record)`` from a file or bundle directory.

    For ``*.jsonl`` files, each non-empty line is parsed. For
    ``*.json`` files, the top-level value must be a list — any other
    shape is a schema violation.
    """
    if source.is_dir():
        files = sorted(
            list(source.rglob("*.json")) + list(source.rglob("*.jsonl"))
        )
    else:
        files = [source]

    for path in files:
        if path.suffix.lower() == ".jsonl":
            raw = path.read_text(encoding="utf-8")
            for lineno, line in enumerate(raw.splitlines(), start=1):
                if not line.strip():
                    continue
                try:
                    yield path, lineno, json.loads(line)
                except json.JSONDecodeError as exc:
                    raise SchemaAssertionError(
                        f"{PARSER_SOURCE_SCHEMA}: {path}:{lineno} "
                        f"is not valid JSON: {exc}"
                    ) from exc
        else:
            raw = path.read_text(encoding="utf-8")
            try:
                records = json.loads(raw)
            except json.JSONDecodeError as exc:
                raise SchemaAssertionError(
                    f"{PARSER_SOURCE_SCHEMA}: {path} is not valid JSON: {exc}"
                ) from exc
            if not isinstance(records, list):
                raise SchemaAssertionError(
                    f"{PARSER_SOURCE_SCHEMA}: {path} must be a top-level "
                    f"list, got {type(records).__name__}"
                )
            for idx, record in enumerate(records):
                yield path, idx, record


def render_interaction(record: dict) -> str:
    """Render one CopilotInteraction audit record as a verbatim
    transcript.

    The Messages array contains the user turn and the Copilot response
    with timestamps and roles. AppHost identifies which Copilot surface
    (Teams, Outlook, SharePoint, etc.). We emit the full envelope in a
    deterministic format so content-hash dedup works across re-runs of
    the same Purview export.
    """
    event_data = record["CopilotEventData"]

    lines: list[str] = [f"# M365 Copilot interaction {record['Id']}"]
    lines.append(f"user_id: {record['UserId']}")
    lines.append(f"creation_time: {record['CreationTime']}")
    lines.append(f"app_host: {event_data['AppHost']}")
    contexts = event_data.get("Contexts")
    if isinstance(contexts, list) and contexts:
        lines.append("contexts:")
        for ctx in contexts:
            lines.append(f"  - {json.dumps(ctx, sort_keys=True)}")
    lines.append("")

    for msg in event_data["Messages"]:
        # Purview's message schema has varied across GA cadences; the
        # parser only requires ``Messages`` to be a list and leaves
        # individual message shape flexible. Emit each message as a
        # pretty-printed JSON block so any structure the export carries
        # survives to the extractor without loss.
        lines.append(json.dumps(msg, indent=2, sort_keys=True))
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
        "source": "m365-copilot-bootstrap",
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
        help="Path to a single Purview export file "
        "(.json array or .jsonl). Mutually exclusive with --export-dir.",
    )
    parser.add_argument(
        "--export-dir",
        type=Path,
        help="Path to a Purview export bundle directory. Every .json "
        "and .jsonl file under it is scanned.",
    )
    parser.add_argument(
        "--namespace",
        required=True,
        help="Loom namespace to ingest these interactions into.",
    )
    args = parser.parse_args(argv)

    loom_url = os.environ.get("LOOM_URL")
    loom_token = os.environ.get("LOOM_TOKEN")
    if not loom_url or not loom_token:
        print("error: set LOOM_URL and LOOM_TOKEN in the environment", file=sys.stderr)
        return 2

    if bool(args.export) == bool(args.export_dir):
        print(
            "error: specify exactly one of --export or --export-dir",
            file=sys.stderr,
        )
        return 2

    source = args.export or args.export_dir
    if args.export and not source.is_file():
        print(f"error: {source} is not a file", file=sys.stderr)
        return 2
    if args.export_dir and not source.is_dir():
        print(f"error: {source} is not a directory", file=sys.stderr)
        return 2

    count = 0
    skipped = 0
    try:
        for path, lineno, record in iter_records(source):
            if not isinstance(record, dict):
                raise SchemaAssertionError(
                    f"{PARSER_SOURCE_SCHEMA}: {path}:{lineno} "
                    f"expected object, got {type(record).__name__}"
                )
            operation = record.get("Operation")
            if operation != "CopilotInteraction":
                skipped += 1
                continue
            assert_schema(record, REQUIRED_AUDIT_FIELDS, PARSER_SOURCE_SCHEMA)
            assert_schema(
                record["CopilotEventData"],
                REQUIRED_EVENT_DATA_FIELDS,
                PARSER_SOURCE_SCHEMA,
            )

            content = render_interaction(record)
            source_event_id = f"m365-copilot-bootstrap:{record['Id']}"
            post_episode(
                base_url=loom_url,
                token=loom_token,
                namespace=args.namespace,
                content=content,
                occurred_at=record["CreationTime"],
                source_event_id=source_event_id,
            )
            count += 1
    except SchemaAssertionError as exc:
        print(str(exc), file=sys.stderr)
        return 1

    print(
        f"done: posted {count} CopilotInteraction records "
        f"(skipped {skipped} non-Copilot audit rows)"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
