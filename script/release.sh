#!/usr/bin/env bash
#
# Release command — produces ONLY the product contract artifact.
#
# The repo has two Soroban contracts under contracts/:
#
#   contracts/stream          -> fluxora_stream.wasm           (the product)
#   contracts/archival-probe  -> fluxora_archival_probe.wasm   (throwaway, NOT product)
#
# The archival probe is described in its own manifest as "NOT part of the product"
# and exists only to prove the live-network archival/restore round trip that the
# unit suite structurally cannot (see KNOWN-LIMITATIONS.md §1). It must never be
# deployed to mainnet or shipped as a release artifact.
#
# Because the probe is a workspace member, a naive
# `cargo build --workspace --release` would drop the probe's wasm into the same
# output directory as the product — exactly the accidental-deployment risk this
# command exists to remove.
#
# This release command therefore builds ONLY the product package (`fluxora-stream`)
# and then asserts that no probe artifact is present in the output. It is the single
# entry point a release/publish pipeline (or a human) uses to obtain deployable
# artifacts, and the only thing it can produce is the product wasm.
#
# Usage:
#   script/release.sh
#
# Output:
#   <repo>/target/wasm32v1-none/release/fluxora_stream.wasm
#
# The probe stays a workspace member so its smoke test remains wired into the
# standard workspace checks (`cargo test --workspace`, `cargo fmt --all`,
# `cargo clippy --all-targets`; see .github/workflows/ci.yml). It remains runnable
# explicitly — but separately — via:
#
#   cargo build -p fluxora-archival-probe --target wasm32v1-none --release
#
# and its live-network round trip via script/archival-canary.sh.

set -euo pipefail

TARGET="wasm32v1-none"
PROFILE="release"
PRODUCT_PKG="fluxora-stream"
PRODUCT_WASM="fluxora_stream.wasm"
PROBE_PKG="fluxora-archival-probe"
PROBE_WASM="fluxora_archival_probe.wasm"

cd "$(dirname "$0")/.."

say() { printf '\n\033[1m── %s\033[0m\n' "$*"; }

say "1. build the product artifact only"
# Build exactly the product package. `--workspace` is deliberately NOT used: it
# would also compile the archival probe and leave its wasm among the outputs.
cargo build -p "$PRODUCT_PKG" --target "$TARGET" --profile "$PROFILE"

OUT="target/$TARGET/$PROFILE"
PRODUCT="$OUT/$PRODUCT_WASM"
PROBE="$OUT/$PROBE_WASM"

say "2. verify the probe is not present among release artifacts"
if [[ ! -f "$PRODUCT" ]]; then
  echo "   ✗ product artifact missing: $PRODUCT" >&2
  exit 1
fi
if [[ -f "$PROBE" ]]; then
  echo "   ✗ probe artifact would be deployed: $PROBE" >&2
  echo "     Remove it from the output before releasing, or build the probe with" >&2
  echo "     its explicit command instead of the workspace build." >&2
  exit 1
fi

say "3. done"
printf '   \033[32m✓\033[0m %s\n' "$PRODUCT"
printf '   \033[32m✓\033[0m release artifacts contain only the product contract\n'
