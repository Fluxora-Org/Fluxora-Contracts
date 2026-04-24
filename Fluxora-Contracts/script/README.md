# Fluxora Contracts

Soroban smart contracts for the Fluxora treasury streaming protocol on Stellar.
Stream USDC from a treasury to recipients over time with configurable rate, duration, and cliff.

---

## What's in this repo

- **Stream contract** (`contracts/stream`) — Lock USDC, accrue per second, withdraw on demand.
- **Data model** — `Stream` (sender, recipient, deposit_amount, rate_per_second, start/cliff/end time, withdrawn_amount, status).
- **Status** — Active, Paused, Completed, Cancelled.
- **Methods (stubs)** — `init`, `create_stream`, `pause_stream`, `resume_stream`, `cancel_stream`, `withdraw`, `calculate_accrued`, `get_stream_state`.

Implementation is scaffolded; storage, token transfers, and events are left for you to complete.

---

## Tech stack

- Rust (edition 2021)
- **soroban-sdk** (Stellar Soroban)
- Build target: `wasm32-unknown-unknown` for deployment

---

## Project structure

```
fluxora-contracts/
  Cargo.toml                        # workspace
  .env.example                      # env var template (copy → .env, never commit)
  scripts/
    deploy-testnet.sh               # ← build + deploy + init script (see below)
  contracts/
    stream/
      Cargo.toml
      src/
        lib.rs                      # contract types and impl
        test.rs                     # unit tests
      tests/
        integration_suite.rs        # integration tests (Soroban testutils)
```

---

## Local setup

### Prerequisites

- Rust 1.70+
- **Stellar CLI** — [install guide](https://developers.stellar.org/docs/smart-contracts/getting-started/setup)

```bash
rustup target add wasm32-unknown-unknown
```

### Build

From the repo root:

```bash
cargo build --release -p fluxora_stream
```

WASM output is under `target/wasm32-unknown-unknown/release/fluxora_stream.wasm`.

### Test

```bash
cargo test -p fluxora_stream
```

This runs both:

- Unit tests in `contracts/stream/src/test.rs`
- Integration tests in `contracts/stream/tests/integration_suite.rs`

The integration suite invokes the contract with Soroban `testutils` and covers:

- `init`
- `create_stream`
- `withdraw`
- `get_stream_state`
- A full stream lifecycle from create to completed withdrawal
- Key edge cases (`init` twice, pre-cliff withdrawal, unknown stream id, underfunded deposit)

---

## Deploy to Testnet

> **Security note:** Never commit secret keys. Use `.env` locally or CI secrets in production pipelines.

### 1. Configure environment variables

```bash
cp .env.example .env
# Open .env and fill in the three required values
```

| Variable | Required | Description |
|---|---|---|
| `STELLAR_SECRET_KEY` | ✅ | Stellar account secret key (`S...`). Fund via [testnet faucet](https://laboratory.stellar.org/#account-creator?network=test). |
| `STELLAR_TOKEN_ADDRESS` | ✅ | Contract address of the token (USDC-SAC or test token) on testnet. |
| `STELLAR_ADMIN_ADDRESS` | ✅ | Public key (`G...`) of the admin/treasury wallet. |
| `STELLAR_NETWORK` | optional | Network alias (default: `testnet`). |
| `STELLAR_RPC_URL` | optional | Custom RPC endpoint (default: `https://soroban-testnet.stellar.org`). |
| `SKIP_INIT` | optional | Set to `1` to skip the automatic `init` invocation after deploy. |
| `WASM_ID_FILE` | optional | File to persist the uploaded WASM ID (default: `.wasm_id`). |
| `CONTRACT_ID_FILE` | optional | File to persist the deployed contract ID (default: `.contract_id`). |

### 2. Run the deploy script

```bash
source .env
bash scripts/deploy-testnet.sh
```

The script will:

1. Validate all required env vars and CLI prerequisites.
2. Ensure the `wasm32-unknown-unknown` target is installed.
3. Build the contract in release mode.
4. Upload the WASM binary to testnet (**idempotent** — skips re-upload if the binary hash is unchanged).
5. Deploy the contract (**idempotent** — skips re-deploy if `.contract_id` already exists).
6. Invoke `init` with your token and admin address.

### 3. Idempotency

The script stores the deployed WASM ID in `.wasm_id` and the contract ID in `.contract_id` (both git-ignored). Re-running the script:

- Will **skip** the WASM upload if the binary has not changed (SHA-256 match).
- Will **skip** the contract deploy if `.contract_id` already exists.
- To force a full re-deploy, delete `.wasm_id`, `.wasm_id.sha256`, and `.contract_id`.

### 4. Skip init (if already initialised)

```bash
SKIP_INIT=1 bash scripts/deploy-testnet.sh
```

### 5. Invoke contract methods after deploy

```bash
# Read the saved contract ID
CONTRACT_ID=$(cat .contract_id)

# Create a stream
stellar contract invoke --id "$CONTRACT_ID" --network testnet --source "$STELLAR_SECRET_KEY" \
  -- create_stream \
     --sender  <G_SENDER_ADDRESS> \
     --recipient <G_RECIPIENT_ADDRESS> \
     --deposit_amount 1000000 \
     --rate_per_second 100 \
     --cliff_time 1700000000 \
     --end_time 1700086400

# Get stream state
stellar contract invoke --id "$CONTRACT_ID" --network testnet --source "$STELLAR_SECRET_KEY" \
  -- get_stream_state --stream_id 0

# Withdraw accrued amount
stellar contract invoke --id "$CONTRACT_ID" --network testnet --source "$STELLAR_SECRET_KEY" \
  -- withdraw --stream_id 0
```

---

## Accrual formula (reference)

```
Accrued      = min((current_time - start_time) × rate_per_second, deposit_amount)
Withdrawable = Accrued - withdrawn_amount
               → 0 before cliff_time
```

---

## Related repos

- [fluxora-backend](https://github.com/Fluxora-Org/fluxora-backend) — API and Horizon sync
- [fluxora-frontend](https://github.com/Fluxora-Org/fluxora-frontend) — Dashboard and recipient UI

Each is a separate Git repository.