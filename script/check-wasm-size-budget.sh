#!/usr/bin/env bash
# check-wasm-size-budget.sh
#
# Builds (unless --no-build is passed) and checks release WASM artifacts against
# per-contract byte budgets. CI appends the measured sizes to the job summary so
# reviewers can see the current raw and optimized artifact sizes on every run.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RELEASE_DIR="${REPO_ROOT}/target/wasm32-unknown-unknown/release"
REPORT_FILE="${RELEASE_DIR}/wasm-size-report.md"
BUILD=true

# Budgets are intentionally explicit per contract. They should be raised only
# with a matching documentation update that explains the new size baseline.
: "${FLUXORA_STREAM_WASM_BUDGET_BYTES:=262144}"
: "${FLUXORA_FACTORY_WASM_BUDGET_BYTES:=98304}"
: "${FLUXORA_GOVERNANCE_WASM_BUDGET_BYTES:=65536}"

CONTRACTS=(
  "fluxora_stream:${FLUXORA_STREAM_WASM_BUDGET_BYTES}"
  "fluxora_factory:${FLUXORA_FACTORY_WASM_BUDGET_BYTES}"
  "fluxora_governance:${FLUXORA_GOVERNANCE_WASM_BUDGET_BYTES}"
)

usage() {
  cat <<'EOF'
Usage: bash script/check-wasm-size-budget.sh [--no-build] [--release-dir DIR] [--report-file FILE]

Options:
  --no-build          Check existing release WASM files without invoking cargo.
  --release-dir DIR   Directory containing fluxora_*.wasm artifacts.
  --report-file FILE  Markdown report path. Defaults under the release dir.
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --no-build)
      BUILD=false
      shift
      ;;
    --release-dir)
      RELEASE_DIR="$2"
      REPORT_FILE="${RELEASE_DIR}/wasm-size-report.md"
      shift 2
      ;;
    --report-file)
      REPORT_FILE="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

for tool in awk wc; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "ERROR: required tool '$tool' not found in PATH" >&2
    exit 1
  fi
done

if [ "$BUILD" = true ]; then
  if ! command -v cargo >/dev/null 2>&1; then
    echo "ERROR: cargo not found; pass --no-build to check existing artifacts." >&2
    exit 1
  fi

  echo "Building release WASM artifacts for size-budget checks..."
  cargo build --release --target wasm32-unknown-unknown \
    --manifest-path "${REPO_ROOT}/Cargo.toml" \
    -p fluxora_stream \
    -p fluxora_factory \
    -p fluxora_governance
fi

file_size_bytes() {
  local file="$1"

  if stat -c '%s' "$file" >/dev/null 2>&1; then
    stat -c '%s' "$file"
    return
  fi

  wc -c < "$file" | awk '{print $1}'
}

check_artifact() {
  local contract="$1"
  local kind="$2"
  local file="$3"
  local budget="$4"

  if [ ! -f "$file" ]; then
    echo "MISSING ${contract} ${kind} artifact: ${file}"
    printf '| `%s` | %s | missing | %s | FAIL |\n' "$contract" "$kind" "$budget" >> "$REPORT_FILE"
    FAILURES=$((FAILURES + 1))
    return
  fi

  local size
  size="$(file_size_bytes "$file")"

  if [ "$size" -le "$budget" ]; then
    echo "PASS ${contract} ${kind}: ${size} bytes <= ${budget}"
    printf '| `%s` | %s | %s | %s | PASS |\n' "$contract" "$kind" "$size" "$budget" >> "$REPORT_FILE"
  else
    echo "FAIL ${contract} ${kind}: ${size} bytes > ${budget}"
    printf '| `%s` | %s | %s | %s | FAIL |\n' "$contract" "$kind" "$size" "$budget" >> "$REPORT_FILE"
    FAILURES=$((FAILURES + 1))
  fi
}

mkdir -p "$(dirname "$REPORT_FILE")"
{
  echo "# WASM Size Budget Report"
  echo ""
  echo "| Contract | Artifact | Size bytes | Budget bytes | Status |"
  echo "|----------|----------|------------|--------------|--------|"
} > "$REPORT_FILE"

FAILURES=0

for entry in "${CONTRACTS[@]}"; do
  contract="${entry%%:*}"
  budget="${entry##*:}"
  raw_file="${RELEASE_DIR}/${contract}.wasm"
  optimized_file="${RELEASE_DIR}/${contract}.optimized.wasm"

  check_artifact "$contract" "raw" "$raw_file" "$budget"

  if [ -f "$optimized_file" ]; then
    check_artifact "$contract" "optimized" "$optimized_file" "$budget"
  else
    echo "SKIP ${contract} optimized: ${optimized_file} not present"
    printf '| `%s` | optimized | not present | %s | SKIP |\n' "$contract" "$budget" >> "$REPORT_FILE"
  fi
done

echo ""
cat "$REPORT_FILE"

if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  {
    echo ""
    cat "$REPORT_FILE"
  } >> "$GITHUB_STEP_SUMMARY"
fi

if [ "$FAILURES" -gt 0 ]; then
  echo "FAIL: ${FAILURES} WASM size-budget check(s) failed." >&2
  exit 1
fi

echo "PASS: all WASM artifacts are within their documented budgets."
