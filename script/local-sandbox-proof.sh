#!/usr/bin/env bash
# Build, deploy, initialize, query, and tear down Fluxora in an isolated
# standalone Soroban network. No testnet identities, credentials, or funds are
# used; all accounts are generated in a temporary stellar CLI home.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"
started=$SECONDS

IMAGE="${SOROBAN_SANDBOX_IMAGE:-stellar/quickstart:testing}"
CONTAINER="fluxora-soroban-sandbox-$$"
CLI_HOME=$(mktemp -d)
WASM="target/wasm32v1-none/release/fluxora_stream.wasm"
RPC_URL="http://127.0.0.1:8000/soroban/rpc"
FRIENDBOT_URL="http://127.0.0.1:8000/friendbot"
NETWORK_PASSPHRASE="Standalone Network ; February 2017"

cleanup() {
  docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
  rm -rf "$CLI_HOME"
}
trap cleanup EXIT

command -v docker >/dev/null || { echo "docker is required" >&2; exit 2; }
command -v stellar >/dev/null || { echo "stellar CLI is required" >&2; exit 2; }

echo "== build =="
cargo build -p fluxora-stream --target wasm32v1-none --release
checksum=$(sha256sum "$WASM" | awk '{print $1}')
size=$(stat -c%s "$WASM")
echo "wasm: $WASM"
echo "sha256: $checksum"
echo "bytes: $size"
echo "sandbox image: $IMAGE"

echo "== start standalone network =="
docker run --detach --rm --name "$CONTAINER" \
  --publish 8000:8000 \
  "$IMAGE" --standalone --enable-soroban-rpc >/dev/null

for attempt in $(seq 1 60); do
  if curl --silent --fail --max-time 2 -X POST "$RPC_URL" \
      -H 'Content-Type: application/json' \
      -d '{"jsonrpc":"2.0","id":1,"method":"getLatestLedger"}' \
      | grep -q '"sequence"'; then
    break
  fi
  if [[ "$attempt" == 60 ]]; then
    echo "standalone Soroban RPC did not become ready" >&2
    exit 1
  fi
  sleep 1
done

export STELLAR_CONFIG_DIR="$CLI_HOME"
stellar network add --global local --rpc-url "$RPC_URL" \
  --network-passphrase "$NETWORK_PASSPHRASE" --friendbot-url "$FRIENDBOT_URL" >/dev/null
stellar keys generate sandbox-sender --network local >/dev/null
stellar keys generate sandbox-recipient --network local >/dev/null
stellar keys fund sandbox-sender --network local >/dev/null
stellar keys fund sandbox-recipient --network local >/dev/null

echo "== deploy =="
contract=$(stellar contract deploy --wasm "$WASM" --source sandbox-sender \
  --network local)
echo "contract: $contract"

sender=$(stellar keys address sandbox-sender)
recipient=$(stellar keys address sandbox-recipient)
token=$(stellar contract id asset --asset native --network local)
now=$(date +%s)
end=$((now + 600))
deposit=6000000000

echo "== initialize stream state =="
stream_id=$(stellar contract invoke --id "$contract" --source sandbox-sender \
  --network local --send=yes -- create_stream \
  --sender "$sender" --recipient "$recipient" --token "$token" \
  --deposit "$deposit" --start_time "$now" --end_time "$end" \
  --cliff_time "$now" --cancellable true --pausable true --transferable true \
  | tail -1 | tr -d '"')
[[ "$stream_id" =~ ^[0-9]+$ ]] || { echo "invalid stream id: $stream_id" >&2; exit 1; }
echo "stream_id: $stream_id"

echo "== read-only entrypoints =="
view() {
  stellar contract invoke --id "$contract" --source sandbox-recipient \
    --network local --send=no -- "$@" | tail -1 | tr -d '"'
}

stream=$(view get_stream --stream_id "$stream_id")
echo "get_stream: $stream"
[[ "$stream" == *"deposited"* ]] || { echo "stream was not initialized" >&2; exit 1; }

withdrawable=$(view withdrawable_of --stream_id "$stream_id")
vested=$(view vested_of --stream_id "$stream_id")
refundable=$(view refundable_of --stream_id "$stream_id")
count=$(view stream_count)
exists=$(view stream_exists --stream_id "$stream_id")
missing=$(view stream_exists --stream_id 999999)
printf 'withdrawable_of: %s\nvested_of: %s\nrefundable_of: %s\n' \
  "$withdrawable" "$vested" "$refundable"
printf 'stream_count: %s\nstream_exists(%s): %s\nstream_exists(999999): %s\n' \
  "$count" "$stream_id" "$exists" "$missing"
[[ "$withdrawable" == "0" && "$vested" == "0" ]] || exit 1
[[ "$refundable" == "$deposit" && "$count" == "1" && "$exists" == "true" ]] || exit 1
[[ "$missing" == "false" ]] || exit 1

echo "== proof complete; tearing down sandbox =="
echo "elapsed_seconds: $((SECONDS - started))"