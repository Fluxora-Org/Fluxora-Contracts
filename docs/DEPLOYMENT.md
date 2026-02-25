# Stellar Testnet Deployment Checklist

A quick, step-by-step guide to deploying the Fluxora stream contract to Stellar testnet.

---

## Table of Contents

1. [Prerequisites](#prerequisites)
2. [Build & Deploy](#build--deploy)
3. [Initialize Contract](#initialize-contract)
4. [Verification](#verification)
5. [Network Details](#network-details)
6. [Troubleshooting](#troubleshooting)

---

## Prerequisites

- [ ] **Rust installed** (v1.70+)  
  ```bash
  rustup --version
  rustup target add wasm32-unknown-unknown
  ```

- [ ] **Stellar CLI installed** ([install guide](https://developers.stellar.org/docs/smart-contracts/getting-started/setup))  
  ```bash
  stellar --version
  ```

- [ ] **Testnet account with funds**  
  - Create/fund via [Stellar testnet faucet](https://laboratory.stellar.org/#account-creator?network=test)
  - Funded account = **Deployer account** (holds the contract, cov gas fees)

- [ ] **Environment variables configured**  
  ```bash
  cp .env.example .env
  # Then edit .env with:
  export STELLAR_SECRET_KEY="S..."           # Deployer secret key
  export STELLAR_ADMIN_ADDRESS="G..."        # Admin/treasury public key
  export STELLAR_TOKEN_ADDRESS="C..."        # USDC or test token contract address
  export STELLAR_NETWORK="testnet"           # (optional)
  export STELLAR_RPC_URL="https://soroban-testnet.stellar.org"  # (optional)
  ```

---

## Build & Deploy

### 1. Build the contract

```bash
cargo build --release -p fluxora_stream --target wasm32-unknown-unknown
```

Expected output: `target/wasm32-unknown-unknown/release/fluxora_stream.wasm` (~150 KB)

### 2. Upload & deploy via script (recommended)

The deployment script handles WASM upload, contract deployment, and init in one go:

```bash
source .env
bash script/deploy-testnet.sh
```

**What it does:**
- ✅ Validates env vars and CLI prerequisites
- ✅ Builds the WASM binary
- ✅ Uploads WASM to testnet (idempotent — skips if unchanged)
- ✅ Deploys contract instance (idempotent — skips if already deployed)
- ✅ Invokes `init` to set token and admin
- ✅ Saves contract ID to `.contract_id` for future use

**Output:** Contract ID will be saved to `.contract_id` file (example: `CAHUB4AGDYVQ3G5T3B...`)

### 3. (Alternative) Manual deployment steps

If you prefer to deploy manually:

```bash
# Step 1: Upload WASM
WASM_ID=$(stellar contract upload \
  --wasm target/wasm32-unknown-unknown/release/fluxora_stream.wasm \
  --network testnet \
  --source "$STELLAR_SECRET_KEY" \
  --rpc-url https://soroban-testnet.stellar.org)

# Step 2: Deploy contract
CONTRACT_ID=$(stellar contract deploy \
  --wasm-hash "$WASM_ID" \
  --network testnet \
  --source "$STELLAR_SECRET_KEY" \
  --rpc-url https://soroban-testnet.stellar.org)

# Save for later
echo "$CONTRACT_ID" > .contract_id
```

---

## Initialize Contract

The contract requires `init` to be called exactly once, setting the token address and admin.

### Via deployment script (automatic)

The script calls `init` automatically at the end:

```bash
bash script/deploy-testnet.sh
```

### Manual init invocation

If you deployed manually or need to re-initialize:

```bash
CONTRACT_ID=$(cat .contract_id)  # or use your deployed contract ID

stellar contract invoke \
  --id "$CONTRACT_ID" \
  --network testnet \
  --source "$STELLAR_SECRET_KEY" \
  --rpc-url https://soroban-testnet.stellar.org \
  -- init \
    --token "$STELLAR_TOKEN_ADDRESS" \
    --admin "$STELLAR_ADMIN_ADDRESS"
```

**Note:** `init` can only be called once. Calling it again will fail (by design).

---

## Verification

After deployment, verify the contract is working:

### 1. Read configuration

Check that `init` succeeded by reading the contract config:

```bash
CONTRACT_ID=$(cat .contract_id)

stellar contract invoke \
  --id "$CONTRACT_ID" \
  --network testnet \
  --source "$STELLAR_SECRET_KEY" \
  --rpc-url https://soroban-testnet.stellar.org \
  -- get_config
```

Expected output: `{"token": "C...", "admin": "G..."}`

### 2. Create a test stream

Create a sample stream to verify `create_stream` works:

```bash
stellar contract invoke \
  --id "$CONTRACT_ID" \
  --network testnet \
  --source "$STELLAR_SECRET_KEY" \
  --rpc-url https://soroban-testnet.stellar.org \
  -- create_stream \
    --sender "$STELLAR_ADMIN_ADDRESS" \
    --recipient "GBRPYHIL2CI3WHZDTOOQFC6EB4CGQOFSNQB37HY5SKBRZGTAE3Z5MJGF" \
    --deposit_amount 1000000 \
    --rate_per_second 1000 \
    --cliff_time 1700000000 \
    --end_time 1800000000
```

Expected output: Stream ID (e.g., `0`) printed to console

### 3. Query stream state

```bash
stellar contract invoke \
  --id "$CONTRACT_ID" \
  --network testnet \
  --source "$STELLAR_SECRET_KEY" \
  --rpc-url https://soroban-testnet.stellar.org \
  -- get_stream_state \
    --stream_id 0
```

Expected output:
```json
{
  "sender": "G...",
  "recipient": "G...",
  "deposit_amount": 1000000,
  "rate_per_second": 1000,
  "start_time": ...,
  "cliff_time": ...,
  "end_time": ...,
  "withdrawn_amount": 0,
  "status": "Active"
}
```

---

## Network Details

### Stellar Testnet RPC

- **RPC URL:** `https://soroban-testnet.stellar.org`
- **Network Passphrase:** `Test SDF Network ; September 2015`
- **Network ID:** `testnet`

The deployment script uses the RPC URL and network name automatically. No manual configuration needed unless you override with `STELLAR_RPC_URL`.

### Viewing contracts on Explorer

After deployment, you can view your contract on the **Stellar testnet explorer**:

- [Stellar Expert Testnet](https://testnet.stellar.expert/)
- Search for your **Contract ID** (from `.contract_id`)

---

## Troubleshooting

### ❌ `STELLAR_SECRET_KEY not set`

**Problem:** Deployment script exits with env var error.

**Solution:**
```bash
export STELLAR_SECRET_KEY="S..."  # or source .env
```

### ❌ `stellar CLI not found`

**Problem:** CLI is not installed or not in PATH.

**Solution:**
```bash
# Install Stellar CLI
# https://developers.stellar.org/docs/smart-contracts/getting-started/setup

# Verify installation
stellar --version
```

### ❌ `Contract deploy failed — no contract ID returned`

**Problem:** Contract deployment failed, usually due to insufficient funds or RPC timeout.

**Solution:**
- Verify your deployer account has funds:
  ```bash
  stellar account info --network testnet --source "$STELLAR_SECRET_KEY"
  ```
- Re-run the deployment script (idempotency should retry the deploy)
- Check RPC status: `curl https://soroban-testnet.stellar.org/health`

### ❌ `init may have already been called`

**Problem:** `init` fails because it's already been called.

**Solution:**
- This is expected behavior. `init` can only run once.
- Verify with `get_config`. If it returns token and admin, `init` succeeded.

### ❌ `WASM binary unchanged` but deploy failed

**Problem:** Script skips WASM re-upload but you want to force a fresh upload.

**Solution:**
```bash
rm .wasm_id .wasm_id.sha256 .contract_id
bash script/deploy-testnet.sh
```

This forces a fresh WASM upload and new contract deployment.

### ❌ `RPC timeout or slow` responses

**Problem:** Testnet RPC is slow or unresponsive.

**Solution:**
- Check RPC status: https://status.stellar.org/
- Temporarily use alternative RPC (if available)
- Retry the deployment after a few minutes

---

## Summary

| Step | Command |
|---|---|
| **Setup** | `cp .env.example .env` → fill in env vars |
| **Build** | `cargo build --release -p fluxora_stream --target wasm32-unknown-unknown` |
| **Deploy** | `bash script/deploy-testnet.sh` |
| **Verify** | `stellar contract invoke --id $(cat .contract_id) -- get_config` |
| **Test stream** | `stellar contract invoke --id $(cat .contract_id) -- create_stream ...` |

---

## Next Steps

After successful deployment:

1. **Fund test accounts** for stream recipients via [testnet faucet](https://laboratory.stellar.org/#account-creator?network=test)
2. **Create streams** with realistic test data (senders, recipients, amounts, durations)
3. **Monitor accrual** by calling `get_stream_state` at different times
4. **Test withdrawals** via the `withdraw` method
5. **Pause/resume/cancel** streams to verify state transitions

---

## Related Documentation

- [Stellar Soroban Docs](https://developers.stellar.org/docs/smart-contracts)
- [Soroban CLI Reference](https://developers.stellar.org/docs/smart-contracts/guides/cli)
- [Fluxora README](../README.md)
- [Deployment Script](../script/deploy-testnet.sh)
