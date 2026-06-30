# Fluxora Contracts

Soroban smart contracts for the Fluxora treasury streaming protocol on Stellar. Stream USDC from a treasury to recipients over time with configurable rate, duration, and cliff.

## Documentation



- **[Stream contract](docs/streaming.md)** — Lifecycle, accrual formula, cliff/end_time, access control, events, and error codes.
- **[Dust threshold](docs/dust-threshold.md)** — `withdraw_dust_threshold` formula, USDC examples, validation table, and template guidance.
- **[Security](docs/security.md)** — CEI ordering, token trust model, authorization paths, overflow protection.
- **[Upgrade strategy](docs/upgrade.md)** — CONTRACT_VERSION policy, breaking-change classification, migration runbook.
- **[Deployment](docs/DEPLOYMENT.md)** — Step-by-step testnet deployment checklist.
- **[Recipient stream index](docs/recipient-stream-index.md)** — `get_recipient_streams` page cap, full-enumeration pattern, and DoS-prevention rationale.
- **[Storage layout](docs/storage.md)** — Contract storage architecture, key design, and TTL policies.
- **[Audit preparation](docs/audit.md)** — Entry-points and invariants for auditors.
- **[Error codes](docs/error.md)** — Full ContractError reference.
- **[Events](docs/events.md)** — Emitted event shapes and topics.

## What's in this repo

- **Stream contract** (`contracts/stream`) — Lock USDC, accrue per second, withdraw on demand.
- **Data model** — `Stream` (sender, recipient, deposit_amount, rate_per_second, start/cliff/end time, withdrawn_amount, status, cancelled_at).
- **Status** — `Active`, `Paused`, `Completed`, `Cancelled`.

### Core Stream Entry Points

The following table lists every public stream contract entry point implemented in `contracts/stream/src/lib.rs` inside the `FluxoraStream` `#[contractimpl]` block.

