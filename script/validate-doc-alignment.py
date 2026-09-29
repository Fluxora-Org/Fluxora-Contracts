#!/usr/bin/env python3
"""Validate that documentation aligns with the stream contract source.

Blocking CI gate (issue #1865). The check fails in both directions:

  1. Missing-from-docs: the contract exposes an ABI entry point that is not
     documented in docs/ABI.md.
  2. Documented-but-nonexistent: docs/ABI.md documents an entry point that no
     longer exists in the contract (removed or renamed).

The contract surface is the set of ``pub fn`` signatures inside
``#[contractimpl]`` blocks of ``contracts/stream/src/lib.rs``, minus the
intentional non-ABI entries in ``AUDIT_ENTRYPOINT_ALLOWLIST`` and internal
helpers (leading underscore). The documentation surface is the ``## Entry
points`` section of ``docs/ABI.md`` (summary table rows and sub-headings that
name a function).

Baseline
--------
Gaps that predate the gate are recorded in ``script/doc-alignment-baseline.json``
with a short reason each. The baseline is a shrink-only ratchet:

  - a gap that is not baselined fails CI (so new gaps must be fixed or
    explicitly baselined in the same PR),
  - a baseline entry whose gap no longer exists fails CI (stale entries must
    be removed; the baseline cannot contain fiction),
  - consequently the baseline can only shrink. Growing it is a deliberate,
    reviewable edit to the JSON file (routed to maintainers via CODEOWNERS),
    and is only possible when a real new gap exists.

The ``docs/audit.md`` entrypoint-table drift check is unchanged: CI also
diffs the lib.rs surface against that table separately.

Exit codes:
  0  aligned (any remaining gaps are baselined)
  1  misalignment: unbaselined gap(s) and/or stale baseline entries
  2  broken inputs: lib.rs or docs/ABI.md missing, no entry points found,
     or an unreadable/malformed baseline

An earlier version of this script printed warnings for entrypoint gaps and
always exited 0, and its ``docs/streaming.md`` / ``docs/error.md`` checks
silently skipped files that do not exist in this repository. Those checks are
gone; ``docs/ABI.md`` is the real entrypoint documentation and error-table
alignment is enforced by ``script/check-discriminant-collisions.py``.
"""

import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
LIB_RS = REPO_ROOT / "contracts" / "stream" / "src" / "lib.rs"
ABI_MD = REPO_ROOT / "docs" / "ABI.md"
BASELINE_PATH = REPO_ROOT / "script" / "doc-alignment-baseline.json"

# Intentional non-ABI entries that are documented but not public entrypoints.
AUDIT_ENTRYPOINT_ALLOWLIST = {"upgrade", "compute_keeper_fee_split"}

# A gap is one misalignment between the contract surface and the docs.
MISSING_DOC_PREFIX = "missing-doc:"
GHOST_DOC_PREFIX = "ghost-doc:"

_HEADING_RE = re.compile(r"^(#{2,6})\s+(.*)$")
_ENTRY_POINTS_SECTION_RE = re.compile(r"^##\s+Entry points\s*$", re.IGNORECASE)
_BACKTICK_RE = re.compile(r"`([^`]+)`")
# A backticked span counts as a function reference when it starts with a
# snake_case identifier, optionally followed by a parameter list.
_FN_SPAN_RE = re.compile(r"^\s*([a-z_][a-z0-9_]*)\s*(\(|$)")
# Headings separate the entry point name from prose with an em-dash:
#   #### `resume(stream_id)` — un-pause and fold the paused interval in
# Only the part before the em-dash may contribute documented names, so
# mentions of fields/functions in heading prose do not count as documentation.
_HEADING_NAME_SPLIT_RE = re.compile(r"\s+—\s+")


class BaselineError(ValueError):
    """The baseline file exists but cannot be parsed/trusted."""


def extract_contractimpl_pub_fns(source: str) -> list[str]:
    """Extract public function names from ``#[contractimpl]`` blocks.

    Tracks brace depth so ``pub fn``s outside the contractimpl blocks are not
    picked up; comment lines are ignored so braces in doc comments cannot
    terminate a block early.
    """
    fns: list[str] = []
    in_block = False
    started = False
    depth = 0
    for line in source.splitlines():
        stripped = line.strip()
        if not in_block:
            if "#[contractimpl]" in stripped:
                in_block = True
                started = False
                depth = 0
            continue
        if stripped.startswith("//"):
            continue
        depth += stripped.count("{") - stripped.count("}")
        if depth > 0:
            started = True
        match = re.search(r"pub\s+fn\s+(\w+)", stripped)
        if match:
            fns.append(match.group(1))
        if started and depth <= 0:
            # Closing brace of the impl block.
            in_block = False
    return sorted(set(fns))


def filter_entrypoints(fns: list[str]) -> set[str]:
    """Reduce extracted function names to the public ABI surface."""
    return {
        f
        for f in fns
        if f not in AUDIT_ENTRYPOINT_ALLOWLIST and not f.startswith("_")
    }


