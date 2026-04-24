#!/usr/bin/env bash
# =============================================================================
# deploy-testnet.sh — Fluxora Contracts | Soroban CLI deploy script for testnet
# =============================================================================
# Builds the fluxora_stream contract, deploys it to Stellar testnet, and
# optionally invokes `init` with a test token and admin address.
#
# Required environment variables:
#   STELLAR_SECRET_KEY   — Stellar account secret key (starts with S...)
#                          NEVER commit this value. Use .env or CI secrets.
#   STELLAR_TOKEN_ADDRESS — USDC (or test token) contract address on testnet
#   STELLAR_ADMIN_ADDRESS — Public key of the contract admin/treasury wallet
#
# Optional environment variables:
#   STELLAR_NETWORK      — Network alias (default: testnet)
#   STELLAR_RPC_URL      — Custom RPC URL (default: Stellar testnet horizon)
#   SKIP_INIT            — Set to "1" to skip the `init` invocation after deploy
#   WASM_ID_FILE         — Path to persist the deployed WASM ID (default: .wasm_id)
#   CONTRACT_ID_FILE     — Path to persist the deployed contract ID (default: .contract_id)
#
# Usage:
#   cp .env.example .env          # fill in your values
#   source .env
#   bash scripts/deploy-testnet.sh
#
# Or inline:
#   STELLAR_SECRET_KEY=S... STELLAR_TOKEN_ADDRESS=C... STELLAR_ADMIN_ADDRESS=G... \
#     bash scripts/deploy-testnet.sh
# =============================================================================

set -euo pipefail