| Entry Point | Caller / Auth Rules | Description |
| :--- | :--- | :--- |
| `init` | `admin.require_auth()` | Initialize contract config with token and admin |
| `create_stream` | `sender.require_auth()` | Create one new stream with explicit absolute timing |
| `create_stream_relative` | `sender.require_auth()` (via `create_stream`) | Create a stream with relative delays instead of absolute timestamps |
| `create_streams` | `sender.require_auth()` | Create a batch of streams in one atomic call |
| `create_streams_relative` | `sender.require_auth()` (via `create_streams`) | Create a batch of streams using relative timing |
| `create_streams_partial` | `sender.require_auth()` | Create a batch of streams with per-entry failure isolation |
| `pause_stream` | `stream.sender.require_auth()` | Pause a sender-owned stream |
| `resume_stream` | `stream.sender.require_auth()` | Resume a sender-owned paused stream |
| `cancel_stream` | `stream.sender.require_auth()` | Cancel a sender-owned stream and refund unstreamed deposit |
| `withdraw` | `stream.recipient.require_auth()` | Withdraw accrued tokens as the stream recipient |
| `withdraw_to` | `stream.recipient.require_auth()` | Withdraw accrued tokens to a specified destination as recipient |
| `update_recipient` | `stream.recipient.require_auth()` | Rotate stream recipient to a new address |
| `get_pending_recipient_update` | Public / None | Read the pending recipient update request |
| `accept_recipient_update` | `stream.recipient.require_auth()` | Accept a pending recipient update as current recipient |
| `cancel_recipient_update` | `stream.sender.require_auth()` | Cancel a pending recipient update as stream sender |
| `batch_withdraw` | `recipient.require_auth()` | Withdraw accrued tokens from many streams as recipient |
| `batch_withdraw_to` | `recipient.require_auth()` | Withdraw accrued tokens from many streams to destinations |
| `delegated_withdraw` | `relayer.require_auth()` | Relayer-executed withdrawal using recipient signature |
| `get_delegated_nonce` | Public / None | Read the delegated withdrawal nonce for a recipient |
| `calculate_accrued` | Public / None | Compute accrued amount for a stream |
| `get_withdrawable` | Public / None | Compute current withdrawable balance for a stream |
| `get_claimable_at` | Public / None | Query claimable amount at a target timestamp |
| `get_config` | Public / None | Read stored contract config |
| `get_global_emergency_paused` | Public / None | Read emergency pause state |
| `set_admin` | `old_admin.require_auth()` | Change contract admin (old admin auth required) |
| `set_max_rate_per_second` | `admin.require_auth()` | Set the global max rate-per-second cap |
| `get_stream_state` | Public / None | Read full stream details |
| `get_stream_health` | Public / None | Read health metrics for a stream |
| `get_stream_memo` | Public / None | Read the stream memo field |
| `get_stream_metadata` | Public / None | Read stream metadata map |
| `get_stream_count` | Public / None | Read total number of streams created |
| `update_rate_per_second` | `stream.sender.require_auth()` | Increase a sender-owned stream rate |
| `decrease_rate_per_second` | `stream.sender.require_auth()` | Decrease a sender-owned stream rate safely |
| `shorten_stream_end_time` | `stream.sender.require_auth()` | Shorten stream duration and refund unstreamed deposit |
| `extend_stream_end_time` | `stream.sender.require_auth()` | Extend stream duration without changing deposit |
| `top_up_stream` | `funder.require_auth()` | Add deposit to a stream by an authorized funder |
| `close_completed_stream` | Public / None | Permissionless cleanup of a completed or cancelled stream |
| `register_stream_template` | `owner.require_auth()` | Create a reusable schedule template |
| `delete_stream_template` | `owner.require_auth()` | Remove a schedule template owned by the caller |
| `create_stream_from_template` | `sender.require_auth()` (via `create_stream_relative` / `create_stream`) | Create a stream from a registered template |
| `get_stream_template` | Public / None | Read a saved schedule template |
| `version` | Public / None | Read current contract version |
| `get_recipient_streams` | Public / None | List stream IDs for a recipient (hard-capped at `RECIPIENT_STREAMS_PAGE_LIMIT`; use `get_recipient_streams_paginated` for full enumeration) |
| `get_recipient_streams_paginated` | Public / None | Paginate recipient stream IDs |
| `get_recipient_stream_count` | Public / None | Count streams for a recipient |
| `get_streams_by_id_range` | Public / None | Read streams in an ID range for export |
| `update_rate` | `caller.require_auth()` (sender or admin) | Update stream rate as sender or admin |
| `cancel_stream_as_admin` | `admin.require_auth()` | Cancel any stream as contract admin |
| `keeper_cancel` | `keeper.require_auth()` | Keeper-cancel an eligible stream after grace period |
| `pause_stream_as_admin` | `admin.require_auth()` | Pause any stream as contract admin |
| `resume_stream_as_admin` | `admin.require_auth()` | Resume any paused stream as contract admin |
| `set_global_emergency_paused` | `admin.require_auth()` | Admin toggle emergency pause |
| `global_resume` | `admin.require_auth()` | Admin clear emergency pause |
| `set_contract_paused` | `admin.require_auth()` | Admin pause or unpause stream creation |
| `pause_protocol` | `admin.require_auth()` | Admin globally pause protocol creation |
| `resume_protocol` | `admin.require_auth()` | Admin globally resume protocol creation |
| `is_paused` | Public / None | Read protocol pause status |
| `get_pause_info` | Public / None | Read current pause metadata |
| `sweep_excess` | `admin.require_auth()` + `recipient.require_auth()` | Admin sweep excess tokens to a recipient |
| `set_auto_claim` | `stream.recipient.require_auth()` | Recipient set auto-claim destination |
| `revoke_auto_claim` | `stream.recipient.require_auth()` | Recipient revoke auto-claim destination |
| `trigger_auto_claim` | Public / None | Permissionless execute auto-claim withdrawal |
| `get_auto_claim_status` | Public / None | Read auto-claim status for a stream |
| `get_auto_claim_destination` | Public / None | Read auto-claim destination if set |
| `clone_stream` | `source.sender.require_auth()` | Clone a source stream into a new stream |
| `reserve_stream_ids` | `caller.require_auth()` | Reserve contiguous stream IDs for later use |
| `get_id_reservation` | Public / None | View active stream ID reservation for caller |

