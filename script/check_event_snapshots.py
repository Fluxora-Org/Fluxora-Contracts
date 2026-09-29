#!/usr/bin/env python3
"""Event snapshot coverage gate (issue #1701).

Every ``#[contractevent]`` struct declared in
``contracts/stream/src/events.rs`` is part of the contract's public event ABI.
Once an indexer or the TypeScript SDK decodes an event, its topic namespace,
topic arity, field order and field types are frozen. Nothing used to assert
that the committed event fixtures actually covered every emitted event type, so
an event could be added — or a payload field renamed, retyped or reordered —
without a checked-in fixture noticing.

This script closes that gap:

* It statically parses every ``#[contractevent]`` struct in ``events.rs`` and
  derives the canonical schema for that event (name, ``topic[0]`` symbol, and
  the ordered field list, marking which fields are ``#[topic]``).
* It requires one fixture per event type under
  ``tests/fixtures/event_snapshots/events/<snake_case_name>.json``.
* It fails (exit 1) when an event has no fixture, when a fixture has no
  matching event, when a fixture is unreadable, or when the fixture no longer
  matches the parsed payload shape.

Regenerating fixtures
---------------------

Fixtures are generated from the source of truth (``events.rs``), never
hand-edited::

    python3 script/check_event_snapshots.py --update

That writes the canonical schema for every event and prunes fixtures whose
event no longer exists. Commit the result alongside the source change.

Exit codes
----------

* ``0`` — every event has a fixture and every fixture matches.
* ``1`` — coverage or payload drift detected.
* ``2`` — broken input (``events.rs`` missing or unparsable).
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# Source of truth: the event struct declarations.
EVENTS_RS = REPO_ROOT / "contracts" / "stream" / "src" / "events.rs"

# One fixture per event type, named after the snake_case topic[0] symbol.
FIXTURES_DIR = REPO_ROOT / "tests" / "fixtures" / "event_snapshots" / "events"

_STRUCT_ATTR_RE = re.compile(r"^\s*#\[contractevent\]\s*$")
_STRUCT_OPEN_RE = re.compile(r"^\s*pub struct\s+(\w+)\s*\{\s*$")
_STRUCT_CLOSE_RE = re.compile(r"^\s*\}\s*$")
_FIELD_RE = re.compile(r"^pub\s+(\w+)\s*:\s*(.+?)\s*,?\s*$")
_TOPIC_ATTR = "#[topic]"


def snake_case(name: str) -> str:
    """Convert a struct name to the snake_case ``topic[0]`` symbol.

    Mirrors the conversion the soroban SDK applies to ``#[contractevent]``
    structs, so ``StreamCreated`` -> ``stream_created`` and ``TtlExtended`` ->
    ``ttl_extended``.
    """
    step_1 = re.sub(r"(.)([A-Z][a-z]+)", r"\1_\2", name)
    step_2 = re.sub(r"([a-z0-9])([A-Z])", r"\1_\2", step_1)
    return step_2.lower()


@dataclass(frozen=True)
class EventField:
    """One field of an event struct, in declaration order."""

    name: str
    type: str
    topic: bool


@dataclass(frozen=True)
class EventSchema:
    """The frozen shape of one emitted event type."""

    name: str
    topic: str
    fields: tuple[EventField, ...]

    @property
    def topic_fields(self) -> tuple[EventField, ...]:
        return tuple(field for field in self.fields if field.topic)

    @property
    def data_fields(self) -> tuple[EventField, ...]:
        return tuple(field for field in self.fields if not field.topic)

    def to_json(self) -> dict:
        """Canonical fixture representation (stable key order)."""
        return {
            "event": self.name,
            "topic": self.topic,
            "fields": [
                {"name": field.name, "type": field.type, "topic": field.topic}
                for field in self.fields
            ],
        }


def parse_events(source: str) -> list[EventSchema]:
    """Parse every ``#[contractevent]`` struct out of ``events.rs`` source."""
    lines = source.splitlines()
    schemas: list[EventSchema] = []
    index = 0

    while index < len(lines):
        if not _STRUCT_ATTR_RE.match(lines[index]):
            index += 1
            continue

        # The struct opening line follows the attribute (possibly after other
        # attributes or blank lines).
        open_index = index + 1
        while open_index < len(lines) and not _STRUCT_OPEN_RE.match(lines[open_index]):
            open_index += 1
        if open_index >= len(lines):
            break

        match = _STRUCT_OPEN_RE.match(lines[open_index])
        assert match is not None
        name = match.group(1)

        fields: list[EventField] = []
        pending_topic = False
        body_index = open_index + 1
        while body_index < len(lines) and not _STRUCT_CLOSE_RE.match(lines[body_index]):
            stripped = lines[body_index].strip()
            if stripped == _TOPIC_ATTR:
                pending_topic = True
            elif not stripped or stripped.startswith("//") or stripped.startswith("#["):
                # Blank line, doc comment, or an unrelated attribute.
                pass
            else:
                field_match = _FIELD_RE.match(stripped)
                if field_match is not None:
                    fields.append(
                        EventField(
                            name=field_match.group(1),
                            type=field_match.group(2),
                            topic=pending_topic,
                        )
                    )
                    pending_topic = False
            body_index += 1

        schemas.append(EventSchema(name=name, topic=snake_case(name), fields=tuple(fields)))
        index = body_index

    return schemas


