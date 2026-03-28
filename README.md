# Fluxora Contracts

Soroban smart contracts for the Fluxora treasury streaming protocol on Stellar. Stream USDC from a treasury to recipients over time with configurable rate, duration, and cliff.

## Documentation

- **[Stream contract](docs/streaming.md)** — Lifecycle, accrual formula, cliff/end_time, access control, events, and error codes.
- **[Security](docs/security.md)** — CEI ordering, token trust model, authorization paths, overflow protection.
- **[Upgrade strategy](docs/upgrade.md)** — CONTRACT_VERSION policy, breaking-change classification, migration runbook.
- **[Deployment](docs/DEPLOYMENT.md)** — Step-by-step testnet deployment checklist.
- **[Storage layout](docs/storage.md)** — Contract storage architecture, key design, and TTL policies.
- **[Audit preparation](docs/audit.md)** — Entry-points and invariants for auditors.
- **[Error codes](docs/error.md)** — Full ContractError reference.
- **[Events](docs/events.md)** — Emitted event shapes and topics.

## What's in this repo

- **Stream contract** (`contracts/stream`) — Lock USDC, accrue per second, withdraw on demand.
- **Data model** — `Stream` (sender, recipient, deposit_amount, rate_per_second, start/cliff/end time, withdrawn_amount, status, cancelled_at).
- **Status** — `Active`, `Paused`, `Completed`, `Cancelled`.

### Implemented entry-points

| Entry-point | Caller | Description |
|---|---|---|
| `init` | Bootstrap admin | One-shot initialisation: set token and admin |
| `create_stream` | Sender | Create a single stream and pull deposit |
| `create_streams` | Sender | Batch-create streams in one atomic transaction |
| `pause_stream` | Sender | Halt withdrawals (accrual continues) |
| `resume_stream` | Sender | Re-enable withdrawals |
| `cancel_stream` | Sender | Terminate stream; refund unstreamed tokens to sender |
| `withdraw` | Recipient | Pull accrued tokens to recipient address |
| `withdraw_to` | Recipient | Pull accrued tokens to a specified destination |
| `batch_withdraw` | Recipient | Withdraw from multiple streams in one call |
| `top_up_stream` | Any funder | Add tokens to an existing stream's deposit |
| `update_rate_per_second` | Sender | Change the streaming rate |
| `shorten_stream_end_time` | Sender | Move end_time earlier; refunds excess deposit |
| `extend_stream_end_time` | Sender | Move end_time later |
| `close_completed_stream` | Anyone | Remove a completed stream from storage (cleanup) |
| `pause_stream_as_admin` | Admin | Administrative pause override |
| `resume_stream_as_admin` | Admin | Administrative resume override |
| `cancel_stream_as_admin` | Admin | Administrative cancel override |
| `set_admin` | Admin | Rotate the admin key |
| `set_contract_paused` | Admin | Global emergency pause (blocks new stream creation) |
| `calculate_accrued` | Anyone | View: accrued amount at current ledger time |
| `get_withdrawable` | Anyone | View: withdrawable amount at current ledger time |
| `get_claimable_at` | Anyone | View: simulated claimable amount at an arbitrary timestamp |
| `get_stream_state` | Anyone | View: full stream struct |
| `get_stream_count` | Anyone | View: total number of streams created |
| `get_recipient_streams` | Anyone | View: all stream IDs for a recipient (sorted) |
| `get_recipient_stream_count` | Anyone | View: number of streams for a recipient |
| `get_config` | Anyone | View: token address and admin address |
| `version` | Anyone | View: CONTRACT_VERSION constant (pre-flight check) |

### Cancel semantics

`cancel_stream` and `cancel_stream_as_admin` are valid only when status is `Active` or `Paused`. Streams in `Completed` or `Cancelled` state return `ContractError::InvalidState`. After cancellation, accrual is frozen at `cancelled_at`; the recipient may still withdraw the frozen accrued amount.

## Tech stack

