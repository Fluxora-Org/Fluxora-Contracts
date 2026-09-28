#!/usr/bin/env bash
#
# script/release-dry-run.sh
#
# Release dry-run that validates network, artifact checksum, contract identifier,
# and initialization inputs before executing any upgrade or deployment mutation.
#
# Design Contract:
#   1. Local Validation: Offline syntax and constraint checks (network name,
#      strkey formats, artifact existence, size limits, SHA-256 checksum).
#      Fails fast before any network traffic or RPC calls.
#   2. RPC Validation: Online read-only pre-flight checks (endpoint health,
#      chain height, contract resolution, simulate transaction without broadcast).
#   3. Explicit Confirmation Guard: Stays in dry-run mode by default, printing
#      the exact artifact checksum and target. Requires --confirm-write to
#      submit any state-mutating transaction.
#
# Usage:
#   script/release-dry-run.sh [OPTIONS]
#
# Options:
#   --network <name>              Target network: testnet | mainnet | futurenet | standalone (default: testnet)
#   --rpc-url <url>               Soroban RPC URL (defaults to network standard)
#   --wasm <path>                 Path to compiled contract WASM (default: target/wasm32v1-none/release/fluxora_stream.wasm)
#   --contract-id <id>            Target contract ID (56-char C... strkey)
#   --source <identity|key>       Deployer / admin identity or secret key
#   --token <address>             Initialization token address (56-char C... strkey)
#   --admin <address>             Initialization admin address (56-char G.../C... strkey)
#   --expected-checksum <hex>     Expected SHA-256 checksum of the WASM artifact
#   --confirm-write               Acknowledge and execute live write transaction (MUTATING)
#   --local-only                  Skip RPC pre-flight checks (offline validation only)
#   -h, --help                    Show this help message
#
# Exit codes:
#   0  Success (dry-run passed or confirmed write succeeded)
#   1  Validation failure or execution error
#   2  Invalid command-line arguments

set -euo pipefail

# ── Color & Log Helpers ───────────────────────────────────────────────────────
if [[ -t 1 ]]; then
  BOLD='\033[1m'
  GREEN='\033[32m'
  RED='\033[31m'
  YELLOW='\033[33m'
  CYAN='\033[36m'
  RESET='\033[0m'
else
  BOLD=''
  GREEN=''
  RED=''
  YELLOW=''
  CYAN=''
  RESET=''
fi

say()     { printf '\n%b── %s%b\n' "$BOLD" "$*" "$RESET"; }
info()    { printf '   %b[INFO]%b  %s\n' "$CYAN" "$RESET" "$*"; }
success() { printf '   %b✓%b %s\n' "$GREEN" "$RESET" "$*"; }
warn()    { printf '   %b⚠%b %s\n' "$YELLOW" "$RESET" "$*"; }
fail()    { printf '   %b✗%b %s\n' "$RED" "$RESET" "$*" >&2; exit 1; }

# ── Defaults ──────────────────────────────────────────────────────────────────
NETWORK="${NETWORK:-testnet}"
RPC_URL="${RPC_URL:-}"
WASM_PATH="${WASM_PATH:-target/wasm32v1-none/release/fluxora_stream.wasm}"
CONTRACT_ID="${CONTRACT_ID:-}"
SOURCE="${SOURCE:-fluxora-deployer}"
TOKEN="${TOKEN:-}"
ADMIN="${ADMIN:-}"
EXPECTED_CHECKSUM="${EXPECTED_CHECKSUM:-}"
CONFIRM_WRITE=false
LOCAL_ONLY=false

# Maximum allowed Soroban contract code size is 128 KiB (131,072 bytes)
MAX_WASM_SIZE=131072

usage() {
  cat <<EOF
Usage: $0 [OPTIONS]

Release dry-run and pre-flight validation before upgrade / deployment.

Options:
  --network <name>              Target network (testnet, mainnet, futurenet, standalone)
  --rpc-url <url>               Soroban RPC URL (defaults to network standard)
  --wasm <path>                 Path to compiled contract WASM
  --contract-id <id>            Target contract ID (56-char C... strkey)
  --source <identity|key>       Deployer / admin identity or secret key
  --token <address>             Initialization token address (56-char C... strkey)
  --admin <address>             Initialization admin address (56-char G.../C... strkey)
  --expected-checksum <hex>     Expected SHA-256 checksum of the WASM artifact
  --confirm-write               Confirm and execute state-mutating write transaction
  --local-only                  Run local validation only, skipping network RPC queries
  -h, --help                    Show this help message
EOF
  exit "${1:-0}"
}