> `cancel_stream` and `cancel_stream_as_admin` are valid only when status is `Active` or `Paused`. Streams in `Completed` or `Cancelled` state return `ContractError::InvalidState`. After cancellation, accrual is frozen at `cancelled_at`; the recipient may still withdraw the frozen accrued amount.

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
- **Property-based balance-conservation harness**: `contracts/stream/tests/balance_conservation.rs` — randomized sequences of `top_up`, `decrease_rate`, `shorten`, `extend`, `pause/resume`, `cancel`, and `withdraw` on both `Linear` and `CliffOnly` streams. Asserts global token conservation, accrual monotonicity, the rate-decrease checkpoint invariant, and `CliffOnly` unsupported-operation guards. Regression seeds live in `contracts/stream/proptest-regressions/`.

Run the new harness with a bounded case count for CI:

```bash
cargo test -p fluxora_stream --features testutils --test balance_conservation
```

For deeper local coverage before an audit or release:

```bash
PROPTEST_CASES=10000 cargo test -p fluxora_stream --features testutils --test balance_conservation
```

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
2. Rebuild locally and verify against the committed reference:
   ```bash
   bash script/verify-wasm-checksum.sh
   ```
3. Or verify existing artifacts without rebuilding:
   ```bash
   bash script/verify-wasm-checksum.sh --no-build
   ```

To update checksums after a source change:

```bash
bash script/update-wasm-checksums.sh
git add wasm/checksums.sha256
git commit -m "chore: update wasm checksums"
```

See [docs/security.md](docs/security.md#reproducible-wasm-builds) for the full reproducibility contract, auditor verification steps, and residual risks.

## Related repos

- **fluxora-backend** — API and Horizon sync
- **fluxora-frontend** — Dashboard and recipient UI

Each is a separate Git repository.
---

## 🏭 Factory Contract
The Factory contract anchors the workspace deployment ecosystem by supervising global templates, managing operational scopes, and generating token-bound stream addresses.

* **Initialization (`init`):** Seals factory state parameters, configuring fundamental baseline settings and mapping the primary admin profile.
* **Policy Setters:** Secure administration entry-points governing structural template overrides, fee parameters, and network ownership allocations.
* **Stream Creation (`create_stream`):** Instantiates an isolated stream proxy sequence matched precisely against the active, verified template hash register.

> For deployment parameter schemas, factory variables, and initialization matrices, see [docs/factory.md](docs/factory.md).

---

## ⚖️ Governance Contract
The Governance module coordinates community actions, parameter threshold overrides, and critical upgrade vectors via verifiable checkpoint logic.

* **Proposal Lifecycles (`propose` / `approve` / `execute`):** Standard multi-signature/voting pipeline driving states from conception through validation rounds directly into on-chain enforcement.
* **Signer Management:** Updates administrative sign-off lists, multisig weights, and required confirmation consensus thresholds.

> For technical consensus models, voter profiles, and cryptographic execution matrices, see [docs/governance.md](docs/governance.md).

---

## 🛠️ Compilation and Local Development

All three workspace contract modules are structured to build simultaneously under standard WebAssembly environments. To compile `stream`, `factory`, and `governance` uniformly in a single action, run:

```bash
cargo build --target wasm32-unknown-unknown --workspace
## Contributing

Please read [CONTRIBUTING.md](CONTRIBUTING.md) for details on the development workflow, branch naming, and testing requirements (including the 95% test coverage standard).

See [CHANGELOG.md](./CHANGELOG.md) for a full history of changes between contract versions.
