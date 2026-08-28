#!/usr/bin/env bash
#
# script/test-release-dry-run.sh
#
# Regression test suite for script/release-dry-run.sh.
# Tests local validation, boundary conditions, artifact checksums,
# failure rejection, retry idempotency, and the write confirmation guard.
#
# Usage:
#   bash script/test-release-dry-run.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DRY_RUN_BIN="$SCRIPT_DIR/release-dry-run.sh"

TMP_DIR="$(mktemp -d 2>/dev/null || mktemp -d -t 'fluxora-test')"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

PASS=0
FAIL=0

check() {
  local label="$1"
  local expected_exit="$2"
  local output_pattern="$3"
  shift 3

  local stdout_file="$TMP_DIR/stdout.log"
  local stderr_file="$TMP_DIR/stderr.log"
  local actual_exit=0

  "$DRY_RUN_BIN" "$@" >"$stdout_file" 2>"$stderr_file" || actual_exit=$?

  local combined_output
  combined_output="$(cat "$stdout_file" "$stderr_file")"

  local test_passed=true
  if [[ "$actual_exit" -ne "$expected_exit" ]]; then
    test_passed=false
  fi

  if [[ -n "$output_pattern" ]] && ! echo "$combined_output" | grep -Eq "$output_pattern"; then
    test_passed=false
  fi

  if $test_passed; then
    printf '   \033[32m✓\033[0m %-60s\n' "$label"
    PASS=$((PASS + 1))
  else
    printf '   \033[31m✗\033[0m %-60s (exit=%d, expected=%d)\n' "$label" "$actual_exit" "$expected_exit"
    echo "       Output: $combined_output"
    FAIL=$((FAIL + 1))
  fi
}

echo "════════════════════════════════════════════════════════════════════════"
echo " Fluxora — Release Dry-Run Regression Suite"
echo "════════════════════════════════════════════════════════════════════════"

# Create mock WASM artifacts for testing
MOCK_WASM="$TMP_DIR/test_contract.wasm"
echo "mock wasm content bytes for testing" > "$MOCK_WASM"

EMPTY_WASM="$TMP_DIR/empty.wasm"
touch "$EMPTY_WASM"

OVERSIZED_WASM="$TMP_DIR/oversized.wasm"
# Create file of 131,073 bytes (1 byte above 128 KiB limit)
head -c 131073 </dev/zero > "$OVERSIZED_WASM" 2>/dev/null || python3 -c "open('$OVERSIZED_WASM', 'wb').write(b'x'*131073)"

MOCK_CHECKSUM=$(sha256sum "$MOCK_WASM" 2>/dev/null | awk '{print $1}' || \
                shasum -a 256 "$MOCK_WASM" 2>/dev/null | awk '{print $1}' || \
                python3 -c 'import hashlib,sys; print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest())' "$MOCK_WASM")

# 56-character valid Stellar identifiers
VALID_CONTRACT_ID="CBCGTSCJXBMPPPE4BPDIPYZXPE2J5TQEKD2KCS7VQF533NKKEYGUTHXW"
VALID_TOKEN_ID="CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC"
VALID_ADMIN_ID="GBDEVU63Y6NTHJQQZIKVTC23EBOWL35KWITOIOQDGDYXMEM3GDIFEXIS"

# ── 1. Happy Path / Dry-Run Manifest ──────────────────────────────────────────
check "Dry-run with valid inputs succeeds (exit 0) without mutation" \
  0 "DRY-RUN SUCCESSFUL" \
  --local-only --wasm "$MOCK_WASM" --contract-id "$VALID_CONTRACT_ID" \
  --token "$VALID_TOKEN_ID" --admin "$VALID_ADMIN_ID"

check "Dry-run manifest prints artifact checksum and target" \
  0 "Artifact SHA-256.*$MOCK_CHECKSUM" \
  --local-only --wasm "$MOCK_WASM" --contract-id "$VALID_CONTRACT_ID"

check "Dry-run with expected checksum match succeeds" \
  0 "Artifact checksum matches expected reference" \
  --local-only --wasm "$MOCK_WASM" --expected-checksum "$MOCK_CHECKSUM"

# ── 2. Local Validation: Network Identifier ────────────────────────────────────
check "Empty network identifier is rejected before RPC" \
  1 "Network identifier cannot be empty" \
  --local-only --network "" --wasm "$MOCK_WASM"

