# Fluxora Contracts

Soroban smart contracts for the Fluxora treasury streaming protocol on Stellar. Stream USDC from a treasury to recipients over time with configurable rate, duration, and cliff.

## Documentation

- **[Stream contract docs](docs/streaming.md)** — Lifecycle, accrual formula, cliff/end_time, access control, events, and error codes.

## What's in this repo

- **Stream contract** (`contracts/stream`) — Lock USDC, accrue per second, withdraw on demand.
- **Data model** — `Stream` (sender, recipient, deposit_amount, rate_per_second, start/cliff/end time, withdrawn_amount, status).
- **Status** — Active, Paused, Completed, Cancelled.
- **Methods (stubs)** — `init`, `create_stream`, `pause_stream`, `resume_stream`, `cancel_stream`, `withdraw`, `calculate_accrued`, `get_stream_state`.
- **Cancel semantics** — `cancel_stream` is valid only in `Active` or `Paused`; `Completed` and `Cancelled` return `InvalidState`.

Implementation is scaffolded; storage, token transfers, and events are left for you to complete.
- **Methods** — `init`, `create_stream`, `pause_stream`, `resume_stream`, `cancel_stream`, `withdraw`, `calculate_accrued`, `get_stream_state`, `set_admin`.
- **Admin functions** — `pause_stream_as_admin`, `resume_stream_as_admin`, `cancel_stream_as_admin`, `set_admin` for key rotation.

**Documentation:** [Audit preparation](docs/audit.md) (entrypoints and invariants for auditors).

## Tech stack

- Rust (edition 2021)
- [soroban-sdk](https://docs.rs/soroban-sdk) (Stellar Soroban)
- Build target: `wasm32-unknown-unknown` for deployment

## Local development and testing

Everything below runs locally. **No secrets or mainnet access are required** to build or test.

### Clone and prerequisites

```bash
git clone https://github.com/Fluxora-Org/Fluxora-Contracts.git
cd Fluxora-Contracts
```

- **Rust 1.70+** (install from [rustup.rs](https://rustup.rs))
- Add the Soroban build target:

```bash
rustup target add wasm32-unknown-unknown
```

[Stellar CLI](https://developers.stellar.org/docs/tools/developer-tools) is optional and only needed if you want to deploy to testnet later.

### Build

From the repo root:

```bash
# Development build (faster compile, for local testing)
cargo build -p fluxora_stream

# Release build (optimized WASM for deployment)
cargo build --release -p fluxora_stream
```

Release WASM output: `target/wasm32-unknown-unknown/release/fluxora_stream.wasm`.

### Run tests

To run all tests (unit and integration tests), use:

```bash
cargo test -p fluxora_stream
```

This runs **unit tests** and **integration tests** in one go. No environment variables or external services are required. Integration tests use Soroban’s in-process test environment (`soroban_sdk::testutils`): the contract and a mock Stellar asset are built in memory, so no emulator or network is needed.
**Note:** Tests rely on the `testutils` feature of the `soroban-sdk` to simulate the ledger environment and manipulate time (e.g., fast-forwarding to test cliff and end periods). 
This feature is already enabled in `contracts/stream/Cargo.toml` under `[dev-dependencies]`. No extra environment setup is required.

The test files are located at:
- Unit tests: `contracts/stream/src/test.rs`
- Integration tests: `contracts/stream/tests/integration_suite.rs`

- **Unit tests**: `contracts/stream/src/test.rs` (contract logic, accrual, auth, edge cases).
- **Integration tests**: `contracts/stream/tests/integration_suite.rs` — full flows with `init`, `create_stream`, `withdraw`, `get_stream_state`, lifecycle and edge cases (double init, pre-cliff withdraw, unknown stream id, underfunded deposit).

To run only unit tests or only the integration suite:

```bash
cargo test -p fluxora_stream --lib
cargo test -p fluxora_stream --test integration_suite
```

### Optional: deploy to testnet

If you have the Stellar CLI configured (no secrets required in the repo):

```bash
stellar contract deploy \
  --wasm-file target/wasm32-unknown-unknown/release/fluxora_stream.wasm \
  --network testnet
```

Then call `init` with your token and admin addresses, and use `create_stream`, `withdraw`, etc. as needed.

## Project structure

```
fluxora-contracts/
  Cargo.toml              # workspace
  docs/
    storage.md            # storage layout and key design
  contracts/
    stream/
      Cargo.toml
      src/
        lib.rs            # contract types and impl
        test.rs           # unit tests
      tests/
        integration_suite.rs  # integration tests (Soroban testutils)
```

## Documentation

- **[Storage Layout](docs/storage.md)** — Contract storage architecture, key design, and TTL policies

## Accrual formula (reference)

- **Accrued** = `0` when `current_time < cliff_time`
- Otherwise: `min((min(current_time, end_time) - start_time) * rate_per_second, deposit_amount)`
- **Withdrawable** = `Accrued - withdrawn_amount`
- No accrual after `end_time` (elapsed time is capped at `end_time`).

## Related repos

- **fluxora-backend** — API and Horizon sync
- **fluxora-frontend** — Dashboard and recipient UI

Each is a separate Git repository.
