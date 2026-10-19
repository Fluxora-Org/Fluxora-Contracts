#!/usr/bin/env bash
# Verify checksums and reproducibility for deployable WASM artifacts.
#
# Usage:
#   bash script/verify-wasm-checksum.sh [--no-build]
#   bash script/verify-wasm-checksum.sh --reproducible
#   bash script/verify-wasm-checksum.sh --compare-only BUILD_1 BUILD_2

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WASM_TARGET="${FLUXORA_WASM_TARGET:-wasm32v1-none}"
WASM_DIR="$REPO_ROOT/target/$WASM_TARGET/release"
REPRO_ROOT="${FLUXORA_REPRO_BUILD_ROOT:-$REPO_ROOT/target/wasm-reproducibility}"
REPORT_DIR="${FLUXORA_REPRO_REPORT_DIR:-$WASM_DIR}"
REQUIRE_OPTIMIZED="${FLUXORA_REQUIRE_OPTIMIZED:-false}"

json_escape() {
    printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'
}

toolchain_channel() {
    sed -nE 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' \
        "$REPO_ROOT/rust-toolchain.toml" | head -n 1
}

soroban_sdk_version() {
    awk '
        /^name = "soroban-sdk"$/ { found = 1; next }
        found && /^version = / { gsub(/version = "|"/, ""); print; exit }
        found && /^\[\[package\]\]/ { exit }
    ' "$REPO_ROOT/Cargo.lock"
}

compare_file() {
    local name="$1" build_one="$2" build_two="$3"
    local first="$build_one/$WASM_TARGET/release/$name"
    local second="$build_two/$WASM_TARGET/release/$name"

    if [[ ! -f "$first" || ! -f "$second" ]]; then
        if [[ -f "$first" || -f "$second" ]]; then
            echo "FAIL: $name exists in only one independent build." >&2
            return 1
        fi
        return 2
    fi

    if ! cmp -s "$first" "$second"; then
        echo "FAIL: $name differs between independent builds." >&2
        echo "  build 1: $(sha256sum "$first" | awk '{print $1}')" >&2
        echo "  build 2: $(sha256sum "$second" | awk '{print $1}')" >&2
        return 1
    fi

    printf '%s\t%s\t%s\n' "$name" \
        "$(sha256sum "$first" | awk '{print $1}')" \
        "$(sha256sum "$second" | awk '{print $1}')"
}

write_report() {
    local result_lines="$1" channel rustc cargo sdk host started
    channel="$(json_escape "$(toolchain_channel)")"
    rustc="$(json_escape "$(rustc --version)")"
    cargo="$(json_escape "$(cargo --version)")"
    sdk="$(json_escape "$(soroban_sdk_version)")"
    host="$(json_escape "$(rustc -vV | sed -n 's/^host: //p')")"
    started="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

    mkdir -p "$REPORT_DIR"
    {
        printf '{\n'
        printf '  "schema": "https://github.com/Fluxora-Org/Fluxora-Contracts/reproducibility/v1",\n'
        printf '  "target": "%s",\n' "$(json_escape "$WASM_TARGET")"
        printf '  "toolchain_channel": "%s",\n' "$channel"
        printf '  "rustc": "%s",\n' "$rustc"
        printf '  "cargo": "%s",\n' "$cargo"
        printf '  "soroban_sdk": "%s",\n' "$sdk"
        printf '  "host": "%s",\n' "$host"
        printf '  "started_on": "%s",\n' "$started"
        printf '  "artifacts": [\n'
        local first=true name sha_one sha_two
        while IFS=$'\t' read -r name sha_one sha_two; do
            [[ -z "$name" ]] && continue
            if [[ "$first" == false ]]; then printf ',\n'; fi
            first=false
            printf '    {"name":"%s","build_1_sha256":"%s","build_2_sha256":"%s","identical":true}' \
                "$(json_escape "$name")" "$sha_one" "$sha_two"
        done <<< "$result_lines"
        printf '\n  ]\n}\n'
    } > "$REPORT_DIR/reproducibility.json"
}

