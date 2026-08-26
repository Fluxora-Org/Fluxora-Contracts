#!/usr/bin/env sh
set -eu

budget_file="${1:-contracts/stream/wasm-size-budget.env}"

if [ ! -f "$budget_file" ]; then
  echo "missing wasm size budget file: $budget_file" >&2
  exit 1
fi

# shellcheck disable=SC1090
. "$budget_file"

: "${PACKAGE:?PACKAGE is required}"
: "${TARGET:?TARGET is required}"
: "${ARTIFACT:?ARTIFACT is required}"
: "${BUILD_COMMAND:?BUILD_COMMAND is required}"
: "${BASELINE_BYTES:?BASELINE_BYTES is required}"
: "${MAX_BYTES:?MAX_BYTES is required}"

case "$BASELINE_BYTES" in
  ''|*[!0-9]*)
    echo "BASELINE_BYTES must be an unsigned integer, got: $BASELINE_BYTES" >&2
    exit 1
    ;;
esac

case "$MAX_BYTES" in
  ''|*[!0-9]*)
    echo "MAX_BYTES must be an unsigned integer, got: $MAX_BYTES" >&2
    exit 1
    ;;
esac

echo "fluxora-stream wasm size budget"
echo "package: $PACKAGE"
echo "target: $TARGET"
echo "artifact: $ARTIFACT"
echo "baseline_bytes: $BASELINE_BYTES"
echo "max_bytes: $MAX_BYTES"
echo "build_command: $BUILD_COMMAND"

sh -c "$BUILD_COMMAND"

if [ ! -f "$ARTIFACT" ]; then
  echo "expected wasm artifact was not produced: $ARTIFACT" >&2
  exit 1
fi

size=$(wc -c < "$ARTIFACT" | tr -d ' ')
delta=$((size - BASELINE_BYTES))

echo "fluxora_stream.wasm: ${size} bytes"
echo "baseline_delta_bytes: ${delta}"

if [ "$size" -gt "$MAX_BYTES" ]; then
  echo "WASM size ${size} exceeds budget ${MAX_BYTES} bytes" >&2
  echo "Update $budget_file only when growth is intentional and reviewed." >&2
  exit 1
fi

