#!/usr/bin/env python3
"""loom-seed — CLI tool for Mode 1 user-authored seed ingestion.

Reads markdown documents the user has authored describing their domain,
patterns, people, projects, or decisions, and POSTs each document to Loom's
``/api/learn`` endpoint with ``ingestion_mode=user_authored_seed``.

This is Mode 1 per ``docs/adr/004-ingestion-modes.md``. The user is the
author. An LLM may assist with *drafting* through interview-style prompting
— the user reviews and approves the content before running this tool — but
the content posted must be the user's own prose, not an LLM summary or
reconstruction. See ``docs/adr/005-verbatim-content-invariant.md`` for the
load-bearing discipline.

## Usage

```bash
export LOOM_URL="https://loom.yourdomain.com"
export LOOM_TOKEN="your-bearer-token"

# Seed a single file into a namespace
loom-seed.py --namespace my-project path/to/domain-overview.md

# Seed every .md in a directory
loom-seed.py --namespace my-project path/to/seeds/
```
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path
from typing import Iterable

import urllib.request
import urllib.error


SOURCE = "seed"


def iter_markdown(paths: Iterable[Path]) -> Iterable[Path]:
    """Yield every *.md file under the given paths.

    Accepts a mix of files and directories; directories are walked
    recursively. Non-markdown files are silently skipped so the tool is
    friendly to `seeds/` dirs that also contain supporting assets.
    """
    for p in paths:
        if p.is_file() and p.suffix.lower() == ".md":
            yield p
        elif p.is_dir():
            yield from sorted(p.rglob("*.md"))


def post_seed(
    base_url: str,
    token: str,
    namespace: str,
    content: str,
    source_event_id: str,
) -> None:
    """POST one markdown document as a user_authored_seed episode.

    Parser metadata is explicitly omitted — those fields are reserved for
    Mode 2 (vendor_import). The server rejects this combination if we get
    it wrong, which is the correct failure mode.
    """
    payload = {
        "content": content,
        "source": SOURCE,
        "namespace": namespace,
        "ingestion_mode": "user_authored_seed",
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
    parser = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "--namespace",
        required=True,
        help="Loom namespace to ingest these seed documents into.",
    )
    parser.add_argument(
        "paths",
        nargs="+",
        type=Path,
        help="One or more .md files or directories containing .md files.",
    )
    args = parser.parse_args(argv)

    loom_url = os.environ.get("LOOM_URL")
    loom_token = os.environ.get("LOOM_TOKEN")
    if not loom_url or not loom_token:
        print("error: set LOOM_URL and LOOM_TOKEN in the environment", file=sys.stderr)
        return 2

    found = list(iter_markdown(args.paths))
    if not found:
        print("error: no .md files found under the given paths", file=sys.stderr)
        return 2

    for md in found:
        content = md.read_text(encoding="utf-8")
        if not content.strip():
            print(f"skip empty {md}", file=sys.stderr)
            continue
        source_event_id = f"seed:{md.resolve()}"
        post_seed(
            base_url=loom_url,
            token=loom_token,
            namespace=args.namespace,
            content=content,
            source_event_id=source_event_id,
        )

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
