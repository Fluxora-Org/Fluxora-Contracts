#!/usr/bin/env bash
#
# Wasm provenance — the release-integrity gate from issue #1546.
#
# Every deployable wasm the workspace produces gets a machine-readable manifest
# (provenance.json) tying its bytes to the git revision, Rust toolchain,
# soroban-sdk version, target triple and release profile, plus a SHASUMS file
# in `sha256sum` format. `verify` re-hashes the artifacts and re-checks the
# environment, and fails the release on any mismatch.
#
# Usage:
#   script/provenance.sh build                      # wasm build + generate + verify
#   script/provenance.sh generate [release-dir]     # write provenance.json + SHASUMS
#   script/provenance.sh verify   [release-dir]     # release gate — exit non-zero on mismatch
#   script/provenance.sh test                       # run the tool's regression suite
#
# The release dir defaults to target/<FLUXORA_WASM_TARGET>/release
# (FLUXORA_WASM_TARGET defaults to wasm32v1-none, per rust-toolchain.toml).
# All commands are idempotent: re-run after a rebuild to regenerate, or after
# fixing whatever drifted to re-verify.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="${FLUXORA_WASM_TARGET:-wasm32v1-none}"
RELEASE_DIR="${2:-$ROOT/target/$TARGET/release}"
TOOL_DIR="$ROOT/tools/provenance"
TOOL="$TOOL_DIR/target/release/fluxora-provenance"

build_tool() {
  (cd "$TOOL_DIR" && cargo build --release --quiet)
}

cmd="${1:-help}"
case "$cmd" in
  build)
    build_tool
    cargo build --workspace --target "$TARGET" --release
    "$TOOL" generate "$RELEASE_DIR" --target "$TARGET"
    "$TOOL" verify "$RELEASE_DIR" --target "$TARGET"
    echo "provenance ok — $RELEASE_DIR/provenance.json is current"
    ;;
  generate)
    build_tool
    "$TOOL" generate "$RELEASE_DIR" --target "$TARGET"
    ;;
  verify)
    build_tool
    "$TOOL" verify "$RELEASE_DIR" --target "$TARGET"
    ;;
  test)
    (cd "$TOOL_DIR" && cargo test)
    ;;
  help|-h|--help)
    sed -n '2,20p' "${BASH_SOURCE[0]}"
    ;;
  *)
    echo "usage: $0 {build|generate|verify|test} [release-dir]" >&2
    exit 2
    ;;
esac