compare_builds() {
    local build_one="$1" build_two="$2" results="" line rc
    [[ -d "$build_one" && -d "$build_two" ]] || {
        echo "ERROR: both independent build directories are required." >&2
        return 1
    }

    if line="$(compare_file fluxora_stream.wasm "$build_one" "$build_two")"; then
        results+="$line\n"
    else
        rc=$?
        return "$rc"
    fi

    if line="$(compare_file fluxora_stream.optimized.wasm "$build_one" "$build_two")"; then
        results+="$line\n"
    else
        rc=$?
        if [[ "$rc" -eq 1 || "$REQUIRE_OPTIMIZED" == true ]]; then
            [[ "$rc" -eq 1 ]] || echo "FAIL: optimized deployable artifact is missing from an independent build." >&2
            return 1
        fi
        echo "INFO: optimized artifact not present; raw deployable artifact was verified."
    fi

    write_report "$(printf '%b' "$results")"
    echo "OK: independent deployable artifact builds are byte-identical."
    echo "OK: reproducibility report written to $REPORT_DIR/reproducibility.json"
}

build_reproducibly() {
    rm -rf "$REPRO_ROOT/build-1" "$REPRO_ROOT/build-2"
    mkdir -p "$REPRO_ROOT"
    local build_one="$REPRO_ROOT/build-1" build_two="$REPRO_ROOT/build-2"

    echo "Building independent release output 1..."
    (cd "$REPO_ROOT" && cargo build --locked --release -p fluxora-stream --target "$WASM_TARGET" --target-dir "$build_one")
    echo "Building independent release output 2..."
    (cd "$REPO_ROOT" && cargo build --locked --release -p fluxora-stream --target "$WASM_TARGET" --target-dir "$build_two")

    if command -v stellar >/dev/null 2>&1; then
        echo "Optimizing both independent outputs..."
        stellar contract optimize --wasm "$build_one/$WASM_TARGET/release/fluxora_stream.wasm"
        stellar contract optimize --wasm "$build_two/$WASM_TARGET/release/fluxora_stream.wasm"
    elif [[ "$REQUIRE_OPTIMIZED" == true ]]; then
        echo "ERROR: stellar CLI is required to verify the optimized deployable artifact." >&2
        return 1
    else
        echo "INFO: stellar CLI unavailable; optimized artifact comparison skipped."
    fi

    compare_builds "$build_one" "$build_two"
}

verify_checksums() {
    local checksum_file="$WASM_DIR/fluxora_stream.wasm.sha256"
    echo "Verifying WASM SHA256 checksums..."
    if [[ ! -f "$checksum_file" ]]; then
        echo "ERROR: WASM checksum file not found at $checksum_file" >&2
        echo "Run the release checksum step first." >&2
        return 1
    fi
    (cd "$REPO_ROOT" && sha256sum -c "$checksum_file")
    echo "OK: fluxora_stream.wasm checksum verified."

    local optimized_checksum="$WASM_DIR/fluxora_stream.optimized.wasm.sha256"
    if [[ -f "$optimized_checksum" ]]; then
        (cd "$REPO_ROOT" && sha256sum -c "$optimized_checksum")
        echo "OK: fluxora_stream.optimized.wasm checksum verified."
    else
        echo "INFO: No optimized WASM checksum file found, skipping."
    fi
    echo "OK: all available WASM checksums verified."
}

case "${1:-}" in
    "")
        (cd "$REPO_ROOT" && cargo build --locked --release -p fluxora-stream --target "$WASM_TARGET")
        verify_checksums
        ;;
    --no-build)
        verify_checksums
        ;;
    --reproducible)
        build_reproducibly
        ;;
    --compare-only)
        [[ $# -eq 3 ]] || { echo "usage: $0 --compare-only BUILD_1 BUILD_2" >&2; exit 2; }
        compare_builds "$2" "$3"
        ;;
    *)
        echo "usage: $0 [--no-build|--reproducible|--compare-only BUILD_1 BUILD_2]" >&2
        exit 2
        ;;
esac
