#!/usr/bin/env python3
"""GitHub Copilot Chat bootstrap parser — stub.

GitHub does not currently publish a conversation export for Copilot Chat
in VS Code. Chat state is held in the editor's extension storage with
no stable export schema to assert against, and GitHub's existing data
exports (repository data, issue / PR comments, Copilot metrics) do not
include Copilot Chat transcripts.

This stub exists so the bootstrap catalog stays accurate and so the
path from ``docs/clients/github-copilot.md`` to the parser file resolves
cleanly. It exits non-zero when invoked.

If GitHub ships a Copilot Chat export in the future, fill this in with
a pinned schema and the standard bootstrap-parser shape — see
``claude_code_parser.py`` for the reference implementation.

Until then, the workable substitutes for historical Copilot coding
work are:

1. Live MCP capture via ``.vscode/mcp.json`` — see
   ``docs/clients/github-copilot.md``.
2. User-authored seed documents via ``cli/loom-seed.py`` describing
   the decisions / patterns / people from the historical work. This is
   Mode 1 (``user_authored_seed``), explicitly authored by you — not
   an LLM summary of past Copilot chats, which would violate the
   verbatim-content invariant (ADR-005).
"""
from __future__ import annotations

import sys


def main() -> int:
    print(
        "github_copilot_parser: GitHub does not ship a Copilot Chat "
        "conversation export. See docs/clients/github-copilot.md for the "
        "live-capture path (.vscode/mcp.json) and the user-authored seed "
        "fallback (cli/loom-seed.py).",
        file=sys.stderr,
    )
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
