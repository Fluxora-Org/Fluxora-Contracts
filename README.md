# Fluxora Contracts

Soroban smart contracts for the Fluxora treasury streaming protocol on Stellar. Stream USDC from a treasury to recipients over time with configurable rate, duration, and cliff.

## Documentation

- **[Stream contract docs](docs/streaming.md)** — Lifecycle, accrual formula, cliff/end_time, access control, events, and error codes.

## What's in this repo

- **Stream contract** (`contracts/stream`) — Lock USDC, accrue per second, withdraw on demand.
- **Data model** — `Stream` (sender, recipient, deposit_amount, rate_per_second, start/cliff/end time, withdrawn_amount, status).
- **Status** — Active, Paused, Completed, Cancelled.
- **Methods** — `init`, `create_stream`, `pause_stream`, `resume_stream`, `cancel_stream`, `withdraw`, `calculate_accrued`, `get_stream_state`, `set_admin`.
- **Admin functions** — `pause_stream_as_admin`, `resume_stream_as_admin`, `cancel_stream_as_admin`, `set_admin` for key rotation.

## Tech stack

- Rust (edition 2021)
- [soroban-sdk](https://docs.rs/soroban-sdk) (Stellar Soroban)
- Build target: `wasm32-unknown-unknown` for deployment

## Local setup

### Prerequisites

- Rust 1.70+
- [Stellar CLI](https://developers.stellar.org/docs/tools/developer-tools) (optional, for deploy/test on network)

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

To run all tests (unit and integration tests), use:

```bash
cargo test -p fluxora_stream
```

**Note:** Tests rely on the `testutils` feature of the `soroban-sdk` to simulate the ledger environment and manipulate time (e.g., fast-forwarding to test cliff and end periods). 
This feature is already enabled in `contracts/stream/Cargo.toml` under `[dev-dependencies]`. No extra environment setup is required.

The test files are located at:
- Unit tests: `contracts/stream/src/test.rs`
- Integration tests: `contracts/stream/tests/integration_suite.rs`

The integration suite invokes the contract with Soroban `testutils` and covers:
- `init`
- `create_stream`
- `withdraw`
- `get_stream_state`
- A full stream lifecycle from create to completed withdrawal
- Key edge cases (`init` twice, pre-cliff withdrawal, unknown stream id, underfunded deposit)

### Deploy (after Stellar CLI setup)

```bash
stellar contract deploy \
  --wasm-file target/wasm32-unknown-unknown/release/fluxora_stream.wasm \
  --network testnet
```

Then invoke `init` with token and admin addresses, and use `create_stream`, `withdraw`, etc. as needed.

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

- **Accrued** = `min((current_time - start_time) * rate_per_second, deposit_amount)`
- **Withdrawable** = `Accrued - withdrawn_amount`
- Before `cliff_time`: withdrawable = 0.

## Related repos

- **fluxora-backend** — API and Horizon sync
- **fluxora-frontend** — Dashboard and recipient UI

Each is a separate Git repository.
