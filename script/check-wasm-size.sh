#!/usr/bin/env bash
# check-wasm-size.sh — Enforce per-contract WASM byte budgets and report
# per-PR byte-delta against the previous release tag.
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
# Delta reporting:
#   The script fetches the previous release tag's WASM sizes from git history
#   and prints the byte-delta for each contract. In CI, a ::warning:: or
#   ::notice:: annotation is emitted per contract to surface the change in PR
#   builds without failing the job (unless the absolute budget is crossed).
#
# Update these budgets when a deliberate feature addition is landed; document
# the change in docs/gas.md and in the PR description.
#
# Artifact integrity: every artifact is also checked for the 4-byte WASM
# magic number (0x00 0x61 0x73 0x6D) and a minimum size before its budget is
# evaluated. This catches empty, truncated, or otherwise corrupted build
# outputs (e.g. a build interrupted mid-write) that would otherwise silently
# "pass" the budget check by virtue of being suspiciously small.
#
# Determinism: contract processing order is a fixed list (see CONTRACTS
# below), not a bash associative-array key iteration, so output order (and
# therefore step-summary row order) is identical across bash versions,
# platforms, and repeated/retried runs. This also keeps the script
# compatible with bash 3.2 (macOS's default `/bin/bash`), which predates
# `declare -A`.

set -euo pipefail

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
WASM_DIR="${WASM_DIR:-target/wasm32-unknown-unknown/release}"
OPTIMIZED=0
# GIT_REPO allows tests to override the repository root for git operations.
GIT_REPO="${GIT_REPO:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"

for arg in "$@"; do
  case "$arg" in
    --optimized) OPTIMIZED=1 ;;
    *) echo "Unknown argument: $arg" >&2; exit 1 ;;
  esac
done

# ---------------------------------------------------------------------------
# Budget table: fixed-order contract list + lookup function.
#
# Deliberately not a bash associative array (`declare -A`): associative-array
# key iteration order is an implementation detail of the bash build (and
# `declare -A` itself requires bash 4+, which macOS's shipped `/bin/bash`
# does not have). An ordered array plus a `case`-based lookup keeps contract
# processing order — and therefore all printed/step-summary output — fixed
# and reproducible everywhere.
# ---------------------------------------------------------------------------
CONTRACTS=(fluxora_stream fluxora_factory fluxora_governance)

budget_for() {
  case "$1" in
    fluxora_stream) echo 262144 ;;      # 256 KiB
    fluxora_factory) echo 131072 ;;     # 128 KiB
    fluxora_governance) echo 131072 ;;  # 128 KiB
    *) echo "" ;;
  esac
}

# ---------------------------------------------------------------------------
# Helper: find the previous release tag (most recent v* tag before HEAD)
# ---------------------------------------------------------------------------
find_previous_release_tag() {
  # List tags matching v* sorted by version, newest first, skip HEAD's tag.
  local tag
  tag=$(git -C "$GIT_REPO" tag -l 'v*' --sort=-version:refname 2>/dev/null | head -n 1)
  if [ -n "$tag" ]; then
    echo "$tag"
  else
    echo ""
  fi
}

# ---------------------------------------------------------------------------
# Helper: get a WASM blob's size at a given git ref from `ls-tree`.
# Returns the size in bytes, or empty string if the ref/path isn't found.
#
# This is the single lookup path for delta reporting (an earlier
# `get_wasm_size_at_ref` helper that tried `git cat-file -s` first has been
# removed: it was never called, and its own `ls-tree` fallback compared the
# blob-size column against the filename pattern instead of the path column,
# so it could not have worked if it had been wired in).
# ---------------------------------------------------------------------------
get_blob_size_from_ls_tree() {
  local ref="$1"
  local contract="$2"
  local suffix=".wasm"

  if [ "$OPTIMIZED" -eq 1 ]; then
    suffix=".optimized.wasm"
  fi

  local pattern="${contract}${suffix}"
  local line
  line=$(git -C "$GIT_REPO" ls-tree -r -l "${ref}" 2>/dev/null \
    | grep -E "${pattern}$" | head -n 1) || true

  if [ -n "$line" ]; then
    # ls-tree -r -l output: <mode> <type> <object hash> <size> <path>
    echo "$line" | awk '{ print $4 }'
  else
    echo ""
  fi
}

# ---------------------------------------------------------------------------
# Main size check and delta computation
# ---------------------------------------------------------------------------
FAILED=0
SUMMARY_ROWS=()
DELTA_ROWS=()
HAS_DELTA_DATA=0

# Find the previous release tag for delta comparison.
PREV_TAG=$(find_previous_release_tag)
if [ -n "$PREV_TAG" ]; then
  echo "Previous release tag: ${PREV_TAG}"
else
  echo "No previous release tag found — delta reporting disabled."
fi