- Rust (edition 2021)
- [soroban-sdk](https://docs.rs/soroban-sdk) 21.7.7 (Stellar Soroban)
- Build target: `wasm32-unknown-unknown` for deployment

## Version pinning

This project pins dependencies for **reproducible builds** and **auditor compatibility**:

| Component       | Version | Location                      | Purpose                                          |
| --------------- | ------- | ----------------------------- | ------------------------------------------------ |
| **Rust**        | 1.75    | `rust-toolchain.toml`         | Ensures consistent WASM compilation              |
| **soroban-sdk** | 21.7.7  | `contracts/stream/Cargo.toml` | Locked to tested Stellar Soroban network version |

When upgrading versions:

1. Update `rust-toolchain.toml` → run `rustup update` → rebuild and test
2. Update `soroban-sdk` version in `Cargo.toml` → update lock file → run full test suite
3. Verify compatibility with the target Stellar network (testnet, mainnet)
4. Document the change in the PR or release notes

## Local setup

### Clone and prerequisites

```bash
git clone https://github.com/Fluxora-Org/Fluxora-Contracts.git
cd Fluxora-Contracts
```

- **Rust 1.75+** — Pinned in `rust-toolchain.toml` (auto-enforced via `rustup`)
- **Soroban SDK 21.7.7** — Pinned in `contracts/stream/Cargo.toml` for reproducible builds
- [Stellar CLI](https://developers.stellar.org/docs/tools/developer-tools) (optional, for deploy/test on network)

Install dependencies:

```bash
rustup update stable
rustup target add wasm32-unknown-unknown
```

Then verify:

```bash
rustc --version       # Should show 1.75 or newer
cargo --version
stellar --version     # Only if installing Stellar CLI
```

### Build

From the repo root:

```bash
# Development build (faster compile, for local testing)
cargo build -p fluxora_stream

# Release build (optimized WASM for deployment)
cargo build --release -p fluxora_stream --target wasm32-unknown-unknown
```

Release WASM output: `target/wasm32-unknown-unknown/release/fluxora_stream.wasm`.

### Run tests

```bash
cargo test -p fluxora_stream
```

Runs all unit and integration tests. No environment variables or external services required — Soroban's in-process test environment (`soroban_sdk::testutils`) simulates the ledger and a mock Stellar asset in memory.

Test files:

- **Unit tests**: `contracts/stream/src/test.rs` — contract logic, accrual math, auth, edge cases, i128 boundary scenarios, version policy.
- **Integration tests**: `contracts/stream/tests/integration_suite.rs` — full flows with `init`, `create_stream`, `withdraw`, `get_stream_state`, lifecycle transitions, and edge cases.

### Deploy to Stellar Testnet

> **See [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) for the complete step-by-step deployment checklist.**

Quick start:

```bash
cp .env.example .env
# Edit .env with your STELLAR_SECRET_KEY, STELLAR_TOKEN_ADDRESS, STELLAR_ADMIN_ADDRESS

source .env
bash script/deploy-testnet.sh
```

Contract ID is saved to `.contract_id`. Verify the deployment with:

```bash
stellar contract invoke --id $(cat .contract_id) -- version
stellar contract invoke --id $(cat .contract_id) -- get_config
```

## Project structure

```
fluxora-contracts/
  Cargo.toml                        # workspace
  rust-toolchain.toml               # pinned Rust version
  contracts/
    stream/
      Cargo.toml
      src/
        lib.rs                      # contract types, storage, and all entry-points
        accrual.rs                  # pure accrual math (calculate_accrued_amount)
        test.rs                     # unit tests
      tests/
        integration_suite.rs        # integration tests (Soroban testutils)
  docs/
    streaming.md                    # lifecycle, accrual, access control, events
    security.md                     # CEI ordering, token trust model, auth paths
    upgrade.md                      # CONTRACT_VERSION policy and migration runbook
    storage.md                      # storage layout and TTL policies
    audit.md                        # entry-points and invariants for auditors
    error.md                        # ContractError reference
    events.md                       # emitted event shapes
    DEPLOYMENT.md                   # testnet deployment checklist
    gas.md                          # gas and budget notes
  script/
    deploy-testnet.sh               # automated testnet deployment script
```

## Accrual formula (reference)

```
if current_time < cliff_time  →  0
else  →  min((min(current_time, end_time) − start_time) × rate_per_second, deposit_amount)
```

- **Withdrawable** = `accrued − withdrawn_amount`
- Accrual is capped at `end_time` — no extra accrual after the stream ends.
- Multiplication overflow in accrual falls back to `deposit_amount` (safe upper bound).
- Cancelled streams: accrual is frozen at `cancelled_at`.
- Completed streams: `calculate_accrued` returns `deposit_amount` deterministically.

## WASM build hash verification

After each CI build, the pipeline computes a SHA256 hash of the contract WASM artifact and uploads it as a CI artifact. This allows deployers and auditors to verify that the deployed contract matches the tested build.

To verify a deployment:

1. Download the hash artifact from the CI run (GitHub Actions → Artifacts → `fluxora_stream-wasm-hash`).
2. Compute the SHA256 hash of your local WASM file:
   ```bash
   sha256sum target/wasm32-unknown-unknown/release/fluxora_stream.wasm
   ```
3. Compare the output to the CI hash file. A match confirms your binary is identical to the tested build.

## Related repos

- **fluxora-backend** — API and Horizon sync
- **fluxora-frontend** — Dashboard and recipient UI

Each is a separate Git repository.

## Contributing

Please read [CONTRIBUTING.md](CONTRIBUTING.md) for details on the development workflow, branch naming, and testing requirements (including the 95% test coverage standard).