def _fn_names_from_heading(text: str) -> set[str]:
    """Pull entry point names out of a heading's name part.

    Bare snake_case identifiers are allowed (``### `withdraw` ``) as well as
    signatures (``#### `pause(stream_id)` — ...``); prose after the em-dash is
    ignored.
    """
    name_part = _HEADING_NAME_SPLIT_RE.split(text, maxsplit=1)[0]
    names = set()
    for span in _BACKTICK_RE.findall(name_part):
        match = _FN_SPAN_RE.match(span)
        if match:
            names.add(match.group(1))
    return names


def _fn_names_from_table_row(line: str) -> set[str]:
    """Pull entry point names out of a markdown table row.

    Only the first cell is considered (signature columns), and the span must
    carry a parameter list, so parameter tables (``| `amount` | u128 |``),
    error tables (``| `TopUpTooSmall` | 23 | ... ``) and prose inside
    description cells do not produce phantom "documented" names.
    """
    if line.count("|") < 2:
        return set()
    first_cell = line.split("|")[1]
    names = set()
    for span in _BACKTICK_RE.findall(first_cell):
        match = _FN_SPAN_RE.match(span)
        if match and match.group(2):
            names.add(match.group(1))
    return names


def parse_documented_entry_points(doc_text: str) -> set[str]:
    """Collect entry point names from the ``## Entry points`` section.

    A name counts as documented when it appears in a sub-heading of that
    section (before its em-dash prose separator) or in the signature cell of
    a table row inside it. Nothing outside the section is considered
    (constants, types, prose elsewhere in the file), and fenced code blocks
    are skipped so code samples cannot inject phantom documentation.
    """
    documented: set[str] = set()
    in_section = False
    in_fence = False
    for line in doc_text.splitlines():
        if line.lstrip().startswith("```"):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        heading = _HEADING_RE.match(line)
        if heading:
            if len(heading.group(1)) == 2:
                in_section = bool(_ENTRY_POINTS_SECTION_RE.match(line))
                continue
            if in_section:
                documented |= _fn_names_from_heading(heading.group(2))
            continue
        if in_section and line.lstrip().startswith("|"):
            documented |= _fn_names_from_table_row(line)
    return documented


def load_baseline_from_text(text: str, source: Path = BASELINE_PATH) -> dict[str, str]:
    """Parse baseline JSON content into a gap id -> reason mapping.

    Raises :class:`BaselineError` on anything that is not a well-formed
    baseline so a corrupted file fails the gate loudly.
    """
    try:
        data = json.loads(text)
    except json.JSONDecodeError as exc:
        raise BaselineError(f"{source} is not valid JSON: {exc}") from exc
    if not isinstance(data, dict) or not {"gaps"} <= set(data) or set(data) - {"gaps", "_comment"}:
        raise BaselineError(
            f"{source} must be a JSON object with a 'gaps' array and an "
            "optional '_comment' string"
        )
    if "_comment" in data and not isinstance(data["_comment"], str):
        raise BaselineError(f"{source}: '_comment' must be a string")
    baseline: dict[str, str] = {}
    for index, entry in enumerate(data["gaps"]):
        if (
            not isinstance(entry, dict)
            or set(entry) != {"id", "reason"}
            or not isinstance(entry["id"], str)
            or not isinstance(entry["reason"], str)
            or not entry["id"].strip()
            or not entry["reason"].strip()
        ):
            raise BaselineError(
                f"{source}: gaps[{index}] must be an object with non-empty "
                "'id' and 'reason' strings"
            )
        if entry["id"] in baseline:
            raise BaselineError(f"{source}: duplicate gap id '{entry['id']}'")
        baseline[entry["id"]] = entry["reason"]
    return baseline


def load_baseline(path: Path) -> dict[str, str]:
    """Load gap id -> reason from the baseline JSON file.

    Missing file means "no baseline". Malformed content raises
    :class:`BaselineError` so the gate fails loudly instead of ignoring a
    baseline that was accidentally corrupted.
    """
    if not path.exists():
        return {}
    return load_baseline_from_text(path.read_text(encoding="utf-8"), path)


def check_audit_md_entrypoint_drift(
    source: str, audit_text: str, audit_path: Path
) -> bool:
    """Check that audit.md entrypoint table covers all public ABI functions.

    Returns True if drift is detected (table is out of date).
    """
    fns = extract_contractimpl_pub_fns(source)
    entrypoints = filter_entrypoints(fns)

    missing = [f for f in entrypoints if f not in audit_text]
    return bool(missing)