def _fixture_path(fixtures_dir: Path, topic: str) -> Path:
    return fixtures_dir / f"{topic}.json"


def _describe_drift(topic: str, expected: dict, actual: object) -> str:
    """Return a human-readable reason a fixture no longer matches its event."""
    if not isinstance(actual, dict):
        return f"{topic}.json: fixture must be a JSON object"

    if actual.get("event") != expected["event"]:
        return (
            f"{topic}.json: event name changed "
            f"(fixture={actual.get('event')!r}, code={expected['event']!r})"
        )

    actual_fields = actual.get("fields")
    expected_fields = expected["fields"]
    if actual_fields == expected_fields:
        # Difference must be a stray/missing key outside the schema fields.
        extra = sorted(set(actual) - set(expected))
        missing = sorted(set(expected) - set(actual))
        return (
            f"{topic}.json: fixture keys drifted "
            f"(missing={missing}, unexpected={extra})"
        )

    if not isinstance(actual_fields, list):
        return f"{topic}.json: 'fields' must be a list"

    reasons: list[str] = []
    length = max(len(actual_fields), len(expected_fields))
    for position in range(length):
        old = actual_fields[position] if position < len(actual_fields) else None
        new = expected_fields[position] if position < len(expected_fields) else None
        if old != new:
            reasons.append(f"  field[{position}]: fixture={old} code={new}")
    return f"{topic}.json: payload shape changed\n" + "\n".join(reasons)


def check(schemas: list[EventSchema], fixtures_dir: Path) -> int:
    """Verify one fixture per event and require an exact payload match."""
    expected = {schema.topic: schema for schema in schemas}
    problems: list[str] = []

    if not fixtures_dir.is_dir():
        problems.append(
            f"missing fixture directory {fixtures_dir}; "
            "run `python3 script/check_event_snapshots.py --update`"
        )
        for schema in schemas:
            problems.append(f"  no fixture for event `{schema.name}` (topic `{schema.topic}`)")
    else:
        actual_files = {path.stem: path for path in sorted(fixtures_dir.glob("*.json"))}

        for schema in schemas:
            if schema.topic in actual_files:
                continue
            problems.append(
                f"{schema.topic}.json: fixture missing for event `{schema.name}`"
            )

        for topic in sorted(set(actual_files) - set(expected)):
            problems.append(
                f"{topic}.json: fixture has no matching #[contractevent] struct"
            )

        for topic, schema in expected.items():
            path = actual_files.get(topic)
            if path is None:
                continue
            try:
                actual = json.loads(path.read_text(encoding="utf-8"))
            except (OSError, ValueError) as error:
                problems.append(f"{topic}.json: fixture is not readable JSON ({error})")
                continue
            if actual != schema.to_json():
                problems.append(_describe_drift(topic, schema.to_json(), actual))

    if problems:
        print("Event snapshot gate FAILED:")
        for problem in problems:
            print(f"  - {problem}")
        print(
            "\nRegenerate with `python3 script/check_event_snapshots.py --update` "
            "and commit the result, or restore the fixture if the payload change "
            "was not intended."
        )
        return 1

    print(f"Event snapshot gate OK: {len(schemas)} event type(s) covered by a fixture.")
    return 0


def update(schemas: list[EventSchema], fixtures_dir: Path) -> int:
    """Write the canonical fixture for every event and prune stale fixtures."""
    fixtures_dir.mkdir(parents=True, exist_ok=True)
    expected = {schema.topic for schema in schemas}

    written = 0
    for schema in schemas:
        path = _fixture_path(fixtures_dir, schema.topic)
        path.write_text(
            json.dumps(schema.to_json(), indent=2) + "\n",
            encoding="utf-8",
        )
        written += 1

    pruned: list[str] = []
    for path in sorted(fixtures_dir.glob("*.json")):
        if path.stem not in expected:
            path.unlink()
            pruned.append(path.name)

    print(f"Wrote {written} event fixture(s) to {fixtures_dir}.")
    if pruned:
        print(f"Pruned {len(pruned)} stale fixture(s): {', '.join(pruned)}")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--update",
        action="store_true",
        help="regenerate every fixture from events.rs and prune stale ones",
    )
    parser.add_argument(
        "--events",
        default=str(EVENTS_RS),
        help="path to events.rs (default: %(default)s)",
    )
    parser.add_argument(
        "--fixtures",
        default=str(FIXTURES_DIR),
        help="fixture directory (default: %(default)s)",
    )
    args = parser.parse_args(argv)

    events_path = Path(args.events)
    if not events_path.is_file():
        print(f"ERROR: events source not found: {events_path}", file=sys.stderr)
        return 2

    schemas = parse_events(events_path.read_text(encoding="utf-8"))
    if not schemas:
        print(
            f"ERROR: no #[contractevent] structs parsed from {events_path}",
            file=sys.stderr,
        )
        return 2

    seen: dict[str, str] = {}
    for schema in schemas:
        if schema.topic in seen:
            print(
                f"ERROR: topic collision: `{schema.name}` and `{seen[schema.topic]}` "
                f"both map to `{schema.topic}`",
                file=sys.stderr,
            )
            return 2
        seen[schema.topic] = schema.name

    fixtures_dir = Path(args.fixtures)
    if args.update:
        return update(schemas, fixtures_dir)
    return check(schemas, fixtures_dir)


if __name__ == "__main__":
    sys.exit(main())
