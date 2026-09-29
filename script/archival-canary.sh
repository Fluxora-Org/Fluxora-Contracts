#!/usr/bin/env bash
#
# The archival canary — the harness that produced the result recorded in
# docs/KNOWN-LIMITATIONS.md §1 on 2026-09-28.
#
# It answers the one question the unit suite structurally cannot: what does a
# live network do when an invocation touches an archived persistent entry?
#
# The answer, measured on testnet: protocol 23 and later restore the archived
# entries as part of the invoking transaction. The read succeeds, the value is
# intact, and no `RestoreFootprint` resubmission happens. This script asserts
# that, and fails loudly if a future protocol changes it back.
#
# Usage:
#   script/archival-canary.sh              status only — submits nothing
#   script/archival-canary.sh --round-trip submit `read` and assert the result
#
# See contracts/archival-probe/src/lib.rs for why this uses a throwaway probe
# contract rather than a Fluxora stream, and docs/archival-canary.md for the
# recorded run.
#
set -euo pipefail

NETWORK="${NETWORK:-testnet}"
RPC_URL="${RPC_URL:-https://soroban-testnet.stellar.org}"
SOURCE="${SOURCE:-fluxora-deployer}"
MIN_PERSISTENT_TTL="${MIN_PERSISTENT_TTL:-120960}"

# Deployed 2026-08-12. Canary planted in the same session and consumed by the
# recorded round trip on 2026-09-28; do not replant it (docs/archival-canary.md).
PROBE="${PROBE:-CB4XJYNXQ62TCXI3GKCVBWADTSTFWYL3ZLYS3MKYPWRANOSADRZG4A7N}"
# ScVal for the unit enum variant `Key::Canary` — Vec[Symbol("Canary")].
KEY_XDR='AAAAEAAAAAEAAAABAAAADwAAAAZDYW5hcnkAAA=='
# Recorded at plant time; the entry received exactly min_persistent_ttl - 1.
PLANTED_AT_LEDGER=4097334
LIVE_UNTIL_LEDGER=4218293
# The round trip that closed §1.
RECORDED_TX=32e08f32d30db0f1f1a45786dbe7f8d87ca4f83dbd3e3ced0a0d5b54d807651c

ROUND_TRIP=true
case "${1:-}" in
  --round-trip|--restore) ;;
  "") ROUND_TRIP=false ;;
  *) echo "unknown argument: $1" >&2; exit 2 ;;
esac

say() { printf '\n\033[1m── %s\033[0m\n' "$*"; }

# Read the canary straight from the RPC, without the CLI and without submitting
# anything. Prints "<liveUntil> <lastModified> <latestLedger> <value|->".
#
# `liveUntilLedgerSeq == 0` means the entry's TTL entry is gone: the data entry
# is archived. The value is still served even then — that is the point.
read_canary() {
  PROBE="$PROBE" KEY_XDR="$KEY_XDR" RPC_URL="$RPC_URL" python3 - <<'PY'
import base64, json, os, urllib.request

probe = os.environ["PROBE"]
key_xdr = base64.b64decode(os.environ["KEY_XDR"])
rpc = os.environ["RPC_URL"]

# strkey "C..." -> 0x10 version byte + 32-byte contract hash + 2-byte crc16.
raw = base64.b32decode(probe + "=" * (-len(probe) % 8))
assert raw[0] == 0x10 and len(raw) == 35, "not a contract strkey"
contract = raw[1:33]

# LedgerKey::ContractData { contract, key, ContractDataDurability::PERSISTENT }
ledger_key = (
    bytes.fromhex("00000006")   # LedgerEntryType::CONTRACT_DATA
    + bytes.fromhex("00000001")  # SCAddressType::SC_ADDRESS_TYPE_CONTRACT
    + contract
    + key_xdr
    + bytes.fromhex("00000001")  # ContractDataDurability::PERSISTENT
)


def call(method, params):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
    req = urllib.request.Request(
        rpc,
        data=body.encode(),
        headers={
            "Content-Type": "application/json",
            # Public RPC endpoints reject the default urllib agent.
            "User-Agent": "fluxora-archival-canary/1.0",
        },
    )
    with urllib.request.urlopen(req, timeout=20) as resp:
        payload = json.load(resp)
    if "error" in payload:
        raise SystemExit(f"rpc error: {payload['error']}")
    return payload["result"]


latest = call("getLatestLedger", {})["sequence"]
entries = call("getLedgerEntries", {"keys": [base64.b64encode(ledger_key).decode()]})["entries"]

if not entries:
    print(0, 0, latest, "-")
else:
    entry = entries[0]
    value = "-"
    if b"canary" in base64.b64decode(entry["xdr"]):
        value = "canary"
    print(entry["liveUntilLedgerSeq"], entry["lastModifiedLedgerSeq"], latest, value)
PY
}