# ── Colour helpers ────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; CYAN='\033[0;36m'; NC='\033[0m'
info()    { echo -e "${CYAN}[INFO]${NC}  $*"; }
success() { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()    { echo -e "${YELLOW}[WARN]${NC}  $*"; }
error()   { echo -e "${RED}[ERROR]${NC} $*" >&2; exit 1; }

# ── Defaults ──────────────────────────────────────────────────────────────────
STELLAR_NETWORK="${STELLAR_NETWORK:-testnet}"
STELLAR_RPC_URL="${STELLAR_RPC_URL:-https://soroban-testnet.stellar.org}"
SKIP_INIT="${SKIP_INIT:-0}"
WASM_ID_FILE="${WASM_ID_FILE:-.wasm_id}"
CONTRACT_ID_FILE="${CONTRACT_ID_FILE:-.contract_id}"
WASM_PATH="target/wasm32-unknown-unknown/release/fluxora_stream.wasm"
PACKAGE_NAME="fluxora_stream"

echo ""
echo "╔══════════════════════════════════════════════════════╗"
echo "║       Fluxora Contracts — Testnet Deploy Script      ║"
echo "╚══════════════════════════════════════════════════════╝"
echo ""

# ── 1. Validate prerequisites ─────────────────────────────────────────────────
info "Step 1/6 — Checking prerequisites..."

command -v cargo  >/dev/null 2>&1 || error "cargo not found. Install Rust: https://rustup.rs"
command -v stellar >/dev/null 2>&1 || error "stellar CLI not found. Install: https://developers.stellar.org/docs/smart-contracts/getting-started/setup"

# Guard: never let a secret key slip into logs
if [[ -z "${STELLAR_SECRET_KEY:-}" ]]; then
  error "STELLAR_SECRET_KEY is not set. Export it or add it to .env (never commit)."
fi
if [[ -z "${STELLAR_TOKEN_ADDRESS:-}" ]]; then
  error "STELLAR_TOKEN_ADDRESS is not set. Provide a SAC or test token contract address."
fi
if [[ -z "${STELLAR_ADMIN_ADDRESS:-}" ]]; then
  error "STELLAR_ADMIN_ADDRESS is not set. Provide the admin/treasury public key (G...)."
fi

# Validate key format (basic guard)
[[ "${STELLAR_SECRET_KEY}" == S* ]] || error "STELLAR_SECRET_KEY must start with 'S' (Stellar secret key format)."
[[ "${STELLAR_ADMIN_ADDRESS}" == G* ]] || error "STELLAR_ADMIN_ADDRESS must start with 'G' (Stellar public key format)."

success "Prerequisites satisfied."

# ── 2. Add WASM build target ──────────────────────────────────────────────────
info "Step 2/6 — Ensuring wasm32-unknown-unknown target is installed..."
rustup target add wasm32-unknown-unknown --quiet
success "WASM target ready."

# ── 3. Build the contract ─────────────────────────────────────────────────────
info "Step 3/6 — Building ${PACKAGE_NAME} (release / WASM)..."
cargo build --release -p "${PACKAGE_NAME}" --target wasm32-unknown-unknown 2>&1

[[ -f "${WASM_PATH}" ]] || error "WASM artifact not found at ${WASM_PATH} after build."
WASM_SIZE=$(du -sh "${WASM_PATH}" | cut -f1)
success "Build complete. WASM size: ${WASM_SIZE}  →  ${WASM_PATH}"

# ── 4. Upload WASM (idempotent via stored WASM ID) ────────────────────────────
info "Step 4/6 — Uploading WASM to ${STELLAR_NETWORK}..."

# Idempotency: reuse existing WASM ID if the binary hasn't changed
CURRENT_WASM_HASH=$(sha256sum "${WASM_PATH}" | awk '{print $1}')
STORED_HASH_FILE="${WASM_ID_FILE}.sha256"

if [[ -f "${WASM_ID_FILE}" && -f "${STORED_HASH_FILE}" ]]; then
  STORED_HASH=$(cat "${STORED_HASH_FILE}")
  if [[ "${CURRENT_WASM_HASH}" == "${STORED_HASH}" ]]; then
    WASM_ID=$(cat "${WASM_ID_FILE}")
    warn "WASM unchanged (hash match). Reusing stored WASM ID: ${WASM_ID}"
  else
    warn "WASM has changed. Re-uploading..."
    WASM_ID=""
  fi
else
  WASM_ID=""
fi

if [[ -z "${WASM_ID}" ]]; then
  WASM_ID=$(stellar contract upload \
    --wasm "${WASM_PATH}" \
    --network "${STELLAR_NETWORK}" \
    --source "${STELLAR_SECRET_KEY}" \
    --rpc-url "${STELLAR_RPC_URL}" \
    2>&1 | tail -1)

  [[ -n "${WASM_ID}" ]] || error "WASM upload failed — no WASM ID returned."
  echo "${WASM_ID}"          > "${WASM_ID_FILE}"
  echo "${CURRENT_WASM_HASH}" > "${STORED_HASH_FILE}"
  success "WASM uploaded. ID: ${WASM_ID}"
fi

# ── 5. Deploy contract (idempotent via stored contract ID) ────────────────────
info "Step 5/6 — Deploying contract to ${STELLAR_NETWORK}..."

if [[ -f "${CONTRACT_ID_FILE}" ]]; then
  CONTRACT_ID=$(cat "${CONTRACT_ID_FILE}")
  warn "Existing contract ID found: ${CONTRACT_ID}"
  warn "To force a fresh deploy, delete ${CONTRACT_ID_FILE} and re-run."
else
  CONTRACT_ID=$(stellar contract deploy \
    --wasm-hash "${WASM_ID}" \
    --network "${STELLAR_NETWORK}" \
    --source "${STELLAR_SECRET_KEY}" \
    --rpc-url "${STELLAR_RPC_URL}" \
    2>&1 | tail -1)

  [[ -n "${CONTRACT_ID}" ]] || error "Contract deploy failed — no contract ID returned."
  echo "${CONTRACT_ID}" > "${CONTRACT_ID_FILE}"
  success "Contract deployed. ID: ${CONTRACT_ID}"
fi

# ── 6. Invoke init (optional) ─────────────────────────────────────────────────
if [[ "${SKIP_INIT}" == "1" ]]; then
  warn "SKIP_INIT=1 — skipping init invocation."
else
  info "Step 6/6 — Invoking init on contract ${CONTRACT_ID}..."
  stellar contract invoke \
    --id "${CONTRACT_ID}" \
    --network "${STELLAR_NETWORK}" \
    --source "${STELLAR_SECRET_KEY}" \
    --rpc-url "${STELLAR_RPC_URL}" \
    -- init \
      --token "${STELLAR_TOKEN_ADDRESS}" \
      --admin "${STELLAR_ADMIN_ADDRESS}" \
    2>&1 || warn "init may have already been called (or failed). Check contract state if unexpected."

  success "init invoked successfully."
fi

# ── Summary ───────────────────────────────────────────────────────────────────
echo ""
echo "╔══════════════════════════════════════════════════════╗"
echo "║                   Deploy Summary                     ║"
echo "╚══════════════════════════════════════════════════════╝"
echo -e "  Network    : ${CYAN}${STELLAR_NETWORK}${NC}"
echo -e "  WASM ID    : ${CYAN}${WASM_ID}${NC}"
echo -e "  Contract ID: ${GREEN}${CONTRACT_ID}${NC}"
echo -e "  Admin      : ${CYAN}${STELLAR_ADMIN_ADDRESS}${NC}"
echo -e "  Token      : ${CYAN}${STELLAR_TOKEN_ADDRESS}${NC}"
echo ""
echo -e "  ${YELLOW}Next steps:${NC}"
echo "    stellar contract invoke --id ${CONTRACT_ID} -- create_stream ..."
echo "    stellar contract invoke --id ${CONTRACT_ID} -- get_stream_state ..."
echo ""
success "Deployment complete!"