# Event snapshots

Every `#[contractevent]` struct in `contracts/stream/src/events.rs` is part of
the contract's public event ABI. Because the contract keeps no per-party index,
these events are the only way an indexer or the TypeScript SDK learns that a
stream exists or that its state moved — so a silent change to an event's topic
namespace, topic arity, field order, or field types is a breaking interface
change, not a cosmetic one.

Issue #1701: nothing asserted that the committed fixtures actually covered
*every* emitted event type, so a new event (or a payload edit) could ship with
no fixture pinning its shape. `script/check_event_snapshots.py` closes that gap.

## How it works

`script/check_event_snapshots.py` statically parses every `#[contractevent]`
struct out of `events.rs` and derives the canonical schema for each one:

* the struct name and its snake_case `topic[0]` namespace symbol, and
* the ordered field list, marking each field as `#[topic]` or payload.

It then requires exactly one fixture per event type under
`tests/fixtures/event_snapshots/events/<snake_case_name>.json` and compares the
fixture against the parsed schema. The gate fails (exit 1) when:

* an event type has no fixture,
* a fixture has no matching event struct (an orphan left behind by a rename or
  removal),
* a fixture is not readable JSON, or
* a fixture no longer matches the payload — a renamed, retyped, reordered, or
  re-topicked field, or an added/removed field.

The fixture format is intentionally the parsed schema, so the file records the
frozen shape rather than a sampled value:

```json
{
  "event": "Withdrawn",
  "topic": "withdrawn",
  "fields": [
    { "name": "stream_id", "type": "u64", "topic": true },
    { "name": "recipient", "type": "Address", "topic": true },
    { "name": "amount", "type": "i128", "topic": false }
  ]
}
```

## Regenerating fixtures

Fixtures are generated from the source of truth (`events.rs`); do not hand-edit
them. After an intentional event change, regenerate and commit the result:

```bash
python3 script/check_event_snapshots.py --update
```

`--update` writes the canonical schema for every event and prunes fixtures whose
event no longer exists. To verify without writing:

```bash
python3 script/check_event_snapshots.py
```

Exit codes: `0` covered and matching, `1` coverage or payload drift, `2` broken
input (`events.rs` missing or unparsable).

## Where it runs

The gate runs as a dedicated hard-failure step in CI's `docs-alignment-check`
job, and `tests/test_check_event_snapshots.py` additionally runs it against the
repository as committed — so adding a new event type without a fixture fails CI
through both paths.

The step is declared `if: always()`. The job also runs the Python suite and the
doc-alignment gates, which can fail for reasons unrelated to events; without it
the runner would skip this gate on those runs and the event verdict would go
unreported.

Because every event is a `#[contractevent]` struct, this gate covers the emitted
set without executing the contract. The Rust-side `test::events` module remains
the runtime check for topic-name uniqueness and arity.
