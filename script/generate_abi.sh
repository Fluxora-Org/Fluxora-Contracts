#!/usr/bin/env bash
#
# Generate the Fluxora contract ABI JSON from the optimized WASM.
#
# Usage:
#   script/generate_abi.sh [--wasm WASM_PATH] [--output OUTPUT_PATH]
#
# The ABI is extracted from the *optimized* deployable WASM — the same bytes
# that are deployed — so the published interface matches exactly what
# integrators and indexers observe on-chain. The ABI_VERSION constant in
# contracts/stream/src/lib.rs is embedded as the top-level abi_version field,
# ensuring the JSON and the Rust source cannot drift apart.
#
# Output:
#   contracts/stream/abi/fluxora_abi.json   (or --output path)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

WASM_PATH=""
OUTPUT_PATH="$REPO_ROOT/contracts/stream/abi/fluxora_abi.json"
ABI_VERSION_SOURCE="$REPO_ROOT/contracts/stream/src/lib.rs"
UPGRADE_POSTURE='Immutable. This contract exposes no upgrade entry point and cannot be replaced in place. New functionality requires deploying a new contract and migrating state explicitly.'

while [[ $# -gt 0 ]]; do
  case "$1" in
    --wasm)
      WASM_PATH="$2"; shift 2 ;;
    --output)
      OUTPUT_PATH="$2"; shift 2 ;;
    *)
      echo "usage: $0 [--wasm WASM_PATH] [--output OUTPUT_PATH]" >&2
      exit 2 ;;
  esac
done

say() { printf '\n\033[1m── %s\033[0m\n' "$*"; }

# Default to the optimized deployable artifact produced by
# `stellar contract optimize`.
if [[ -z "$WASM_PATH" ]]; then
  WASM_PATH="$REPO_ROOT/target/wasm32v1-none/release/fluxora_stream.optimized.wasm"
fi

if [[ ! -f "$WASM_PATH" ]]; then
  echo "ERROR: optimized WASM not found at $WASM_PATH" >&2
  echo "Run: stellar contract optimize --wasm <wasm>" >&2
  exit 1
fi

# Extract ABI_VERSION from the Rust source (the single source of truth).
ABI_VERSION=$(sed -n 's/^pub const ABI_VERSION: u32 = \([0-9]\+\);/\1/p' "$ABI_VERSION_SOURCE")
if [[ -z "$ABI_VERSION" ]]; then
  echo "ERROR: could not extract ABI_VERSION from $ABI_VERSION_SOURCE" >&2
  exit 1
fi

say "1. extract interface spec from optimized WASM"
INTERFACE_RAW=$(stellar contract info interface --wasm "$WASM_PATH" --output json)
ENTRY_COUNT=$(printf '%s' "$INTERFACE_RAW" | python3 -c "import json,sys; print(len(json.load(sys.stdin)))")

say "   wasm:        $WASM_PATH"
say "   abi_version: $ABI_VERSION"
say "   entries:     $ENTRY_COUNT"

say "2. write ABI JSON"
mkdir -p "$(dirname "$OUTPUT_PATH")"
printf '%s' "$INTERFACE_RAW" | \
  ABI_VERSION="$ABI_VERSION" OUTPUT_PATH="$OUTPUT_PATH" UPGRADE_POSTURE="$UPGRADE_POSTURE" \
  python3 -c '
import json, os, sys

entries = json.load(sys.stdin)
abi_version = int(os.environ["ABI_VERSION"])
output_path = os.environ["OUTPUT_PATH"]
upgrade_posture = os.environ["UPGRADE_POSTURE"]

abi = {
    "abi_version": abi_version,
    "upgradeable": False,
    "upgrade_posture": upgrade_posture,
    "functions": entries,
}

with open(output_path, "w") as f:
    json.dump(abi, f, indent=2)
    f.write("\n")

print(f"   written:    {output_path}")
print(f"   functions:  {len(entries)}")
'

say "3. done"
printf '   \033[32m✓\033[0m %s\n' "$OUTPUT_PATH"
