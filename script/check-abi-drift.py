#!/usr/bin/env python3
"""
check-abi-drift.py  —  ABI drift gate (issue #1692)

Verifies that contracts/stream/abi/fluxora_stream.json is consistent with:
  1. The public #[contractimpl] entry points declared in contracts/stream/src/lib.rs
  2. The entry-point surface documented in docs/ABI.md

Exit codes:
  0  — no drift detected
  1  — drift detected (ABI out of sync)
  2  — bad inputs (missing file, parse error)

Run to check:
  python3 script/check-abi-drift.py

To regenerate the committed ABI JSON from the source:
  python3 script/check-abi-drift.py --regenerate
  # then review and commit contracts/stream/abi/fluxora_stream.json
"""

import argparse
import json
import re
import sys
from pathlib import Path

# ---------------------------------------------------------------------------
# Paths
# ---------------------------------------------------------------------------

REPO_ROOT = Path(__file__).resolve().parent.parent
ABI_JSON = REPO_ROOT / "contracts" / "stream" / "abi" / "fluxora_stream.json"
LIB_RS   = REPO_ROOT / "contracts" / "stream" / "src" / "lib.rs"
ABI_MD   = REPO_ROOT / "docs" / "ABI.md"

# ---------------------------------------------------------------------------
# Entry points that live in lib.rs as #[contractimpl] pub fn but are
# intentionally excluded from the ABI JSON and ABI.md  (internal or
# meta-methods that consumers never call directly).
# ---------------------------------------------------------------------------
EXCLUDE_FROM_ABI = {
    "upgradeable",   # internal flag, not a callable ABI entrypoint
}

# ---------------------------------------------------------------------------
# Entry-point names that are in the ABI JSON but are not yet fully
# documented in ABI.md.  These are tracked as known gaps and do not fail
# the docs check; they WILL fail the JSON-vs-lib.rs check if missing.
# Shrink this set as docs are added; never grow it without a comment.
# ---------------------------------------------------------------------------
KNOWN_ABI_MD_GAPS: set[str] = set()


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def die(msg: str, code: int = 2) -> None:
    print(f"ERROR: {msg}", file=sys.stderr)
    sys.exit(code)


def load_json(path: Path) -> dict:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        die(f"File not found: {path}")
    except json.JSONDecodeError as exc:
        die(f"JSON parse error in {path}: {exc}")


def extract_lib_rs_entrypoints(source: str) -> set[str]:
    """
    Parse the pub fn names from a #[contractimpl] block in lib.rs.

    Strategy: find every `pub fn <name>(` inside a `#[contractimpl]` block.
    We look for them naively (all pub fn in the file) then exclude names that
    appear outside impl blocks (e.g. helper functions defined at module level).

    A simpler and more robust approach: collect ALL `pub fn` names in the file
    — they are almost exclusively entry points in a Soroban contract — then
    subtract the known exclusion set.  If a pub fn is truly not an entry point
    it belongs in EXCLUDE_FROM_ABI.
    """
    # Match:  pub fn name(
    pattern = re.compile(r"^\s{4}pub fn\s+(\w+)\s*\(", re.MULTILINE)
    names = {m.group(1) for m in pattern.finditer(source)}
    return names - EXCLUDE_FROM_ABI


def extract_abi_json_functions(data: dict) -> set[str]:
    """Return the set of function names listed in the ABI JSON."""
    functions = data.get("functions", [])
    if not isinstance(functions, list):
        die("ABI JSON 'functions' key is not a list")
    return {f["name"] for f in functions if isinstance(f, dict) and "name" in f}


def extract_abi_md_functions(text: str) -> set[str]:
    """
    Extract function names that are explicitly documented in docs/ABI.md.

    ABI.md uses several heading and table patterns:
      #### `fn_name(...)`
      ### `fn_name`
      ### fn_name
      `fn_name(args)` — table cell or prose
      `fn_name` — standalone code span

    We collect every snake_case identifier that appears either:
      - at the start of a backtick-quoted span (possibly followed by `(`), or
      - right after a markdown heading marker.
    """
    names: set[str] = set()

    # Heading-style: `fn_name` or fn_name right after ####/###/## (with optional backtick)
    heading_pattern = re.compile(
        r"^#{1,4}\s+[`']?([a-z][a-z0-9_]*)[`'(]?", re.MULTILINE
    )
    # Backtick spans: `fn_name` or `fn_name(` — captures the bare identifier
    # before any opening paren or closing backtick.
    backtick_pattern = re.compile(r"`([a-z][a-z0-9_]*)[\(`]")

    for m in heading_pattern.finditer(text):
        names.add(m.group(1))
    for m in backtick_pattern.finditer(text):
        names.add(m.group(1))
    return names


# ---------------------------------------------------------------------------
# Regenerate ABI JSON from lib.rs  (--regenerate flag)
# ---------------------------------------------------------------------------