def check_error_alignment() -> bool:
    """Check that error.md discriminants match ContractError enum.

    Informational only: docs/error.md does not exist in this repository (the
    error tables live in docs/ABI.md and are gated by
    script/check-discriminant-collisions.py), so this usually reports SKIP.
    """
    error_rs = REPO_ROOT / "contracts" / "stream" / "src" / "error.rs"
    error_md = REPO_ROOT / "docs" / "error.md"

    if not error_rs.exists():
        print(f"SKIP: {error_rs} not found")
        return True
    if not error_md.exists():
        print(f"SKIP: {error_md} not found")
        return True

    source = error_rs.read_text(encoding="utf-8")
    doc = error_md.read_text(encoding="utf-8")

    variants = extract_error_variants(source)
    if not variants:
        print("WARNING: No error variants found in source")
        return True

    missing = [v for v in variants if v not in doc]
    if missing:
        print(f"WARNING: {len(missing)} error variant(s) not in error.md: {missing}")
    return True


def extract_error_variants(source: str) -> dict[str, int]:
    """Extract ContractError variants with their explicit discriminants."""
    variants: dict[str, int] = {}
    current_discriminant = 0
    for line in source.splitlines():
        stripped = line.strip()
        # Match explicit discriminants like: Variant = 42,
        explicit = re.match(r"(\w+)\s*=\s*(\d+)", stripped)
        if explicit:
            variants[explicit.group(1)] = int(explicit.group(2))
            current_discriminant = int(explicit.group(2)) + 1
            continue
        # Match plain variants
        plain = re.match(r"(\w+)\s*[,{]", stripped)
        if plain and plain.group(1) not in ("ContractError", "enum", "pub"):
            variants[plain.group(1)] = current_discriminant
            current_discriminant += 1
    return variants


def collect_gaps(entrypoints: set[str], documented: set[str]) -> tuple[list[str], list[str]]:
    """Compute both misalignment directions.

    Returns (missing_from_docs, documented_but_nonexistent), both sorted.
    """
    missing = sorted(entrypoints - documented)
    ghosts = sorted(documented - entrypoints)
    return missing, ghosts


def _display_path(path: Path) -> str:
    """Render a path repo-relative for messages when possible."""
    try:
        return str(path.relative_to(REPO_ROOT))
    except ValueError:
        return str(path)


def _describe_gap(gap_id: str) -> str:
    name = gap_id.split(":", 1)[1] if ":" in gap_id else gap_id
    if gap_id.startswith(MISSING_DOC_PREFIX):
        return f"{name}: entry point in lib.rs but missing from docs/ABI.md"
    if gap_id.startswith(GHOST_DOC_PREFIX):
        return f"{name}: documented in docs/ABI.md but not an entry point in lib.rs"
    return name


def main() -> int:
    if not LIB_RS.exists():
        print(f"ERROR: {LIB_RS} not found; cannot determine the contract surface.")
        return 2
    if not ABI_MD.exists():
        print(f"ERROR: {ABI_MD} not found; nothing to align against.")
        return 2

    source = LIB_RS.read_text(encoding="utf-8")
    doc = ABI_MD.read_text(encoding="utf-8")

    entrypoints = filter_entrypoints(extract_contractimpl_pub_fns(source))
    if not entrypoints:
        print("ERROR: no public entry points found in lib.rs; parser or source is broken.")
        return 2
    documented = parse_documented_entry_points(doc)

    missing, ghosts = collect_gaps(entrypoints, documented)
    gap_ids = {f"{MISSING_DOC_PREFIX}{n}" for n in missing}
    gap_ids |= {f"{GHOST_DOC_PREFIX}{n}" for n in ghosts}

    print(f"Entry points in lib.rs: {len(entrypoints)}")
    print(f"Entry points documented in docs/ABI.md: {len(documented)}")
    for name in missing:
        print(f"  MISSING-FROM-DOCS: {name}")
    for name in ghosts:
        print(f"  DOCUMENTED-BUT-NONEXISTENT: {name}")

    try:
        baseline = load_baseline(BASELINE_PATH)
    except BaselineError as exc:
        print(f"ERROR: {exc}")
        return 2

    unbaselined = sorted(gap_ids - baseline.keys())
    stale = sorted(baseline.keys() - gap_ids)

    if stale:
        print("Stale baseline entries (gap no longer exists; remove them from")
        print(f"{_display_path(BASELINE_PATH)}):")
        for gap_id in stale:
            print(f"  STALE-BASELINE: {gap_id} (was baselined: {baseline[gap_id]})")
    if unbaselined:
        print("Documentation gaps not covered by the baseline:")
        for gap_id in unbaselined:
            print(f"  UNBASELINED: {gap_id} ({_describe_gap(gap_id)})")
        print(
            f"Fix docs/ABI.md (or lib.rs), or add the gap to "
            f"{_display_path(BASELINE_PATH)} with a reason."
        )

    if unbaselined or stale:
        print(
            f"FAIL: documentation alignment check failed "
            f"({len(unbaselined)} unbaselined gap(s), {len(stale)} stale baseline entries)."
        )
        return 1

    if gap_ids:
        print(f"OK: documentation alignment check passed ({len(gap_ids)} baselined gap(s)).")
    else:
        print("OK: documentation alignment check passed (no gaps).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