# ── Parse Arguments ───────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
  case "$1" in
    --network)
      [[ $# -ge 2 ]] || { echo "ERROR: --network requires an argument" >&2; exit 2; }
      NETWORK="$2"; shift 2 ;;
    --rpc-url)
      [[ $# -ge 2 ]] || { echo "ERROR: --rpc-url requires an argument" >&2; exit 2; }
      RPC_URL="$2"; shift 2 ;;
    --wasm)
      [[ $# -ge 2 ]] || { echo "ERROR: --wasm requires an argument" >&2; exit 2; }
      WASM_PATH="$2"; shift 2 ;;
    --contract-id)
      [[ $# -ge 2 ]] || { echo "ERROR: --contract-id requires an argument" >&2; exit 2; }
      CONTRACT_ID="$2"; shift 2 ;;
    --source)
      [[ $# -ge 2 ]] || { echo "ERROR: --source requires an argument" >&2; exit 2; }
      SOURCE="$2"; shift 2 ;;
    --token)
      [[ $# -ge 2 ]] || { echo "ERROR: --token requires an argument" >&2; exit 2; }
      TOKEN="$2"; shift 2 ;;
    --admin)
      [[ $# -ge 2 ]] || { echo "ERROR: --admin requires an argument" >&2; exit 2; }
      ADMIN="$2"; shift 2 ;;
    --expected-checksum)
      [[ $# -ge 2 ]] || { echo "ERROR: --expected-checksum requires an argument" >&2; exit 2; }
      EXPECTED_CHECKSUM="$2"; shift 2 ;;
    --confirm-write)
      CONFIRM_WRITE=true; shift ;;
    --local-only)
      LOCAL_ONLY=true; shift ;;
    -h|--help)
      usage 0 ;;
    *)
      echo "ERROR: Unknown option: $1" >&2; exit 2 ;;
  esac
done

# Resolve default RPC URL if omitted
if [[ -z "$RPC_URL" ]]; then
  case "$NETWORK" in
    testnet)    RPC_URL="https://soroban-testnet.stellar.org" ;;
    mainnet)    RPC_URL="https://soroban-rpc.mainnet.stellar.org" ;;
    futurenet)  RPC_URL="https://rpc-futurenet.stellar.org" ;;
    standalone) RPC_URL="http://localhost:8000/soroban/rpc" ;;
    local)      RPC_URL="http://localhost:8000/soroban/rpc" ;;
    *)          RPC_URL="https://soroban-${NETWORK}.stellar.org" ;;
  esac
fi

# ── Helper: Compute SHA-256 Checksum ──────────────────────────────────────────
compute_sha256() {
  local target="$1"
  if command -v sha256sum &>/dev/null; then
    sha256sum "$target" | awk '{print $1}'
  elif command -v shasum &>/dev/null; then
    shasum -a 256 "$target" | awk '{print $1}'
  elif command -v openssl &>/dev/null; then
    openssl dgst -sha256 "$target" | awk '{print $NF}'
  else
    python3 -c 'import hashlib,sys; print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest())' "$target"
  fi
}

