#!/usr/bin/env bash
#
# Stage 4 acceptance criterion — the live archival/restore round trip.
#
# Closes docs/KNOWN-LIMITATIONS.md §1, the one thing the unit suite structurally
# cannot prove: that reading an archived persistent entry *fails* on a real
# network, that `RestoreFootprint` recovers it, and that the data survives.
#
#   before archival:  reports how long is left and exits 0
#   after  archival:  runs the round trip and asserts each step
#
# Usage:
#   script/archival-canary.sh [--restore]
#
#     (no flag)   status only — safe to run any time
#     --restore   once archived, perform the restore and verify recovery
#
# See contracts/archival-probe/src/lib.rs for why this uses a throwaway probe
# contract rather than a Fluxora stream.

set -euo pipefail

NETWORK="${NETWORK:-testnet}"
RPC_URL="${RPC_URL:-https://soroban-testnet.stellar.org}"
SOURCE="${SOURCE:-fluxora-deployer}"

# Deployed 2026-08-12. Canary planted in the same session.
PROBE="${PROBE:-CB4XJYNXQ62TCXI3GKCVBWADTSTFWYL3ZLYS3MKYPWRANOSADRZG4A7N}"
# ScVal for the unit enum variant `Key::Canary` — Vec[Symbol("Canary")].
KEY_XDR='AAAAEAAAAAEAAAABAAAADwAAAAZDYW5hcnkAAA=='
# Recorded at plant time; the entry received exactly min_persistent_ttl - 1.
PLANTED_AT_LEDGER=4097334
LIVE_UNTIL_LEDGER=4218293

RESTORE=false
[[ "${1:-}" == "--restore" ]] && RESTORE=true

latest_ledger() {
  curl -s -m 10 -X POST "$RPC_URL" -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"getLatestLedger"}' |
    python3 -c 'import sys,json;print(json.load(sys.stdin)["result"]["sequence"])'
}

say() { printf '\n\033[1m── %s\033[0m\n' "$*"; }

NOW=$(latest_ledger)
REMAINING=$((LIVE_UNTIL_LEDGER - NOW))

cat <<BANNER
╭──────────────────────────────────────────────────────────────────────╮
│ Fluxora — archival canary                                            │
╰──────────────────────────────────────────────────────────────────────╯
 probe        $PROBE
 planted at   ledger $PLANTED_AT_LEDGER
 lives until  ledger $LIVE_UNTIL_LEDGER
 current      ledger $NOW
BANNER

if (( REMAINING > 0 )); then
  printf ' status       ALIVE — %d ledgers left (~%.1f days)\n\n' \
    "$REMAINING" "$(python3 -c "print($REMAINING*5/86400)")"
  echo "Not archived yet. The entry received exactly min_persistent_ttl (120,960"
  echo "ledgers, ~7 days) because the probe deliberately never extends it."
  echo "Re-run after ledger $LIVE_UNTIL_LEDGER, with --restore, to close"
  echo "docs/KNOWN-LIMITATIONS.md §1."
  exit 0
fi

printf ' status       PAST LIVE-UNTIL by %d ledgers — archival expected\n' "$((-REMAINING))"

# ---------------------------------------------------------------------------
say "1. the entry is no longer readable as live state"
# ---------------------------------------------------------------------------
if stellar contract read --id "$PROBE" --network "$NETWORK" \
     --durability persistent --key-xdr "$KEY_XDR" 2>/dev/null; then
  echo "   ✗ entry still readable — it has not been evicted yet."
  echo "     Eviction is a background scan, so it lags live-until. Retry later."
  exit 1
fi
echo "   ✓ read failed: the entry is archived"

# ---------------------------------------------------------------------------
say "2. invoking the contract fails rather than returning stale data"
# ---------------------------------------------------------------------------
if OUT=$(stellar contract invoke --id "$PROBE" --source "$SOURCE" \
           --network "$NETWORK" --send=yes -- read 2>&1); then
  echo "   ✗ invocation SUCCEEDED against an archived entry: $OUT"
  echo "     If this happens, the network auto-restored — which would mean the"
  echo "     unit-test caveat in docs/KNOWN-LIMITATIONS.md §1 does not apply on-network."
  exit 1
fi
echo "   ✓ invocation failed as expected"
echo "$OUT" | grep -oiE 'archiv[a-z]*|restore[a-z]*|entry.*(expired|missing)' | head -3 |
  sed 's/^/     network said: /' || true

if ! $RESTORE; then
  echo
  echo "Archived and failing as designed. Re-run with --restore to complete the"
  echo "round trip."
  exit 0
fi

# ---------------------------------------------------------------------------
say "3. restore via RestoreFootprint"
# ---------------------------------------------------------------------------
stellar contract restore --id "$PROBE" --source "$SOURCE" --network "$NETWORK" \
  --durability persistent --key-xdr "$KEY_XDR" 2>&1 | tail -3
echo "   ✓ restore submitted"

# ---------------------------------------------------------------------------
say "4. the data came back intact"
# ---------------------------------------------------------------------------
VALUE=$(stellar contract invoke --id "$PROBE" --source "$SOURCE" \
  --network "$NETWORK" --send=no -- read 2>/dev/null | tail -1 | tr -d '"')
if [[ "$VALUE" == "canary" ]]; then
  echo "   ✓ read returns \"canary\" — value survived archival and restore"
else
  echo "   ✗ read returned '$VALUE', expected 'canary'"
  exit 1
fi

NEW_TTL=$(stellar contract read --id "$PROBE" --network "$NETWORK" \
  --durability persistent --key-xdr "$KEY_XDR" 2>/dev/null | awk -F, '{print $NF}')
echo "   ✓ entry live again until ledger $NEW_TTL"

cat <<'DONE'

╭──────────────────────────────────────────────────────────────────────╮
│ Round trip complete — docs/KNOWN-LIMITATIONS.md §1 can be closed.          │
╰──────────────────────────────────────────────────────────────────────╯
Update docs/KNOWN-LIMITATIONS.md §1 with the transaction hashes above, and note
in the SDK requirements that a client must detect this failure and offer
restore rather than surfacing the raw error.
DONE

