# Quickstart Guide

Get started with local Fluxora development on Soroban.

## Prerequisites

- [Rust](https://rustup.rs/) (stable toolchain)
- [Soroban CLI](https://soroban.stellar.org/docs/getting-started/setup)
- [Stellar CLI](https://developers.stellar.org/docs/tools/developer-tools)

### Install Soroban CLI

```bash
cargo install --locked soroban-cli
```

### Add the WASM target

```bash
rustup target add wasm32-unknown-unknown
```

## Clone and Build

```bash
git clone https://github.com/Fluxora-Org/Fluxora-Contracts.git
cd Fluxora-Contracts
soroban contract build
```

## Run Tests

```bash
cargo test
```

## Deploy to Testnet

1. Configure testnet identity:
```bash
soroban config identity generate --global my-account
soroban config network add --global testnet \
  --rpc-url https://soroban-testnet.stellar.org:443 \
  --network-passphrase "Test SDF Network ; September 2015"
```

2. Fund with Friendbot:
```bash
curl "https://friendbot.stellar.org?addr=$(soroban config identity address my-account)"
```

3. Deploy:
```bash
soroban contract deploy \
  --wasm target/wasm32-unknown-unknown/release/fluxora.wasm \
  --source my-account \
  --network testnet
```

## Troubleshooting

- **Build fails**: Ensure `wasm32-unknown-unknown` target is installed
- **Deploy fails**: Check your testnet account has funds via Friendbot
- **CLI version**: Run `soroban version` to verify CLI is installed

## Next Steps

- Read the [Soroban docs](https://soroban.stellar.org/docs)
- Explore the contract source in `src/`
- Check open issues for contribution opportunities
