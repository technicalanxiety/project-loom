"""Shared schema-assertion helper for Loom bootstrap parsers.

The assertion pattern is intentionally simple: every parser declares a
`REQUIRED_FIELDS` mapping of dotted paths to expected Python types, then
calls :func:`assert_schema` on each record before ingestion. Missing fields
and type mismatches raise :class:`SchemaAssertionError` with a precise
field-name message, which the parser propagates to stderr and exits on.

There is no "tolerant mode." See ``bootstrap/README.md`` for the
degraded-mode contract.
"""
from __future__ import annotations

from typing import Any, Iterable, Mapping


class SchemaAssertionError(Exception):
    """Raised when a required field is missing or has the wrong type.

    The message names the specific field so the user can act on it
    without inspecting the parser source.
    """


def _resolve_path(obj: Any, path: str) -> tuple[bool, Any]:
    """Walk a dotted path, descending into dicts and (for ``[]`` segments)
    into list elements.

    Returns ``(found, value)``. When the path includes ``[]``, each list
    element is checked; the first missing segment short-circuits to
    ``(False, None)``.
    """
    parts = path.split(".")
    current: Any = obj
    for part in parts:
        if part.endswith("[]"):
            key = part[:-2]
            if key:
                if not isinstance(current, Mapping) or key not in current:
                    return False, None
                current = current[key]
            if not isinstance(current, list):
                return False, None
            # Validate that every list element has the subsequent path segments.
            return True, current
        if not isinstance(current, Mapping) or part not in current:
            return False, None
        current = current[part]
    return True, current


def assert_schema(
    record: Mapping[str, Any],
    required_fields: Mapping[str, type],
    schema_name: str,
) -> None:
    """Assert that ``record`` matches the pinned schema.

    Args:
        record: The parsed export record (dict-like).
        required_fields: Mapping of dotted field paths to expected Python
            types. List elements are indicated by ``[]`` (e.g.
            ``conversations[].chat_messages[].text``); every element must
            satisfy the remaining path.
        schema_name: The schema identifier for error messages (e.g.
            ``claude_ai_export_v2``). Included verbatim in every error so
            dashboard parser-health views can group failures.

    Raises:
        SchemaAssertionError: The first failing field is reported;
            subsequent fields are not checked. Fail-fast is intentional —
            once the export's shape diverges, partial parsing is unsafe.
    """
    for path, expected_type in required_fields.items():
        _check_path(record, path, expected_type, schema_name)


def _check_path(
    obj: Any,
    path: str,
    expected_type: type,
    schema_name: str,
) -> None:
    parts = path.split(".")
    current: Any = obj
    traversed: list[str] = []
    for part in parts:
        traversed.append(part)
        if part.endswith("[]"):
            key = part[:-2]
            if key:
                if not isinstance(current, Mapping) or key not in current:
                    raise SchemaAssertionError(
                        f"{schema_name}: missing {'.'.join(traversed)}"
                    )
                current = current[key]
            if not isinstance(current, list):
                raise SchemaAssertionError(
                    f"{schema_name}: expected list at {'.'.join(traversed)}, "
                    f"got {type(current).__name__}"
                )
            remaining = parts[parts.index(part) + 1:]
            if not remaining:
                if not isinstance(current, list):
                    raise SchemaAssertionError(
                        f"{schema_name}: expected list at {'.'.join(traversed)}"
                    )
                continue
            remaining_path = ".".join(remaining)
            for element in current:
                _check_path(element, remaining_path, expected_type, schema_name)
            return
        if not isinstance(current, Mapping) or part not in current:
            raise SchemaAssertionError(
                f"{schema_name}: missing {'.'.join(traversed)}"
            )
        current = current[part]
    if not isinstance(current, expected_type):
        raise SchemaAssertionError(
            f"{schema_name}: expected {expected_type.__name__} at {path}, "
            f"got {type(current).__name__}"
        )


def iter_required_fields(fields: Iterable[str]) -> Iterable[str]:
    """Order-preserving uniqueness pass over required-field paths."""
    seen: set[str] = set()
    for f in fields:
        if f in seen:
            continue
        seen.add(f)
        yield f
