# Release Runbook

## ABI Generation

The deployable contract ABI is generated from the optimized WASM (the exact bytes that get deployed) using the Stellar CLI:

```bash
script/generate_abi.sh
```

The script:
1. Reads `ABI_VERSION` from `contracts/stream/src/lib.rs` (the single source of truth).
2. Runs `stellar contract info interface --wasm <optimized.wasm> --output json` to extract the interface spec.
3. Writes `contracts/stream/abi/fluxora_abi.json` with `abi_version`, `upgradeable`, `upgrade_posture`, and `functions`.

The ABI is generated **only** from the optimized WASM so it matches what is deployed. CI runs `script/generate_abi.sh` after the "Optimize WASM" step and uploads the result as the `fluxora-stream-abi` artifact.

## Mainnet Deployment Approval

A mainnet deployment cannot proceed without approval. The `deploy-mainnet` job in our CI pipeline leverages `trstringer/manual-approval@v1` to enforce this rule within the repository configuration. It requires approval from a named reviewer (e.g. `Fluxora-Org/maintainers`) before executing the deployment to Stellar mainnet. Approvals are auditable via the automatically generated issue in this repository.