SNAPSHOT=$(read_canary) || {
  echo "failed to read the canary from $RPC_URL" >&2
  exit 1
}
[[ -n "$SNAPSHOT" ]] || { echo "empty RPC response" >&2; exit 1; }
read -r LIVE_UNTIL LAST_MODIFIED NOW VALUE <<<"$SNAPSHOT"
REMAINING=$((LIVE_UNTIL - NOW))

cat <<BANNER
╭──────────────────────────────────────────────────────────────────────╮
│ Fluxora — archival canary                                            │
╰──────────────────────────────────────────────────────────────────────╯
 probe        $PROBE
 planted at   ledger $PLANTED_AT_LEDGER
 lives until  ledger $LIVE_UNTIL_LEDGER
 current      ledger $NOW
BANNER

if [[ "$VALUE" == "-" ]]; then
  printf ' status       ABSENT — the RPC did not return the canary entry at all\n\n'
  echo "Nothing to assert. Check PROBE and RPC_URL before drawing a conclusion."
  exit 1
fi

if (( REMAINING > 0 )); then
  printf ' status       ALIVE — live-until %s, %d ledgers left (~%.1f days)\n\n' \
    "$LIVE_UNTIL" "$REMAINING" "$(python3 -c "print($REMAINING*5/86400)")"
  echo "The entry has not been evicted. An entry archives at its live-until"
  echo "ledger, and eviction is a background scan, so archival can lag by"
  echo "hours or days. Re-run later."
else
  printf ' status       ARCHIVED — live-until %s, %d ledgers past it\n' \
    "$LIVE_UNTIL" "$((-REMAINING))"
  printf '              value still served: %s\n\n' "$VALUE"
fi

if ! $ROUND_TRIP; then
  echo "Status only. Re-run with --round-trip to submit \`read\` and assert"
  echo "what the network does to an archived entry."
  exit 0
fi

if (( REMAINING > 0 )); then
  echo "--round-trip needs an archived entry; this one is still live." >&2
  exit 1
fi

# ---------------------------------------------------------------------------
say "1. an invocation on the archived entry succeeds — it does not fail"
# ---------------------------------------------------------------------------
OUT=$(stellar contract invoke --id "$PROBE" --source "$SOURCE" \
        --network "$NETWORK" --send=yes -- read 2>&1)
echo "$OUT" | sed 's/^/   /'
echo "$OUT" | grep -q "$VALUE" || {
  echo "   ✗ invocation did not return '$VALUE'" >&2
  exit 1
}
echo "   ✓ returned \"$VALUE\": the invocation restored the entry it read"

TX=$(echo "$OUT" | grep -oE 'tx/[0-9a-f]{64}' | head -1 | cut -d/ -f2 || true)

# ---------------------------------------------------------------------------
say "2. the entry is live again, at the network minimum"
# ---------------------------------------------------------------------------
AFTER=$(read_canary) || {
  echo "failed to re-read the canary after the invocation" >&2
  exit 1
}
read -r LIVE_UNTIL_AFTER LAST_MODIFIED_AFTER NOW_AFTER VALUE_AFTER <<<"$AFTER"
if (( LIVE_UNTIL_AFTER <= NOW_AFTER )); then
  echo "   ✗ entry is still archived after the invocation" >&2
  exit 1
