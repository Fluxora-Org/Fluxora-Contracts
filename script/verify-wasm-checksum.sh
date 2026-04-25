#!/usr/bin/env bash
# verify-wasm-checksum.sh
#
# Verifies that locally-built WASM artifacts match the reference checksums in
# wasm/checksums.sha256. Use this to confirm a build is reproducible before
# deployment, or to audit a downloaded artifact.
#
# Usage:
#   bash script/verify-wasm-checksum.sh              # verify all contracts
#   bash script/verify-wasm-checksum.sh --no-build   # skip rebuild, check existing artifacts
#
# Exit codes:
#   0  All checksums match
#   1  One or more checksums mismatch, or a required file is missing

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHECKSUMS_FILE="${REPO_ROOT}/wasm/checksums.sha256"
BUILD=true

for arg in "$@"; do
  case "$arg" in
    --no-build) BUILD=false ;;
    *) echo "Unknown argument: $arg" >&2; exit 1 ;;
  esac
done

# ---------------------------------------------------------------------------
# Verify required tools
# ---------------------------------------------------------------------------
for tool in sha256sum awk grep; do
  if ! command -v "$tool" &>/dev/null; then
    echo "ERROR: required tool '$tool' not found in PATH" >&2
    exit 1
  fi
done

# ---------------------------------------------------------------------------
# Verify checksums file exists
# ---------------------------------------------------------------------------
if [ ! -f "${CHECKSUMS_FILE}" ]; then
  echo "ERROR: Reference checksums file not found: ${CHECKSUMS_FILE}" >&2
  echo "  Run: bash script/update-wasm-checksums.sh" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Optionally rebuild
# ---------------------------------------------------------------------------
if [ "$BUILD" = true ]; then
  if ! command -v cargo &>/dev/null; then
    echo "ERROR: cargo not found — cannot rebuild. Use --no-build to skip." >&2
    exit 1
  fi
  echo "Building WASM artifacts (release, wasm32-unknown-unknown)..."
  cargo build --release --target wasm32-unknown-unknown \
    --manifest-path "${REPO_ROOT}/Cargo.toml" \
    -p fluxora_stream
fi

# ---------------------------------------------------------------------------
# Verify each entry in checksums file
# ---------------------------------------------------------------------------
PASS=0
FAIL=0

while IFS= read -r line; do
  # Skip comments and blank lines
  [[ "$line" =~ ^#.*$ || -z "$line" ]] && continue

  EXPECTED_HASH=$(echo "$line" | awk '{print $1}')
  FILENAME=$(echo "$line" | awk '{print $2}')

  # Map filename to full path
  WASM_PATH="${REPO_ROOT}/target/wasm32-unknown-unknown/release/${FILENAME}"

  if [ ! -f "${WASM_PATH}" ]; then
    echo "MISSING  ${FILENAME}"
    echo "         Expected path: ${WASM_PATH}"
    FAIL=$((FAIL + 1))
    continue
  fi

  ACTUAL_HASH=$(sha256sum "${WASM_PATH}" | awk '{print $1}')

  if [ "${ACTUAL_HASH}" = "${EXPECTED_HASH}" ]; then
    echo "OK       ${FILENAME}"
    echo "         ${ACTUAL_HASH}"
    PASS=$((PASS + 1))
  else
    echo "MISMATCH ${FILENAME}"
    echo "         Expected: ${EXPECTED_HASH}"
    echo "         Actual:   ${ACTUAL_HASH}"
    echo ""
    echo "  The WASM output differs from the committed reference."
    echo "  If this is an intentional source change, run:"
    echo "    bash script/update-wasm-checksums.sh"
    echo "  Then commit the updated wasm/checksums.sha256."
    FAIL=$((FAIL + 1))
  fi
done < "${CHECKSUMS_FILE}"

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
echo "Results: ${PASS} passed, ${FAIL} failed"

if [ "${FAIL}" -gt 0 ]; then
  echo "FAIL: Build is not reproducible — checksum(s) do not match reference."
  exit 1
fi

echo "PASS: All WASM checksums match reference. Build is reproducible."