for CONTRACT in "${CONTRACTS[@]}"; do
  BUDGET="$(budget_for "$CONTRACT")"

  if [ "$OPTIMIZED" -eq 1 ]; then
    WASM_FILE="${WASM_DIR}/${CONTRACT}.optimized.wasm"
  else
    WASM_FILE="${WASM_DIR}/${CONTRACT}.wasm"
  fi

  if [ ! -f "$WASM_FILE" ]; then
    echo "::error::Artifact not found: ${WASM_FILE}" >&2
    FAILED=1
    SUMMARY_ROWS+=("| ${CONTRACT} | missing | ${BUDGET} | ❌ MISSING |")
    if [ -n "$PREV_TAG" ]; then
      DELTA_ROWS+=("| ${CONTRACT} | — | — | — |")
    fi
    continue
  fi

  SIZE=$(wc -c < "$WASM_FILE")

  # Artifact-integrity check: reject empty/truncated/non-WASM files before
  # they can be evaluated against the byte budget. Without this, a 0-byte
  # or partial artifact from an interrupted build would silently "pass"
  # with maximal headroom instead of failing loudly.
  if [ "$SIZE" -lt 8 ]; then
    echo "::error::${WASM_FILE} is only ${SIZE} bytes — too small to be a valid WASM module (empty or truncated build artifact?)." >&2
    FAILED=1
    SUMMARY_ROWS+=("| ${CONTRACT} | ${SIZE} bytes | ${BUDGET} | ❌ INVALID (too small) |")
    continue
  fi
  MAGIC=$(od -An -tx1 -N4 "$WASM_FILE" | tr -d ' \n')
  if [ "$MAGIC" != "0061736d" ]; then
    echo "::error::${WASM_FILE} does not start with the WASM magic number (expected 0061736d, got ${MAGIC}) — not a valid WASM module." >&2
    FAILED=1
    SUMMARY_ROWS+=("| ${CONTRACT} | ${SIZE} bytes | ${BUDGET} | ❌ INVALID (bad magic) |")
    continue
  fi

  BUDGET_KIB=$(( BUDGET / 1024 ))
  SIZE_KIB=$(awk "BEGIN { printf \"%.1f\", ${SIZE}/1024 }")

  # Compute delta against previous release tag.
  DELTA_INFO=""
  if [ -n "$PREV_TAG" ]; then
    PREV_SIZE=$(get_blob_size_from_ls_tree "$PREV_TAG" "$CONTRACT")
    if [ -n "$PREV_SIZE" ] && [ "$PREV_SIZE" -gt 0 ] 2>/dev/null; then
      DELTA=$(( SIZE - PREV_SIZE ))
      DELTA_KIB=$(awk "BEGIN { printf \"%.1f\", ${DELTA}/1024 }")
      PREV_KIB=$(awk "BEGIN { printf \"%.1f\", ${PREV_SIZE}/1024 }")
      HAS_DELTA_DATA=1

      if [ "$DELTA" -gt 0 ]; then
        DELTA_INFO="+${DELTA} bytes (+${DELTA_KIB} KiB)"
      elif [ "$DELTA" -lt 0 ]; then
        ABS_DELTA=$(( -DELTA ))
        DELTA_INFO="-${ABS_DELTA} bytes (-${DELTA_KIB} KiB)"
      else
        DELTA_INFO="0 bytes (no change)"
      fi

      echo "  vs ${PREV_TAG}: ${DELTA_INFO} (was ${PREV_SIZE} / ${PREV_KIB} KiB)"
      DELTA_ROWS+=("| ${CONTRACT} | ${SIZE} (${SIZE_KIB} KiB) | ${PREV_SIZE} (${PREV_KIB} KiB) | ${DELTA_INFO} |")

      # Emit CI annotation for the delta (does NOT fail the build).
      if [ -n "${GITHUB_STEP_SUMMARY:-}" ] || [ -n "${GITHUB_ACTIONS:-}" ]; then
        if [ "$DELTA" -gt 0 ]; then
          # Positive delta (bloat) → warning annotation.
          echo "::warning::${CONTRACT}.wasm grew by ${DELTA_INFO} since ${PREV_TAG}"
        elif [ "$DELTA" -lt 0 ]; then
          # Negative delta (shrink) → notice annotation.
          echo "::notice::${CONTRACT}.wasm shrank by ${DELTA_INFO} since ${PREV_TAG}"
        else
          echo "::notice::${CONTRACT}.wasm unchanged since ${PREV_TAG}"
        fi
      fi
    else
      echo "  vs ${PREV_TAG}: baseline not available for ${CONTRACT}"
      DELTA_ROWS+=("| ${CONTRACT} | ${SIZE} (${SIZE_KIB} KiB) | — | baseline unavailable |")
    fi
  fi

  # Budget enforcement.
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

    if [ "$HAS_DELTA_DATA" -eq 1 ]; then
      echo ""
      echo "## WASM Size Delta (vs ${PREV_TAG})"
      echo ""
      echo "| Contract | Current | Previous (${PREV_TAG}) | Delta |"
      echo "|----------|---------|----------------------|-------|"
      for ROW in "${DELTA_ROWS[@]}"; do
        echo "$ROW"
      done
    fi
  } >> "$GITHUB_STEP_SUMMARY"
fi

# ---------------------------------------------------------------------------
# Exit with failure if any contract exceeded its budget.
# ---------------------------------------------------------------------------
if [ "$FAILED" -ne 0 ]; then
  echo ""
  echo "One or more contracts exceeded their WASM size budget." >&2
  echo "See docs/gas.md for budget values and how to update them." >&2
  exit 1
fi

# Edge-case: warn when headroom drops below 10% of budget.
for CONTRACT in "${CONTRACTS[@]}"; do
  BUDGET="$(budget_for "$CONTRACT")"
  if [ "$OPTIMIZED" -eq 1 ]; then
    WASM_FILE="${WASM_DIR}/${CONTRACT}.optimized.wasm"
  else
    WASM_FILE="${WASM_DIR}/${CONTRACT}.wasm"
  fi
  if [ -f "$WASM_FILE" ]; then
    SIZE=$(wc -c < "$WASM_FILE")
    THRESHOLD=$(( BUDGET * 90 / 100 ))
    if [ "$SIZE" -gt "$THRESHOLD" ]; then
      echo "::warning::${CONTRACT}.wasm is within 10% of budget — consider expanding budget or reducing binary size." >&2
    fi
  fi
done

echo ""
echo "All contracts within WASM size budget."