def regenerate_abi_json() -> None:
    """
    Regenerate contracts/stream/abi/fluxora_stream.json from lib.rs.

    This is a *structural* regeneration — it rewrites the "functions" list to
    match the pub fn surface of lib.rs while preserving every other key in the
    existing JSON (abi_version, types, upgradeable, etc.).

    Full type and parameter information should come from
    `stellar contract info interface` against the built WASM.  Use this helper
    to keep the function-name list in sync; re-run the Stellar CLI step after
    a full build to refresh parameter types.

    To regenerate from the built WASM (recommended):
      stellar contract info interface \\
        --wasm target/wasm32v1-none/release/fluxora_stream.wasm \\
        --output json > contracts/stream/abi/fluxora_stream.json
    """
    if not LIB_RS.exists():
        die(f"lib.rs not found: {LIB_RS}")

    source = LIB_RS.read_text(encoding="utf-8")
    lib_names = extract_lib_rs_entrypoints(source)

    existing: dict = {}
    if ABI_JSON.exists():
        try:
            existing = json.loads(ABI_JSON.read_text(encoding="utf-8"))
        except json.JSONDecodeError:
            pass  # start fresh

    # Keep existing function entries for names that still exist; add stubs for new ones
    existing_functions: list[dict] = existing.get("functions", [])
    existing_by_name = {f["name"]: f for f in existing_functions if isinstance(f, dict)}

    new_functions = []
    for name in sorted(lib_names):
        if name in existing_by_name:
            new_functions.append(existing_by_name[name])
        else:
            # Stub — types unknown until a full WASM build
            new_functions.append({
                "name": name,
                "auth": "unknown",
                "inputs": [],
                "outputs": "unknown",
            })

    updated = {**existing, "functions": new_functions}
    ABI_JSON.write_text(json.dumps(updated, indent=2) + "\n", encoding="utf-8")
    print(f"Regenerated {ABI_JSON} ({len(new_functions)} functions)")
    print()
    print("NOTE: parameter types are stubs for newly added functions.")
    print("For full type info, run:")
    print("  stellar contract info interface \\")
    print("    --wasm target/wasm32v1-none/release/fluxora_stream.wasm \\")
    print("    --output json > contracts/stream/abi/fluxora_stream.json")


# ---------------------------------------------------------------------------
# Check
# ---------------------------------------------------------------------------

def check() -> bool:
    """
    Run all drift checks.  Returns True if everything is clean, False on drift.
    """
    failures: list[str] = []

    # ------------------------------------------------------------------
    # 1. Load inputs
    # ------------------------------------------------------------------
    for path in (ABI_JSON, LIB_RS, ABI_MD):
        if not path.exists():
            die(f"Required file not found: {path}")

    abi_data      = load_json(ABI_JSON)
    lib_source    = LIB_RS.read_text(encoding="utf-8")
    abi_md_text   = ABI_MD.read_text(encoding="utf-8")

    lib_names     = extract_lib_rs_entrypoints(lib_source)
    json_names    = extract_abi_json_functions(abi_data)
    md_names      = extract_abi_md_functions(abi_md_text)

    # ------------------------------------------------------------------
    # 2. Check 1: ABI JSON vs lib.rs
    # ------------------------------------------------------------------
    in_lib_not_json = lib_names - json_names
    in_json_not_lib = json_names - lib_names

    if in_lib_not_json:
        failures.append(
            "Functions in lib.rs but MISSING from ABI JSON:\n"
            + "\n".join(f"  - {n}" for n in sorted(in_lib_not_json))
        )
    if in_json_not_lib:
        failures.append(
            "Functions in ABI JSON but NOT FOUND in lib.rs:\n"
            + "\n".join(f"  - {n}" for n in sorted(in_json_not_lib))
        )

    # ------------------------------------------------------------------
    # 3. Check 2: ABI JSON vs docs/ABI.md
    # ------------------------------------------------------------------
    # We check that every JSON function name appears somewhere in ABI.md,
    # minus the documented known gaps.
    undocumented = json_names - md_names - KNOWN_ABI_MD_GAPS
    if undocumented:
        failures.append(
            "Functions in ABI JSON but NOT mentioned in docs/ABI.md:\n"
            + "\n".join(f"  - {n}" for n in sorted(undocumented))
            + "\n  (Add them to docs/ABI.md or to KNOWN_ABI_MD_GAPS in this script)"
        )

    # ------------------------------------------------------------------
    # 4. Report
    # ------------------------------------------------------------------
    if failures:
        print("=" * 70)
        print("ABI DRIFT DETECTED")
        print("=" * 70)
        for msg in failures:
            print()
            print(msg)
        print()
        print("To regenerate the committed ABI JSON from source:")
        print("  python3 script/check-abi-drift.py --regenerate")
        print()
        print("To regenerate from the built WASM (full types):")
        print("  stellar contract info interface \\")
        print("    --wasm target/wasm32v1-none/release/fluxora_stream.wasm \\")
        print("    --output json > contracts/stream/abi/fluxora_stream.json")
        print("=" * 70)
        return False

    print("OK: ABI JSON is consistent with lib.rs entry points and docs/ABI.md.")
    print(f"    {len(json_names)} functions checked.")
    return True


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def main() -> None:
    parser = argparse.ArgumentParser(
        description="ABI drift gate: assert committed ABI matches lib.rs and docs/ABI.md"
    )
    parser.add_argument(
        "--regenerate",
        action="store_true",
        help="Regenerate contracts/stream/abi/fluxora_stream.json from lib.rs (review and commit the result)",
    )
    args = parser.parse_args()

    if args.regenerate:
        regenerate_abi_json()
        sys.exit(0)

    ok = check()
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
