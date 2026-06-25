#!/usr/bin/env bash
# check-wasm-size.sh — Enforce per-contract WASM byte budgets.
#
# Usage: bash script/check-wasm-size.sh [--optimized]
#
# Flags:
#   --optimized   Check *.optimized.wasm files instead of raw release artifacts.
#
# Exit codes:
#   0  All contracts within budget.
#   1  One or more contracts exceed their budget (or an artifact is missing).
#
# Budgets are set with ~25% headroom above the sizes measured during the
# 2026-06 baseline audit. Soroban's effective upload limit is ~100 KiB
# (post-compression), so raw WASM budgets are intentionally conservative.
#
# Per-contract raw WASM budgets (bytes):
#   fluxora_stream    — 262 144  (256 KiB)
#   fluxora_factory   — 131 072  (128 KiB)
#   fluxora_governance— 131 072  (128 KiB)
#
# Update these budgets when a deliberate feature addition is landed; document
# the change in docs/gas.md and in the PR description.

set -euo pipefail

# Allow tests (and CI) to override the artifact directory via environment.
WASM_DIR="${WASM_DIR:-target/wasm32-unknown-unknown/release}"
OPTIMIZED=0

for arg in "$@"; do
  case "$arg" in
    --optimized) OPTIMIZED=1 ;;
    *) echo "Unknown argument: $arg" >&2; exit 1 ;;
  esac
done

# ---------------------------------------------------------------------------
# Budget table: contract_name -> max_bytes
# ---------------------------------------------------------------------------
declare -A BUDGETS=(
  [fluxora_stream]=262144       # 256 KiB
  [fluxora_factory]=131072      # 128 KiB
  [fluxora_governance]=131072   # 128 KiB
)

FAILED=0
SUMMARY_ROWS=()

for CONTRACT in "${!BUDGETS[@]}"; do
  BUDGET="${BUDGETS[$CONTRACT]}"

  if [ "$OPTIMIZED" -eq 1 ]; then
    WASM_FILE="${WASM_DIR}/${CONTRACT}.optimized.wasm"
  else
    WASM_FILE="${WASM_DIR}/${CONTRACT}.wasm"
  fi

  if [ ! -f "$WASM_FILE" ]; then
    echo "::error::Artifact not found: ${WASM_FILE}" >&2
    FAILED=1
    SUMMARY_ROWS+=("| ${CONTRACT} | missing | ${BUDGET} | ❌ MISSING |")
    continue
  fi

  SIZE=$(wc -c < "$WASM_FILE")
  BUDGET_KIB=$(( BUDGET / 1024 ))
  SIZE_KIB=$(awk "BEGIN { printf \"%.1f\", ${SIZE}/1024 }")

  if [ "$SIZE" -gt "$BUDGET" ]; then
    echo "::error::${CONTRACT}.wasm is ${SIZE} bytes (${SIZE_KIB} KiB) — exceeds budget of ${BUDGET} bytes (${BUDGET_KIB} KiB)." >&2
    FAILED=1
    SUMMARY_ROWS+=("| ${CONTRACT} | ${SIZE} (${SIZE_KIB} KiB) | ${BUDGET} (${BUDGET_KIB} KiB) | ❌ OVER BUDGET |")
  else
    HEADROOM=$(( BUDGET - SIZE ))
    HEADROOM_KIB=$(awk "BEGIN { printf \"%.1f\", ${HEADROOM}/1024 }")
    echo "${CONTRACT}: ${SIZE} bytes (${SIZE_KIB} KiB) — OK (headroom: ${HEADROOM_KIB} KiB)"
    SUMMARY_ROWS+=("| ${CONTRACT} | ${SIZE} (${SIZE_KIB} KiB) | ${BUDGET} (${BUDGET_KIB} KiB) | ✅ OK (${HEADROOM_KIB} KiB headroom) |")
  fi
done

# ---------------------------------------------------------------------------
# GitHub Actions step summary (no-op outside of CI)
# ---------------------------------------------------------------------------
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  {
    echo "## WASM Size Budget Report"
    echo ""
    echo "| Contract | Size | Budget | Status |"
    echo "|----------|------|--------|--------|"
    for ROW in "${SUMMARY_ROWS[@]}"; do
      echo "$ROW"
    done
  } >> "$GITHUB_STEP_SUMMARY"
fi

if [ "$FAILED" -ne 0 ]; then
  echo ""
  echo "One or more contracts exceeded their WASM size budget." >&2
  echo "See docs/gas.md for budget values and how to update them." >&2
  exit 1
fi

echo ""
echo "All contracts within WASM size budget."