fi
printf '   ✓ live again until ledger %s (%d ledgers), value %s\n' \
  "$LIVE_UNTIL_AFTER" "$((LIVE_UNTIL_AFTER - NOW_AFTER))" "$VALUE_AFTER"
printf '     last modified in ledger %s, i.e. the restoring transaction\n' \
  "$LAST_MODIFIED_AFTER"

# ---------------------------------------------------------------------------
say "3. the transaction marked the entries it restored"
# ---------------------------------------------------------------------------
if [[ -z "$TX" ]]; then
  echo "   ! could not read the transaction hash out of the CLI output" >&2
  exit 1
fi
echo "   tx $TX"

ENVELOPE=$(TX="$TX" RPC_URL="$RPC_URL" python3 - <<'PY'
import json, os, urllib.request

tx = os.environ["TX"]
body = json.dumps(
    {"jsonrpc": "2.0", "id": 1, "method": "getTransaction", "params": {"hash": tx}}
)
req = urllib.request.Request(
    os.environ["RPC_URL"],
    data=body.encode(),
    headers={
        "Content-Type": "application/json",
        "User-Agent": "fluxora-archival-canary/1.0",
    },
)
with urllib.request.urlopen(req, timeout=20) as resp:
    payload = json.load(resp)
if "error" in payload:
    raise SystemExit(f"rpc error: {payload['error']}")
result = payload["result"]
print(result["status"], result["envelopeXdr"])
PY
)

STATUS="${ENVELOPE%% *}"
ENV_XDR="${ENVELOPE#* }"
[[ "$STATUS" == "SUCCESS" ]] || { echo "   ✗ transaction status $STATUS" >&2; exit 1; }

DECODED=$(stellar xdr decode --type TransactionEnvelope --input single-base64 \
  "$ENV_XDR" --output json)
ARCHIVED=$(printf '%s' "$DECODED" | python3 -c '
import json, sys
def find(node, key):
    if isinstance(node, dict):
        for k, v in node.items():
            if k == key:
                return v
            hit = find(v, key)
            if hit is not None:
                return hit
    elif isinstance(node, list):
        for v in node:
            hit = find(v, key)
            if hit is not None:
                return hit
    return None
entries = find(json.load(sys.stdin), "archived_soroban_entries")
print(" ".join(str(i) for i in entries) if entries else "")
')

if [[ -z "$ARCHIVED" ]]; then
  echo "   ✗ the transaction restored nothing: this is the failure mode §1 used"
  echo "     to describe. Escalate before changing any documentation." >&2
  exit 1
fi

printf '   ✓ archived_soroban_entries = [%s]\n' "$ARCHIVED"
echo "     Indices into the transaction footprint (read_only then read_write):"
printf '%s' "$DECODED" | python3 -c '
import json, sys
def find(node, key):
    if isinstance(node, dict):
        for k, v in node.items():
            if k == key:
                return v
            hit = find(v, key)
            if hit is not None:
                return hit
    elif isinstance(node, list):
        for v in node:
            hit = find(v, key)
            if hit is not None:
                return hit
    return None
env = json.load(sys.stdin)
fp = find(env, "footprint") or {}
order = list(fp.get("read_only") or []) + list(fp.get("read_write") or [])
for idx, value in enumerate(order):
    name = next(iter(value)) if isinstance(value, dict) else value
    detail = value.get("contract_data") or value.get("contract_code") or {}
    key = detail.get("key") if isinstance(detail, dict) else None
    if isinstance(key, dict):
        key = next(iter(key.values()))
    if key is None:
        key = ""
    print(f"       [{idx}] {name} {key}".rstrip())
'

cat <<DONE

╭──────────────────────────────────────────────────────────────────────╮
│ Round trip complete — docs/KNOWN-LIMITATIONS.md §1 records Outcome B. │
╰──────────────────────────────────────────────────────────────────────╯
The archived entries were restored by the invoking transaction itself. There
was no failed read and no RestoreFootprint resubmission, which is why §1 says
archival is not a failure mode for persistent entries rather than claiming a
recovery path works.

Recorded reference run: tx $RECORDED_TX on testnet, 2026-09-28.
DONE