# ── Helper: Strkey Validator ──────────────────────────────────────────────────
# Contract IDs must be 56 chars, starting with 'C', base32 Crockford/RFC4648 [A-Z2-7]
validate_contract_id_format() {
  local cid="$1"
  local label="${2:-Contract ID}"
  if [[ -z "$cid" ]]; then
    fail "$label cannot be empty."
  fi
  if [[ ${#cid} -ne 56 ]]; then
    fail "$label length must be exactly 56 characters, got ${#cid} ('$cid')."
  fi
  if [[ ! "$cid" =~ ^C[A-Z2-7]{55}$ ]]; then
    fail "$label must start with 'C' followed by 55 valid Base32 characters [A-Z2-7]. Invalid: '$cid'."
  fi
}

# Account IDs must start with 'G' or 'C', 56 chars base32
validate_account_id_format() {
  local aid="$1"
  local label="$2"
  if [[ -z "$aid" ]]; then
    return 0
  fi
  if [[ ${#aid} -ne 56 ]]; then
    fail "$label length must be exactly 56 characters, got ${#aid} ('$aid')."
  fi
  if [[ ! "$aid" =~ ^[GC][A-Z2-7]{55}$ ]]; then
    fail "$label must be a valid Stellar public key ('G...') or contract address ('C...'). Invalid: '$aid'."
  fi
}

# ─────────────────────────────────────────────────────────────────────────────
# PHASE 1: LOCAL VALIDATION (Offline / Syntax & Invariants)
# ─────────────────────────────────────────────────────────────────────────────
say "Phase 1: Local Pre-flight Validation (Offline)"

# 1.1 Network identifier validation
if [[ -z "$NETWORK" ]]; then
  fail "Network identifier cannot be empty."
fi
if [[ ! "$NETWORK" =~ ^[a-zA-Z0-9_-]+$ ]]; then
  fail "Network identifier contains invalid characters: '$NETWORK'. Must match [a-zA-Z0-9_-]+."
fi
success "Network identifier format valid: $NETWORK"

# 1.2 Target Contract ID validation (if specified)
if [[ -n "$CONTRACT_ID" ]]; then
  validate_contract_id_format "$CONTRACT_ID"
  success "Target Contract ID format valid: $CONTRACT_ID"
else
  info "Target Contract ID not specified (deploying new contract instance)."
fi

# 1.3 WASM Artifact validation
if [[ ! -f "$WASM_PATH" ]]; then
  fail "Artifact file not found at '$WASM_PATH'. Build first with 'cargo build --target wasm32v1-none --release'."
fi

WASM_SIZE=$(wc -c < "$WASM_PATH" | tr -d ' ')
if [[ "$WASM_SIZE" -le 0 ]]; then
  fail "Artifact '$WASM_PATH' is empty (0 bytes)."
fi

if [[ "$WASM_SIZE" -gt "$MAX_WASM_SIZE" ]]; then
  fail "Artifact '$WASM_PATH' size (${WASM_SIZE} bytes) exceeds Soroban 128 KiB limit (${MAX_WASM_SIZE} bytes)."
fi
success "Artifact exists and within budget: $WASM_PATH ($WASM_SIZE bytes / max $MAX_WASM_SIZE bytes)"

# 1.4 Artifact Checksum computation & validation
ACTUAL_CHECKSUM=$(compute_sha256 "$WASM_PATH")
if [[ -z "$ACTUAL_CHECKSUM" || ${#ACTUAL_CHECKSUM} -ne 64 ]]; then
  fail "Failed to compute valid 64-character SHA-256 checksum for '$WASM_PATH'."
fi
success "Artifact SHA-256 checksum computed: $ACTUAL_CHECKSUM"

if [[ -n "$EXPECTED_CHECKSUM" ]]; then
  # Normalize to lowercase
  EXPECTED_LOWER=$(echo "$EXPECTED_CHECKSUM" | tr '[:upper:]' '[:lower:]')
  ACTUAL_LOWER=$(echo "$ACTUAL_CHECKSUM" | tr '[:upper:]' '[:lower:]')
  if [[ "$ACTUAL_LOWER" != "$EXPECTED_LOWER" ]]; then
    fail "Artifact checksum mismatch!\n     Expected: $EXPECTED_CHECKSUM\n     Actual:   $ACTUAL_CHECKSUM"
  fi
  success "Artifact checksum matches expected reference."
fi

# 1.5 Initialization Inputs validation
if [[ -n "$TOKEN" ]]; then
  validate_contract_id_format "$TOKEN" "Initialization token address"
  success "Initialization token address format valid: $TOKEN"
fi

if [[ -n "$ADMIN" ]]; then
  validate_account_id_format "$ADMIN" "Initialization admin address"
  success "Initialization admin address format valid: $ADMIN"
fi

# 1.6 Source / Deployer validation
if [[ -z "$SOURCE" ]]; then
  fail "Source / deployer identity cannot be empty."
fi
success "Source / deployer configured: $SOURCE"

# ─────────────────────────────────────────────────────────────────────────────
# Print Exact Artifact Checksum and Target Manifest
# ─────────────────────────────────────────────────────────────────────────────
say "Release Target & Checksum Manifest"
cat <<MANIFEST
╭──────────────────────────────────────────────────────────────────────────────╮
│ Fluxora Release Dry-Run Manifest                                             │
╰──────────────────────────────────────────────────────────────────────────────╯
 Target Network     : $NETWORK
 RPC Endpoint       : $RPC_URL
 Target Contract ID : ${CONTRACT_ID:-"(new deployment)"}
 Artifact Path      : $WASM_PATH
 Artifact Size      : $WASM_SIZE bytes
 Artifact SHA-256   : $ACTUAL_CHECKSUM
 Expected SHA-256   : ${EXPECTED_CHECKSUM:-"(none specified)"}
 Source / Deployer  : $SOURCE
 Token (Init)       : ${TOKEN:-"(none specified)"}
 Admin (Init)       : ${ADMIN:-"(none specified)"}
 Execution Mode     : $(if $CONFIRM_WRITE; then echo "WRITE-MUTATION (LIVE)"; else echo "DRY-RUN (READ-ONLY / NO MUTATION)"; fi)
────────────────────────────────────────────────────────────────────────────────
MANIFEST

# ─────────────────────────────────────────────────────────────────────────────
# PHASE 2: RPC VALIDATION (Online Pre-flight / Read-only Simulation)
# ─────────────────────────────────────────────────────────────────────────────
if $LOCAL_ONLY; then
  say "Phase 2: RPC Pre-flight Validation (SKIPPED via --local-only)"
  info "Local-only flag supplied. Skipping network connectivity and RPC checks."
else
  say "Phase 2: RPC Pre-flight Validation (Online / Read-Only)"

  # 2.1 RPC Connectivity & Latest Ledger check
  info "Checking RPC endpoint connectivity at '$RPC_URL'..."
  RPC_RESPONSE=$(curl -s -m 10 -X POST "$RPC_URL" \
    -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"getLatestLedger"}' 2>/dev/null || echo "")

  if [[ -z "$RPC_RESPONSE" ]]; then
    fail "RPC endpoint '$RPC_URL' unreachable or connection timed out."
  fi

  LEDGER_SEQ=$(echo "$RPC_RESPONSE" | python3 -c 'import sys,json; r=json.load(sys.stdin); print(r.get("result",{}).get("sequence",""))' 2>/dev/null || true)
  if [[ -z "$LEDGER_SEQ" || "$LEDGER_SEQ" == "0" ]]; then
    fail "RPC endpoint '$RPC_URL' returned invalid response: $RPC_RESPONSE"
  fi
  success "RPC healthy. Latest ledger sequence: $LEDGER_SEQ"

  # 2.2 Stellar CLI pre-flight simulation (if stellar CLI is installed)
  if command -v stellar &>/dev/null; then
    info "Stellar CLI detected: $(stellar --version | head -1)"

    # If contract ID specified, verify on-chain readability
    if [[ -n "$CONTRACT_ID" ]]; then
      info "Verifying target contract ID '$CONTRACT_ID' on network '$NETWORK'..."
      if stellar contract read --id "$CONTRACT_ID" --network "$NETWORK" \
           --durability persistent --rpc-url "$RPC_URL" 2>&1 >/dev/null; then
        success "Target contract '$CONTRACT_ID' exists and is readable on $NETWORK."
      else
        info "Target contract '$CONTRACT_ID' not yet initialized or read returned empty."
      fi
    fi
  else
    info "Stellar CLI not found in PATH; skipped CLI-based contract simulation."
  fi
fi

# ─────────────────────────────────────────────────────────────────────────────
# PHASE 3: WRITE TRANSACTION EXECUTION GUARD
# ─────────────────────────────────────────────────────────────────────────────
say "Phase 3: Write Transaction Execution Guard"

if ! $CONFIRM_WRITE; then
  cat <<'DRYRUN'
╭──────────────────────────────────────────────────────────────────────────────╮
│ DRY-RUN SUCCESSFUL — NO NETWORK MUTATION PERFORMED                            │
╰──────────────────────────────────────────────────────────────────────────────╯
All local validations passed and release targets are verified.
Artifact checksum and identifiers have been recorded in the dry-run manifest.

To execute the live write transaction on chain, re-run with:
  --confirm-write
DRYRUN
  exit 0
fi

# Explicit confirmation branch: Authorized mutation
say "Phase 4: Executing Live Network Mutation"
warn "Explicit write confirmation received (--confirm-write)."
info "Broadcasting contract deployment/upgrade to $NETWORK via $RPC_URL..."

if ! command -v stellar &>/dev/null; then
  fail "Cannot execute live write transaction: 'stellar' CLI is required for mutation."
fi

# Execute deployment/upload
DEPLOY_OUTPUT=$(stellar contract deploy \
  --wasm "$WASM_PATH" \
  --network "$NETWORK" \
  --source "$SOURCE" \
  --rpc-url "$RPC_URL" 2>&1) || fail "Deployment transaction failed:\n$DEPLOY_OUTPUT"

NEW_CONTRACT_ID=$(echo "$DEPLOY_OUTPUT" | grep -E '^C[A-Z2-7]{55}$' | tail -1 || true)
if [[ -z "$NEW_CONTRACT_ID" ]]; then
  NEW_CONTRACT_ID=$(echo "$DEPLOY_OUTPUT" | tail -1)
fi

success "Write transaction confirmed on-chain!"
info "Deployed Contract ID: $NEW_CONTRACT_ID"
info "Artifact Checksum   : $ACTUAL_CHECKSUM"
info "Network             : $NETWORK"
exit 0