check "Network identifier with illegal characters is rejected" \
  1 "Network identifier contains invalid characters" \
  --local-only --network "testnet;rm -rf" --wasm "$MOCK_WASM"

# ── 3. Local Validation: Contract Identifier Boundaries ────────────────────────
check "Contract ID: 55 chars (too short) is rejected" \
  1 "Contract ID length must be exactly 56 characters" \
  --local-only --contract-id "CBCGTSCJXBMPPPE4BPDIPYZXPE2J5TQEKD2KCS7VQF533NKKEYGUTHX" --wasm "$MOCK_WASM"

check "Contract ID: 57 chars (too long) is rejected" \
  1 "Contract ID length must be exactly 56 characters" \
  --local-only --contract-id "CBCGTSCJXBMPPPE4BPDIPYZXPE2J5TQEKD2KCS7VQF533NKKEYGUTHXWA" --wasm "$MOCK_WASM"

check "Contract ID: starts with G instead of C is rejected" \
  1 "Contract ID must start with 'C'" \
  --local-only --contract-id "GBCGTSCJXBMPPPE4BPDIPYZXPE2J5TQEKD2KCS7VQF533NKKEYGUTHXW" --wasm "$MOCK_WASM"

check "Contract ID: invalid base32 characters (e.g. 0, 1, 8, 9) rejected" \
  1 "Contract ID must start with 'C' followed by 55 valid Base32 characters" \
  --local-only --contract-id "CBCGTSCJXBMPPPE4BPDIPYZXPE2J5TQEKD2KCS7VQF533NKKEYGUTH08" --wasm "$MOCK_WASM"

check "Contract ID: lowercase characters rejected" \
  1 "Contract ID must start with 'C' followed by 55 valid Base32 characters" \
  --local-only --contract-id "cbcgtscjXbmpppe4bpdipyzxpe2j5tqekd2kcs7vqf533nkkeyguthxw" --wasm "$MOCK_WASM"

# ── 4. Local Validation: WASM Artifact & Checksum ──────────────────────────────
check "Non-existent WASM file is rejected" \
  1 "Artifact file not found" \
  --local-only --wasm "$TMP_DIR/does_not_exist.wasm"

check "Empty (0-byte) WASM file is rejected" \
  1 "is empty \(0 bytes\)" \
  --local-only --wasm "$EMPTY_WASM"

check "Oversized WASM (> 128 KiB) is rejected" \
  1 "exceeds Soroban 128 KiB limit" \
  --local-only --wasm "$OVERSIZED_WASM"

check "Checksum mismatch is rejected" \
  1 "Artifact checksum mismatch" \
  --local-only --wasm "$MOCK_WASM" --expected-checksum "0000000000000000000000000000000000000000000000000000000000000000"

# ── 5. Local Validation: Initialization Inputs ─────────────────────────────────
check "Token address with invalid length is rejected" \
  1 "Initialization token address length must be exactly 56 characters" \
  --local-only --wasm "$MOCK_WASM" --token "INVALID_TOKEN_ADDRESS"

check "Token address starting with G instead of C is rejected" \
  1 "Initialization token address must start with 'C'" \
  --local-only --wasm "$MOCK_WASM" --token "$VALID_ADMIN_ID"

check "Invalid admin address format is rejected" \
  1 "Initialization admin address length must be exactly 56 characters" \
  --local-only --wasm "$MOCK_WASM" --admin "G_SHORT"

# ── 6. Write Confirmation Guard ───────────────────────────────────────────────
check "Without --confirm-write, execution remains in dry-run mode" \
  0 "DRY-RUN SUCCESSFUL — NO NETWORK MUTATION PERFORMED" \
  --local-only --wasm "$MOCK_WASM"

# ── 7. Retry Idempotency ──────────────────────────────────────────────────────
# Proves that retrying after a failed validation runs cleanly without state residue
check "First attempt with invalid contract ID fails safely" \
  1 "Contract ID length must be exactly 56 characters" \
  --local-only --contract-id "SHORT" --wasm "$MOCK_WASM"

check "Immediate retry with valid contract ID succeeds with zero side-effects" \
  0 "DRY-RUN SUCCESSFUL" \
  --local-only --contract-id "$VALID_CONTRACT_ID" --wasm "$MOCK_WASM"

echo "────────────────────────────────────────────────────────────────────────"
echo " Results: $PASS passed, $FAIL failed"
echo "════════════════════════════════════════════════════════════════════════"

[[ $FAIL -eq 0 ]]
